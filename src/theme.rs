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

use crate::model::PaintColor;

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
    /// RGB snapshots of the extended `colors.toml` role keys, parsed at
    /// load ([`load`] / [`from_toml_str`]) for the Colors panel groups
    /// (Backgrounds, Foregrounds, Accent, Colors, Brights). Keys are the
    /// lowercase TOML names (`"dark_background"`, `"bright_red"`, …);
    /// values are always [`PaintColor::Rgb`]. Missing/unparseable keys are
    /// simply absent — [`color_groups`] falls back to the role fields and
    /// [`defaults`] above.
    pub snapshots: HashMap<String, PaintColor>,
}

/// Built-in dark fallback (used when no Omarchy theme file is found, e.g.
/// non-Omarchy machines and tests). Legible dark-background values in the
/// Catppuccin Mocha family — deliberately generic, NOT any specific
/// Omarchy theme. Live systems always override these via [`load`].
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
        snapshots: HashMap::new(),
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
    // Omarchy themes use named colors instead of color0–15: fill any slot
    // the file left unset from the names (explicit colorN always wins).
    // 0 muted, 1–6 red/green/yellow/blue/magenta/cyan, 7 foreground,
    // 8 dark_foreground, 9–14 brights, 15 bright_foreground.
    // (`orange`/`brown` have no ANSI slot; they still theme nothing here.)
    let named_fallback: [(usize, &str); 16] = [
        (0, "muted"),
        (1, "red"),
        (2, "green"),
        (3, "yellow"),
        (4, "blue"),
        (5, "magenta"),
        (6, "cyan"),
        (7, "foreground"),
        (8, "dark_foreground"),
        (9, "bright_red"),
        (10, "bright_green"),
        (11, "bright_yellow"),
        (12, "bright_blue"),
        (13, "bright_magenta"),
        (14, "bright_cyan"),
        (15, "bright_foreground"),
    ];
    for (slot, name) in named_fallback {
        let explicit = format!("color{slot}");
        if kv.get(&explicit).is_some() {
            continue;
        }
        if let Some(c) = kv.get(name).and_then(|v| parse_hex_color(v)) {
            theme.ansi[slot] = c;
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
    theme.snapshots = capture_snapshots(&kv);
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
            theme.snapshots = capture_snapshots(&kv);
        }
    }
    theme
}

// ---------------------------------------------------------------------------
// Live theme reload (theme watcher)
// ---------------------------------------------------------------------------

/// Tracks the active theme file across event-loop iterations: when the
/// resolved path or its mtime changes (user ran `omarchy theme set`, or
/// edited the file), the app reloads so chrome, ANSI slots, and panel
/// swatches follow the new theme without a restart. Frozen RGB cells are
/// untouched by design.
pub struct ThemeWatch {
    path: Option<PathBuf>,
    mtime: Option<std::time::SystemTime>,
}

impl ThemeWatch {
    /// Seed from the current state (no spurious reload on first check).
    pub fn new() -> Self {
        let mut w = ThemeWatch {
            path: None,
            mtime: None,
        };
        let _ = w.check();
        w
    }

    /// Poll the filesystem. Returns the new theme path when it changed
    /// since the last check (caller reloads via [`load`]).
    pub fn check(&mut self) -> Option<PathBuf> {
        Self::check_path(self, resolved_path())
    }

    /// Testable core: compare + latch a candidate path.
    pub fn check_path(&mut self, path: Option<PathBuf>) -> Option<PathBuf> {
        let mtime = path
            .as_ref()
            .and_then(|p| std::fs::metadata(p).ok())
            .and_then(|m| m.modified().ok());
        if path != self.path || mtime != self.mtime {
            self.path = path.clone();
            self.mtime = mtime;
            path
        } else {
            None
        }
    }
}

impl Default for ThemeWatch {
    fn default() -> Self {
        ThemeWatch::new()
    }
}

