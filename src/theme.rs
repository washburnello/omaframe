//! Theme provider: Omarchy `colors.toml` → Ratatui styles.
//!
//! Plan §7 (app chrome, not canvas): reads the active theme on start (the
//! TUI re-calls [`load`] on focus regain / `theme-set` signal) and maps
//! `background→bg`, `foreground→fg`, `accent→accent`, `selection→highlight`,
//! `muted→dim`, `color0–15→ansi slots`. Canvas cells resolve via
//! [`cell_style`] against the wireframe slots, so a theme switch re-tints
//! hues but never remaps structure.
//!
//! Resolution order for [`load`] (never panics, tolerant parse):
//!
//! 1. `~/.config/omarchy/current/theme` symlink if present (dir →
//!    `<dir>/colors.toml`, file → itself when named `colors.toml` or a
//!    theme-name pointer into `themes/<name>/colors.toml`).
//! 2. First `~/.config/omarchy/themes/*/colors.toml` found (sorted).
//! 3. Built-in picotron-dark defaults from plan §2.4.
//!
//! The on-disk `colors.toml` is parsed manually (simple `key="value"`
//! lines, no new dependencies). Unknown keys are ignored, missing keys
//! keep defaults, malformed values are skipped — never a crash on
//! missing/renamed keys (plan §10: theme churn).

use std::collections::HashMap;
use std::path::PathBuf;

use ratatui::style::{Color, Style};

/// App-chrome theme plus the 16 wireframe palette slots.
///
/// `bg/fg/accent/dim/highlight` paint chrome; `ansi[0..16]` paints canvas
/// cells (stable indices, plan §7). All colors are opaque [`Color::Rgb`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Theme {
    /// Omarchy `background` (app bg, dark preview canvas bg).
    pub bg: Color,
    /// Omarchy `foreground` (app text, light preview canvas bg).
    pub fg: Color,
    /// Omarchy `accent` (active/selected).
    pub accent: Color,
    /// Omarchy `muted` (dim/borders).
    pub dim: Color,
    /// Omarchy `selection` (highlight).
    pub highlight: Color,
    /// Omarchy `color0–15` (wireframe palette slots).
    pub ansi: [Color; 16],
}

/// Built-in picotron-dark defaults (plan §2.4).
///
/// Roles come straight from the plan: bg `#1e1e2e`, fg `#cdd6f4`,
/// accent `#89b4fa`, selection `#45475a`, muted `#585b70`.
/// The plan lists no `color0–15`, so `ansi` falls back to a sane
/// Catppuccin-Mocha-flavoured 16-slot ramp (distinct, dark-bg legible).
pub fn defaults() -> Theme {
    Theme {
        bg: Color::Rgb(0x1e, 0x1e, 0x2e),
        fg: Color::Rgb(0xcd, 0xd6, 0xf4),
        accent: Color::Rgb(0x89, 0xb4, 0xfa),
        dim: Color::Rgb(0x58, 0x5b, 0x70),
        highlight: Color::Rgb(0x45, 0x47, 0x5a),
        ansi: [
            Color::Rgb(0x45, 0x47, 0x5a), // 0  Surface1
            Color::Rgb(0xf3, 0x8b, 0xa8), // 1  Red
            Color::Rgb(0xa6, 0xe3, 0xa1), // 2  Green
            Color::Rgb(0xf9, 0xe2, 0xaf), // 3  Yellow
            Color::Rgb(0x89, 0xb4, 0xfa), // 4  Blue
            Color::Rgb(0xcb, 0xa6, 0xf7), // 5  Mauve
            Color::Rgb(0x94, 0xe2, 0xd5), // 6  Teal
            Color::Rgb(0xba, 0xc2, 0xde), // 7  Subtext1
            Color::Rgb(0x58, 0x5b, 0x70), // 8  Surface2
            Color::Rgb(0xeb, 0xa0, 0xac), // 9  Maroon
            Color::Rgb(0xfa, 0xb3, 0x87), // 10 Peach
            Color::Rgb(0xf5, 0xe0, 0xdc), // 11 Rosewater
            Color::Rgb(0x74, 0xc7, 0xec), // 12 Sapphire
            Color::Rgb(0xf5, 0xc2, 0xe7), // 13 Pink
            Color::Rgb(0x89, 0xdc, 0xeb), // 14 Sky
            Color::Rgb(0xcd, 0xd6, 0xf4), // 15 Text (= fg)
        ],
    }
}