/// Human theme name for status messages: the theme directory name
/// (`…/themes/gruvbox/colors.toml` → `gruvbox`), else `"custom"`.
pub fn theme_name(path: &std::path::Path) -> String {
    path.parent()
        .and_then(|p| p.file_name())
        .and_then(|n| n.to_str())
        .filter(|n| !n.is_empty())
        .unwrap_or("custom")
        .to_string()
}

/// Ratatui style for a canvas cell.
///
/// `fg` resolves against [`Theme::ansi`] via [`resolve`]; `bg == None`
/// means transparent: the preview surface shows through (dark → theme
/// bg, light → theme fg per `preview_dark`). Never panics.
pub fn cell_style(
    fg: PaintColor,
    bg: Option<PaintColor>,
    theme: &Theme,
    preview_dark: bool,
) -> Style {
    let preview_bg = if preview_dark { theme.bg } else { theme.fg };
    let bg_color = bg.map(|c| resolve(c, theme)).unwrap_or(preview_bg);
    Style::default().fg(resolve(fg, theme)).bg(bg_color)
}

// ---------------------------------------------------------------------------
// Colors panel groups (truecolor migration)
// ---------------------------------------------------------------------------

/// Resolve a paint color against the theme: ANSI slots read the live
/// [`Theme::ansi`] entry (out-of-range slots fall back to theme fg, never
/// panic); RGB triples are frozen truecolor.
pub fn resolve(p: PaintColor, theme: &Theme) -> Color {
    match p {
        PaintColor::Ansi(i) => theme.ansi.get(i as usize).copied().unwrap_or(theme.fg),
        PaintColor::Rgb(r, g, b) => Color::Rgb(r, g, b),
    }
}

/// View a [`Color`] as a [`PaintColor`] (theme roles are opaque Rgb in
/// practice; anything else degrades to a neutral slot, never panics).
fn paint_of(c: Color) -> PaintColor {
    match c {
        Color::Rgb(r, g, b) => PaintColor::Rgb(r, g, b),
        Color::Indexed(i) => PaintColor::Ansi(i.min(15)),
        _ => PaintColor::Ansi(7),
    }
}

/// One Colors-panel group: theme-role snapshots or a constant bonus
/// palette (Pico-8, Picotron).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ColorGroup {
    pub name: String,
    pub entries: Vec<ColorEntry>,
}

/// One swatch: display label plus the paint color it applies.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ColorEntry {
    pub label: String,
    pub color: PaintColor,
}

/// One clickable Colors-panel row: a group header, the transparent
/// ("none") background row, or a swatch entry (`group`/`index` address
/// [`color_groups`] output).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ColorRow {
    Header(String),
    Transparent,
    Entry { group: usize, index: usize },
}

/// TOML keys snapshotted for the Colors panel (union of the Backgrounds,
/// Foregrounds, Accent, Colors, and Brights group keys). Only
/// successfully-parsed hex values are stored; anything missing falls back
/// per [`color_groups`].
const SNAPSHOT_KEYS: [&str; 25] = [
    "background",
    "dark_background",
    "darker_background",
    "lighter_background",
    "foreground",
    "dark_foreground",
    "light_foreground",
    "bright_foreground",
    "accent",
    "selection",
    "muted",
    "red",
    "orange",
    "yellow",
    "green",
    "cyan",
    "blue",
    "magenta",
    "brown",
    "bright_red",
    "bright_green",
    "bright_yellow",
    "bright_blue",
    "bright_magenta",
    "bright_cyan",
];

/// Collect the parsed RGB snapshots out of a `colors.toml` key map
/// (malformed values are skipped so the caller falls back).
fn capture_snapshots(kv: &HashMap<String, String>) -> HashMap<String, PaintColor> {
    let mut out = HashMap::new();
    for key in SNAPSHOT_KEYS {
        if let Some(c) = kv.get(key).and_then(|v| parse_hex_color(v)) {
            out.insert(key.to_string(), paint_of(c));
        }
    }
    out
}

/// Snapshot entry helper: `(label, toml key, theme fallback)`.
fn snap(
    theme: &Theme,
    label: &str,
    key: &str,
    fallback: Color,
) -> ColorEntry {
    let color = theme
        .snapshots
        .get(key)
        .copied()
        .unwrap_or_else(|| paint_of(fallback));
    ColorEntry {
        label: label.to_string(),
        color,
    }
}

/// Pico-8 standard palette (constant bonus table): `(name, hex)` in slot
/// order 0–15.
const PICO8: [(&str, &str); 16] = [
    ("black", "#000000"),
    ("dark-blue", "#1d2b53"),
    ("dark-purple", "#7e2553"),
    ("dark-green", "#008751"),
    ("brown", "#ab5236"),
    ("dark-gray", "#5f574f"),
    ("light-gray", "#c2c3c7"),
    ("white", "#fff1e8"),
    ("red", "#ff004d"),
    ("orange", "#ffa300"),
    ("yellow", "#ffec27"),
    ("green", "#00e436"),
    ("blue", "#29adff"),
    ("lavender", "#83769c"),
    ("pink", "#ff77a8"),
    ("peach", "#ffccaa"),
];

/// Picotron secondary palette (constant bonus table): the 16
/// Picotron-exclusive system colors after the Pico-8-compatible first 16
/// (`#1c5eac`…`#ff856d`).
const PICOTRON: [(&str, &str); 16] = [
    ("royal-blue", "#1c5eac"),
    ("teal", "#00a5a1"),
    ("purple", "#754e97"),
    ("deep-teal", "#125359"),
    ("brick", "#742f29"),
    ("coffee", "#492d38"),
    ("tan", "#a28879"),
    ("light-pink", "#ffacc5"),
    ("crimson", "#c3004c"),
    ("orange", "#eb6b00"),
    ("lime", "#90ec42"),
    ("green", "#00b251"),
    ("sky", "#64dff6"),
    ("lilac", "#bd9adf"),
    ("magenta", "#e40dab"),
    ("salmon", "#ff856d"),
];