/// Parse `#rrggbb` / `#rgb` (with or without `#`, any case) into a
/// Ratatui color. Returns `None` for anything else (tolerant: caller
/// skips the key and keeps the default).
fn parse_hex_color(s: &str) -> Option<Color> {
    let t = s.trim().trim_matches(',').trim();
    let hex = t.strip_prefix('#').unwrap_or(t);
    match hex.len() {
        3 => {
            let mut v = [0u8; 3];
            for (i, ch) in hex.chars().enumerate() {
                let d = ch.to_digit(16)? as u8;
                v[i] = d * 17; // expand single nibble: 0xA -> 0xAA
            }
            Some(Color::Rgb(v[0], v[1], v[2]))
        }
        6 => {
            // Byte-aligned ASCII hex, so byte slicing is safe; use `get`
            // anyway to stay panic-free on any input.
            let r = u8::from_str_radix(hex.get(0..2)?, 16).ok()?;
            let g = u8::from_str_radix(hex.get(2..4)?, 16).ok()?;
            let b = u8::from_str_radix(hex.get(4..6)?, 16).ok()?;
            Some(Color::Rgb(r, g, b))
        }
        _ => None,
    }
}

/// Minimal TOML-subset parse: collects simple `key = "value"` lines.
///
/// Tolerant by design: skips blanks, `#` comments, `[sections]`, lines
/// without `=`, unterminated quotes, and bare non-string values it cannot
/// use. Keys are lowercased; last occurrence wins. Never panics.
fn parse_toml_kv(text: &str) -> HashMap<String, String> {
    let mut map = HashMap::new();
    for raw in text.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') || line.starts_with('[') {
            continue;
        }
        let Some(eq) = line.find('=') else {
            continue;
        };
        let key_part = line.get(..eq).unwrap_or("").trim();
        if key_part.is_empty() {
            continue;
        }
        // Strip optional surrounding quotes from the key itself.
        let key = key_part
            .trim_matches('"')
            .trim_matches('\'')
            .trim()
            .to_ascii_lowercase();
        if key.is_empty() {
            continue;
        }
        let val_raw = line.get(eq + 1..).unwrap_or("").trim();
        if val_raw.is_empty() {
            continue;
        }
        let parsed: Option<String> = if val_raw.starts_with('"') {
            match val_raw.get(1..).and_then(|rest| rest.find('"')) {
                Some(end) => val_raw.get(1..1 + end).map(|s| s.to_string()),
                None => continue,
            }
        } else if val_raw.starts_with('\'') {
            match val_raw.get(1..).and_then(|rest| rest.find('\'')) {
                Some(end) => val_raw.get(1..1 + end).map(|s| s.to_string()),
                None => continue,
            }
        } else {
            // Bare value (`true`, `42`, unquoted hex): take the first
            // whitespace/comma-separated token, cut a trailing `#comment`
            // unless the token itself is a `#hex` color.
            let mut token = val_raw.split_whitespace().next().unwrap_or("");
            token = token.trim_end_matches(',').trim();
            if token.is_empty() {
                continue;
            }
            if token.starts_with('#') {
                Some(token.to_string())
            } else if let Some(hash) = token.find('#') {
                let left = token.get(..hash).unwrap_or("").trim_end_matches(',').trim();
                if left.is_empty() {
                    continue;
                }
                Some(left.to_string())
            } else {
                let clean = token.trim_matches('"').trim_matches('\'').trim();
                if clean.is_empty() {
                    continue;
                }
                Some(clean.to_string())
            }
        };
        if let Some(v) = parsed {
            map.insert(key, v);
        }
    }
    map
}

/// Overlay parsed keys onto a theme (missing/malformed keys keep current
/// values). Recognised: `background`, `foreground`, `accent`, `muted`,
/// `selection`, `color0`–`color15`. Everything else is ignored.
fn apply_kv(theme: &mut Theme, kv: &HashMap<String, String>) {
    let get = |k: &str| kv.get(k).and_then(|v| parse_hex_color(v));
    if let Some(c) = get("background") {
        theme.bg = c;
    }
    if let Some(c) = get("foreground") {
        theme.fg = c;
    }
    if let Some(c) = get("accent") {
        theme.accent = c;
    }
    if let Some(c) = get("muted") {
        theme.dim = c;
    }
    if let Some(c) = get("selection") {
        theme.highlight = c;
    }
    for i in 0..16 {
        let key = format!("color{i}");
        if let Some(v) = kv.get(&key) {
            if let Some(c) = parse_hex_color(v) {
                theme.ansi[i] = c;
            }
        }
    }
}

/// Build a theme from a `colors.toml` string over the built-in defaults.
///
/// Public for tests and tooling; [`load`] is the filesystem entry point.
/// Never panics.
pub fn from_toml_str(s: &str) -> Theme {
    let mut theme = defaults();
    let kv = parse_toml_kv(s);
    apply_kv(&mut theme, &kv);
    theme
}