/// Colors-panel groups in display order: Backgrounds, Foregrounds, Accent,
/// Colors, Brights, Pico-8, Picotron.
///
/// The first five groups are RGB snapshots parsed from the active
/// `colors.toml` at load; missing keys fall back to the current role
/// fields (chromatic colors without a role field fall back to the nearest
/// live ANSI slot: `orange → ansi[10]`, `brown → ansi[1]`; brights fall
/// back to `ansi[9–14]`). Pico-8 and Picotron are constant tables.
pub fn color_groups(theme: &Theme) -> Vec<ColorGroup> {
    let backgrounds = ColorGroup {
        name: "Backgrounds".to_string(),
        entries: vec![
            snap(theme, "background", "background", theme.bg),
            snap(theme, "dark background", "dark_background", theme.bg),
            snap(theme, "darker background", "darker_background", theme.bg),
            snap(theme, "lighter background", "lighter_background", theme.bg),
        ],
    };
    let foregrounds = ColorGroup {
        name: "Foregrounds".to_string(),
        entries: vec![
            snap(theme, "foreground", "foreground", theme.fg),
            snap(theme, "dark foreground", "dark_foreground", theme.fg),
            snap(theme, "light foreground", "light_foreground", theme.fg),
            snap(theme, "bright foreground", "bright_foreground", theme.fg),
        ],
    };
    let accent = ColorGroup {
        name: "Accent".to_string(),
        entries: vec![
            snap(theme, "accent", "accent", theme.accent),
            snap(theme, "selection", "selection", theme.highlight),
            snap(theme, "muted", "muted", theme.dim),
        ],
    };
    let colors = ColorGroup {
        name: "Colors".to_string(),
        entries: vec![
            snap(theme, "red", "red", theme.ansi[1]),
            snap(theme, "orange", "orange", theme.ansi[10]),
            snap(theme, "yellow", "yellow", theme.ansi[3]),
            snap(theme, "green", "green", theme.ansi[2]),
            snap(theme, "cyan", "cyan", theme.ansi[6]),
            snap(theme, "blue", "blue", theme.ansi[4]),
            snap(theme, "magenta", "magenta", theme.ansi[5]),
            snap(theme, "brown", "brown", theme.ansi[1]),
        ],
    };
    let brights = ColorGroup {
        name: "Brights".to_string(),
        entries: vec![
            snap(theme, "bright red", "bright_red", theme.ansi[9]),
            snap(theme, "bright green", "bright_green", theme.ansi[10]),
            snap(theme, "bright yellow", "bright_yellow", theme.ansi[11]),
            snap(theme, "bright blue", "bright_blue", theme.ansi[12]),
            snap(theme, "bright magenta", "bright_magenta", theme.ansi[13]),
            snap(theme, "bright cyan", "bright_cyan", theme.ansi[14]),
        ],
    };
    let pico8 = ColorGroup {
        name: "Pico-8".to_string(),
        entries: PICO8
            .iter()
            .map(|(label, hex)| ColorEntry {
                label: label.to_string(),
                color: PaintColor::from_hex(hex)
                    .expect("Pico-8 table holds valid hex"),
            })
            .collect(),
    };
    let picotron = ColorGroup {
        name: "Picotron".to_string(),
        entries: PICOTRON
            .iter()
            .map(|(label, hex)| ColorEntry {
                label: label.to_string(),
                color: PaintColor::from_hex(hex)
                    .expect("Picotron table holds valid hex"),
            })
            .collect(),
    };
    vec![
        backgrounds,
        foregrounds,
        accent,
        colors,
        brights,
        pico8,
        picotron,
    ]
}

/// Clickable Colors-panel rows in display order: each group's header, its
/// entries — plus the transparent ("none") background row directly under
/// the Backgrounds header (the old row-0 semantics).
pub fn color_rows(theme: &Theme) -> Vec<ColorRow> {
    let groups = color_groups(theme);
    let mut rows = Vec::new();
    for (gi, g) in groups.iter().enumerate() {
        rows.push(ColorRow::Header(g.name.clone()));
        if gi == 0 {
            rows.push(ColorRow::Transparent);
        }
        for (ei, _) in g.entries.iter().enumerate() {
            rows.push(ColorRow::Entry { group: gi, index: ei });
        }
    }
    rows
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
    fn theme_watch_detects_change() {
        let root = std::env::temp_dir().join("omaframe-watch-test");
        let _ = std::fs::create_dir_all(&root);
        let a = root.join("a.toml");
        let b = root.join("b.toml");
        std::fs::write(&a, "accent = \"#111111\"\n").unwrap();
        std::fs::write(&b, "accent = \"#222222\"\n").unwrap();
        let mut w = ThemeWatch {
            path: None,
            mtime: None,
        };
        // First sighting latches and reports.
        assert_eq!(w.check_path(Some(a.clone())), Some(a.clone()));
        // Same path + mtime: quiet.
        assert_eq!(w.check_path(Some(a.clone())), None);
        // Retarget (theme switch): reports.
        assert_eq!(w.check_path(Some(b.clone())), Some(b.clone()));
        // Content edit: mtime moves, reports. (Different content ensures
        // the write lands; filesystems with coarse mtime still differ
        // because the bytes changed... no — mtime may tie. Rewrite twice.)
        std::fs::write(&b, "accent = \"#333333\"\n").unwrap();
        let first = w.check_path(Some(b.clone()));
        std::fs::write(&b, "accent = \"#444444\"\n").unwrap();
        let second = w.check_path(Some(b.clone()));
        assert!(
            first.is_some() || second.is_some(),
            "an edit must surface within two writes"
        );
        // Vanished file reports the change (caller keeps the old theme).
        std::fs::remove_file(&b).unwrap();
        assert_eq!(w.check_path(Some(b.clone())), Some(b.clone()));
        assert_eq!(w.check_path(None), None);
        let _ = std::fs::remove_dir_all(&root);
        // Name helper.
        assert_eq!(
            theme_name(&PathBuf::from("/x/themes/gruvbox/colors.toml")),
            "gruvbox"
        );
        assert_eq!(theme_name(&PathBuf::from("colors.toml")), "custom");
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
    fn named_colors_fill_ansi_slots_unless_explicit() {
        // Real Omarchy themes use names, not colorN: slots follow the names.
        let t = from_toml_str("red = \"#ff0000\"\nblue = \"#0000ff\"\nmuted = \"#111111\"\n");
        assert_eq!(t.ansi[1], Color::Rgb(0xff, 0, 0));
        assert_eq!(t.ansi[4], Color::Rgb(0, 0, 0xff));
        assert_eq!(t.ansi[0], Color::Rgb(0x11, 0x11, 0x11));
        // Explicit colorN always wins over the name.
        let t2 = from_toml_str("color1 = \"#00ff00\"\nred = \"#ff0000\"\n");
        assert_eq!(t2.ansi[1], Color::Rgb(0, 0xff, 0));
    }

    #[test]
    fn hex_forms_resolve_and_cell_style() {
        assert_eq!(parse_hex_color("#fff"), Some(Color::Rgb(0xff, 0xff, 0xff)));
        assert_eq!(parse_hex_color("#000000"), Some(Color::Rgb(0, 0, 0)));
        assert_eq!(parse_hex_color("89b4fa"), Some(Color::Rgb(0x89, 0xb4, 0xfa)));
        assert_eq!(parse_hex_color("not-a-color"), None);
        assert_eq!(parse_hex_color("#zzzzzz"), None);
        assert_eq!(parse_hex_color(""), None);

        let t = defaults();
        // ANSI slots resolve live; RGB triples pass through frozen.
        assert_eq!(resolve(PaintColor::Ansi(4), &t), t.ansi[4]);
        assert_eq!(
            resolve(PaintColor::Rgb(1, 2, 3), &t),
            Color::Rgb(1, 2, 3)
        );
        // Out-of-range slots degrade to theme fg, never panic.
        assert_eq!(resolve(PaintColor::Ansi(99), &t), t.fg);

        // Transparent bg follows the preview flag.
        let s = cell_style(PaintColor::Ansi(4), None, &t, true);
        assert_eq!(s.fg, Some(t.ansi[4]));
        assert_eq!(s.bg, Some(t.bg));
        let s2 = cell_style(PaintColor::Ansi(4), None, &t, false);
        assert_eq!(s2.bg, Some(t.fg));
        let s3 = cell_style(
            PaintColor::Ansi(1),
            Some(PaintColor::Ansi(2)),
            &t,
            true,
        );
        assert_eq!(s3.fg, Some(t.ansi[1]));
        assert_eq!(s3.bg, Some(t.ansi[2]));
        // Frozen RGB paints exactly.
        let s4 = cell_style(
            PaintColor::Rgb(0xff, 0, 0x4d),
            Some(PaintColor::Rgb(0, 0, 0)),
            &t,
            true,
        );
        assert_eq!(s4.fg, Some(Color::Rgb(0xff, 0, 0x4d)));
        assert_eq!(s4.bg, Some(Color::Rgb(0, 0, 0)));
    }

    #[test]
    fn color_group_count_and_names() {
        let groups = color_groups(&defaults());
        let names: Vec<&str> = groups.iter().map(|g| g.name.as_str()).collect();
        assert_eq!(
            names,
            [
                "Backgrounds",
                "Foregrounds",
                "Accent",
                "Colors",
                "Brights",
                "Pico-8",
                "Picotron"
            ]
        );
        assert_eq!(groups[0].entries.len(), 4);
        assert_eq!(groups[1].entries.len(), 4);
        assert_eq!(groups[2].entries.len(), 3);
        assert_eq!(groups[3].entries.len(), 8);
        assert_eq!(groups[4].entries.len(), 6);
        assert_eq!(groups[5].entries.len(), 16);
        assert_eq!(groups[6].entries.len(), 16);
    }

    #[test]
    fn snapshots_override_and_missing_keys_fall_back() {
        // Empty input: snapshots stay empty, groups fall back to role fields.
        let t = from_toml_str("");
        assert!(t.snapshots.is_empty());
        let groups = color_groups(&t);
        assert_eq!(
            groups[0].entries[0].color,
            paint_of(t.bg),
            "missing background falls back to theme bg"
        );
        assert_eq!(groups[1].entries[0].color, paint_of(t.fg));
        assert_eq!(groups[2].entries[0].color, paint_of(t.accent));

        // Present keys become frozen RGB snapshots.
        let t2 = from_toml_str(
            "background = \"#112233\"\nred = \"#ff004d\"\nnot-a-key = \"#abcdef\"\n",
        );
        let g2 = color_groups(&t2);
        assert_eq!(
            g2[0].entries[0].color,
            PaintColor::Rgb(0x11, 0x22, 0x33)
        );
        assert_eq!(
            g2[3].entries[0].color,
            PaintColor::Rgb(0xff, 0, 0x4d)
        );
        // Malformed values are skipped (fall back, never crash).
        let t3 = from_toml_str("background = \"nope\"\n");
        assert_eq!(
            color_groups(&t3)[0].entries[0].color,
            paint_of(t3.bg)
        );
    }

    #[test]
    fn pico8_first_last_hex() {
        let groups = color_groups(&defaults());
        let pico = &groups[5];
        assert_eq!(pico.name, "Pico-8");
        assert_eq!(pico.entries.len(), 16);
        assert_eq!(pico.entries[0].label, "black");
        assert_eq!(pico.entries[0].color, PaintColor::Rgb(0, 0, 0));
        assert_eq!(pico.entries[15].label, "peach");
        assert_eq!(pico.entries[15].color, PaintColor::Rgb(0xff, 0xcc, 0xaa));
    }

    #[test]
    fn picotron_count_first_last() {
        let groups = color_groups(&defaults());
        let pt = &groups[6];
        assert_eq!(pt.name, "Picotron");
        assert_eq!(pt.entries.len(), 16);
        assert_eq!(pt.entries[0].color, PaintColor::Rgb(0x1c, 0x5e, 0xac));
        assert_eq!(pt.entries[15].color, PaintColor::Rgb(0xff, 0x85, 0x6d));
    }

    #[test]
    fn color_rows_shape() {
        let t = defaults();
        let rows = color_rows(&t);
        // 7 headers + Transparent + 57 entries.
        assert_eq!(rows.len(), 7 + 1 + 57);
        assert_eq!(
            rows[0],
            ColorRow::Header("Backgrounds".to_string())
        );
        assert_eq!(rows[1], ColorRow::Transparent);
        assert_eq!(
            rows[2],
            ColorRow::Entry { group: 0, index: 0 }
        );
        // Every entry addresses a real swatch.
        let groups = color_groups(&t);
        for r in &rows {
            if let ColorRow::Entry { group, index } = r {
                assert!(
                    groups.get(*group).and_then(|g| g.entries.get(*index)).is_some(),
                    "dangling entry {group}/{index}"
                );
            }
        }
        // Headers name the groups in order.
        let headers: Vec<String> = rows
            .iter()
            .filter_map(|r| match r {
                ColorRow::Header(n) => Some(n.clone()),
                _ => None,
            })
            .collect();
        assert_eq!(
            headers,
            ["Backgrounds", "Foregrounds", "Accent", "Colors", "Brights", "Pico-8", "Picotron"]
        );
    }
}