/// Resolve the active `colors.toml` path, if any.
///
/// `~/.config/omarchy/current/theme` symlink (or dir/file) first, else
/// the first `~/.config/omarchy/themes/*/colors.toml` in sorted order.
/// Returns `None` when `$HOME` is unset or nothing is found (caller falls
/// back to [`defaults`]). Never panics.
pub fn resolved_path() -> Option<PathBuf> {
    let home = std::env::var("HOME").ok()?;
    let base = PathBuf::from(home).join(".config/omarchy");
    let cur = base.join("current/theme");

    if let Ok(meta) = std::fs::symlink_metadata(&cur) {
        if meta.file_type().is_symlink() {
            if let Ok(target) = std::fs::read_link(&cur) {
                let abs = if target.is_absolute() {
                    target
                } else {
                    cur.parent().unwrap_or(&base).join(&target)
                };
                if abs.is_dir() {
                    let cand = abs.join("colors.toml");
                    if cand.is_file() {
                        return Some(cand);
                    }
                } else if abs.is_file() {
                    return Some(abs);
                } else if let Some(name) = abs.file_name().and_then(|n| n.to_str()) {
                    // Target names a theme (e.g. `../themes/foo` that has
                    // since moved): try `themes/<name>/colors.toml`.
                    let cand = base.join("themes").join(name).join("colors.toml");
                    if cand.is_file() {
                        return Some(cand);
                    }
                }
            }
            // Fall through to metadata check below (symlink may still
            // resolve via the filesystem even if read_link handling missed).
            if let Ok(md) = std::fs::metadata(&cur) {
                if md.is_dir() {
                    let cand = cur.join("colors.toml");
                    if cand.is_file() {
                        return Some(cand);
                    }
                } else if md.is_file() {
                    if cur
                        .file_name()
                        .and_then(|n| n.to_str())
                        .is_some_and(|n| n == "colors.toml")
                    {
                        return Some(cur.clone());
                    }
                    if let Ok(content) = std::fs::read_to_string(&cur) {
                        let name = content
                            .lines()
                            .next()
                            .unwrap_or("")
                            .trim()
                            .trim_matches('"')
                            .trim_matches('\'')
                            .trim();
                        if !name.is_empty() && !name.contains('=') {
                            let cand = base.join("themes").join(name).join("colors.toml");
                            if cand.is_file() {
                                return Some(cand);
                            }
                        }
                    }
                }
            }
        } else if meta.is_dir() {
            let cand = cur.join("colors.toml");
            if cand.is_file() {
                return Some(cand);
            }
        } else if meta.is_file() {
            if cur
                .file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n == "colors.toml")
            {
                return Some(cur.clone());
            }
            if let Ok(content) = std::fs::read_to_string(&cur) {
                let name = content
                    .lines()
                    .next()
                    .unwrap_or("")
                    .trim()
                    .trim_matches('"')
                    .trim_matches('\'')
                    .trim();
                if !name.is_empty() && !name.contains('=') {
                    let cand = base.join("themes").join(name).join("colors.toml");
                    if cand.is_file() {
                        return Some(cand);
                    }
                }
            }
        }
    }

    let themes_dir = base.join("themes");
    if let Ok(entries) = std::fs::read_dir(&themes_dir) {
        let mut cands: Vec<PathBuf> = Vec::new();
        for entry in entries.flatten() {
            let p = entry.path();
            if p.is_dir() {
                let cand = p.join("colors.toml");
                if cand.is_file() {
                    cands.push(cand);
                }
            } else if p.is_file()
                && p.file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|n| n == "colors.toml")
            {
                cands.push(p);
            }
        }
        cands.sort();
        if let Some(first) = cands.into_iter().next() {
            return Some(first);
        }
    }
    None
}

/// Load the active Omarchy theme (see [`resolved_path`]).
///
/// Tolerant: any IO/parse failure yields increasingly-defaulted values,
/// ending at [`defaults`]. Never panics — safe to call on start, on
/// focus regain, and on the `theme-set` hook.
pub fn load() -> Theme {
    let mut theme = defaults();
    if let Some(path) = resolved_path() {
        if let Ok(text) = std::fs::read_to_string(&path) {
            let kv = parse_toml_kv(&text);
            apply_kv(&mut theme, &kv);
        }
    }
    theme
}

/// Ratatui style for a canvas cell.
///
/// `fg`/`bg` are ANSI slot indices into [`Theme::ansi`]. `bg == -1`
/// means transparent: the preview surface shows through (dark → theme
/// bg, light → theme fg per `preview_dark`). Out-of-range indices fall
/// back tolerantly (`fg` → theme fg, `bg` → preview bg) and never panic.
pub fn cell_style(fg: i8, bg: i8, theme: &Theme, preview_dark: bool) -> Style {
    let fg_color = if (0..16).contains(&fg) {
        theme.ansi[fg as usize]
    } else {
        theme.fg
    };
    let preview_bg = if preview_dark { theme.bg } else { theme.fg };
    let bg_color = if bg == -1 {
        preview_bg
    } else if (0..16).contains(&bg) {
        theme.ansi[bg as usize]
    } else {
        preview_bg
    };
    Style::default().fg(fg_color).bg(bg_color)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_sane() {
        let t = defaults();
        // Plan §2.4 reference values.
        assert_eq!(t.bg, Color::Rgb(0x1e, 0x1e, 0x2e));
        assert_eq!(t.fg, Color::Rgb(0xcd, 0xd6, 0xf4));
        assert_eq!(t.accent, Color::Rgb(0x89, 0xb4, 0xfa));
        assert_eq!(t.highlight, Color::Rgb(0x45, 0x47, 0x5a));
        assert_eq!(t.dim, Color::Rgb(0x58, 0x5b, 0x70));
        assert_eq!(t.ansi.len(), 16);
        // Every slot must be an opaque Rgb (no Indexed/Reset leaks).
        for c in &t.ansi {
            assert!(matches!(c, Color::Rgb(_, _, _)), "ansi slot not Rgb: {c:?}");
        }
        // `load()` without a theme dir still yields the defaults (never panics).
        let _ = load();
    }

    #[test]
    fn tolerant_parse_of_sample_colors_toml() {
        let sample = r##"
# sample omarchy colors.toml
background = "#1d2b53"
foreground = '#fff1e8'
accent= "#00a5a1"
selection = "#432932"
muted = "#5f574f"
color0 = "#000000"
color1 = "#ff004d"
color7 = "#fff1e8"
color15="#ffccaa"

[ignored-section]
foo = "bar"

this line has no equals and is skipped
badcolor = "not-a-color"
unterminated = "oops
color3 = "#008751" # trailing comment
"##;
        let t = from_toml_str(sample);
        assert_eq!(t.bg, Color::Rgb(0x1d, 0x2b, 0x53));
        assert_eq!(t.fg, Color::Rgb(0xff, 0xf1, 0xe8));
        assert_eq!(t.accent, Color::Rgb(0x00, 0xa5, 0xa1));
        assert_eq!(t.highlight, Color::Rgb(0x43, 0x29, 0x32));
        assert_eq!(t.dim, Color::Rgb(0x5f, 0x57, 0x4f));
        assert_eq!(t.ansi[0], Color::Rgb(0x00, 0x00, 0x00));
        assert_eq!(t.ansi[1], Color::Rgb(0xff, 0x00, 0x4d));
        assert_eq!(t.ansi[3], Color::Rgb(0x00, 0x87, 0x51));
        assert_eq!(t.ansi[7], Color::Rgb(0xff, 0xf1, 0xe8));
        assert_eq!(t.ansi[15], Color::Rgb(0xff, 0xcc, 0xaa));
        // Untouched slots keep defaults; garbage keys never crash.
        let d = defaults();
        assert_eq!(t.ansi[2], d.ansi[2]);
        assert_eq!(t.ansi[5], d.ansi[5]);
        // Empty/garbage input degrades to defaults, never panics.
        assert_eq!(from_toml_str(""), d);
        assert_eq!(from_toml_str("###\n[[[\n=\n"), d);
    }

    #[test]
    fn hex_forms_and_cell_style() {
        assert_eq!(parse_hex_color("#fff"), Some(Color::Rgb(0xff, 0xff, 0xff)));
        assert_eq!(parse_hex_color("#000000"), Some(Color::Rgb(0, 0, 0)));
        assert_eq!(parse_hex_color("89b4fa"), Some(Color::Rgb(0x89, 0xb4, 0xfa)));
        assert_eq!(parse_hex_color("not-a-color"), None);
        assert_eq!(parse_hex_color("#zzzzzz"), None);
        assert_eq!(parse_hex_color(""), None);

        let t = defaults();
        // Indexed slots resolve; bg -1 follows the preview flag.
        let s = cell_style(4, -1, &t, true);
        assert_eq!(s.fg, Some(t.ansi[4]));
        assert_eq!(s.bg, Some(t.bg));
        let s2 = cell_style(4, -1, &t, false);
        assert_eq!(s2.bg, Some(t.fg));
        let s3 = cell_style(1, 2, &t, true);
        assert_eq!(s3.fg, Some(t.ansi[1]));
        assert_eq!(s3.bg, Some(t.ansi[2]));
        // Out-of-range indices degrade gracefully, never panic.
        let s4 = cell_style(99, 99, &t, true);
        assert_eq!(s4.fg, Some(t.fg));
        assert_eq!(s4.bg, Some(t.bg));
    }
}
