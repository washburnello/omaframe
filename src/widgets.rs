//! Widget stamps (Phase 3): parametric ASCII UI kit baked to cells.
//!
//! The catalog lives in `assets/widgets.toml` (`[[widget]]` tables with
//! `kind`, `label`, `default_w/h`, `styles`, and `template_<style>`
//! renderings; `{label}` and `{bar}` placeholders; `"""` triple-quoted
//! multi-line templates). This module parses that file with a tiny
//! purpose-built reader (no new dependencies) and bakes widgets to cell
//! patches at any size:
//!
//! - 1-row widgets stretch by extending the last horizontal-rule run
//!   (`─ ━ ═ -`) or padding/truncating; the label recenters where the
//!   template centers it (kept simple: left-anchored, padded).
//! - `panel` rebuilds its border box at the target size (rounded / light /
//!   double / ascii corner sets) with title + `[X]`.
//! - `scrollbar` scales its track (`█`/`░`, `#`/`-`) to the target
//!   height (vertical) or width (horizontal).
//! - `divider` repeats its rule char across the width (× height rows).
//! - `progress` recomputes `{bar}` fill from the label's trailing `%`
//!   (default 50%) across the bracket width.
//! - Anything else falls back to pad/truncate per line (last line repeats
//!   when `h` exceeds the template).
//!
//! Bake output is relative `(x, y)` cells with the caller's colors; the
//! caller (TUI stamp/move/resize) offsets them into the document and owns
//! undo. Unknown kinds bake to nothing (`None`).

use std::collections::HashMap;
use std::sync::OnceLock;

use crate::model::{Cell, PaintColor};

/// One catalog entry: identity + defaults + style templates.
#[derive(Clone, Debug)]
pub struct WidgetSpec {
    pub kind: String,
    pub label: String,
    pub default_w: u32,
    pub default_h: u32,
    pub styles: Vec<String>,
    /// `template_<style>` values keyed by style name.
    pub templates: HashMap<String, String>,
}

impl WidgetSpec {
    /// Template for `style`, else the first listed style's template, else
    /// any template, else empty.
    pub fn template(&self, style: &str) -> &str {
        if let Some(t) = self.templates.get(style) {
            return t;
        }
        if let Some(first) = self.styles.first() {
            if let Some(t) = self.templates.get(first.as_str()) {
                return t;
            }
        }
        self.templates.values().next().map(String::as_str).unwrap_or("")
    }
}

/// Parsed catalog, sorted by kind.
pub fn catalog() -> &'static Vec<WidgetSpec> {
    static CACHE: OnceLock<Vec<WidgetSpec>> = OnceLock::new();
    CACHE.get_or_init(|| {
        let mut specs = parse_catalog(include_str!("../assets/widgets.toml"));
        specs.sort_by(|a, b| a.kind.cmp(&b.kind));
        specs
    })
}

/// Look up one kind by name.
pub fn spec(kind: &str) -> Option<&'static WidgetSpec> {
    catalog().iter().find(|s| s.kind == kind)
}

/// Minimal reader for the `assets/widgets.toml` subset: `[[widget]]`
/// tables, `key = "value"`, `key = ["a", "b"]`, and `key = """multi…"""`
/// values. Anything else (comments, blanks) is skipped. Never panics.
fn parse_catalog(src: &str) -> Vec<WidgetSpec> {
    let mut specs = Vec::new();
    let mut cur: HashMap<String, String> = HashMap::new();
    let mut in_widget = false;
    let mut lines = src.lines().peekable();
    while let Some(line) = lines.next() {
        let t = line.trim();
        if t.is_empty() || t.starts_with('#') {
            continue;
        }
        if t == "[[widget]]" {
            if in_widget {
                if let Some(spec) = build_spec(&cur) {
                    specs.push(spec);
                }
            }
            cur = HashMap::new();
            in_widget = true;
            continue;
        }
        if !in_widget {
            continue;
        }
        let Some(eq) = t.find('=') else { continue };
        let key = t[..eq].trim().to_string();
        let mut val = t[eq + 1..].trim().to_string();
        if val.starts_with("\"\"\"") {
            // Triple-quoted: consume until the closing fence.
            let mut acc = val.trim_start_matches("\"\"\"").to_string();
            if acc.ends_with("\"\"\"") && acc.len() > 3 {
                acc.truncate(acc.len() - 3);
                cur.insert(key, acc);
                continue;
            }
            if acc.ends_with("\"\"\"") {
                acc.truncate(acc.len() - 3);
                cur.insert(key, acc);
                continue;
            }
            let mut done = false;
            for next in lines.by_ref() {
                if let Some(end) = next.find("\"\"\"") {
                    acc.push('\n');
                    acc.push_str(&next[..end]);
                    done = true;
                    break;
                }
                acc.push('\n');
                acc.push_str(next);
            }
            if done || !acc.is_empty() {
                cur.insert(key, acc);
            }
            continue;
        }
        if val.starts_with('[') {
            // String list: collect quoted items (single-line in practice).
            let items: Vec<String> = val
                .split('"')
                .skip(1)
                .step_by(2)
                .map(|s| s.to_string())
                .collect();
            cur.insert(key + "\u{0}list", items.join("\u{0}"));
            continue;
        }
        // Plain value: strip one layer of quotes, or parse a number.
        val = val.trim_matches('"').to_string();
        cur.insert(key, val);
    }
    if in_widget {
        if let Some(spec) = build_spec(&cur) {
            specs.push(spec);
        }
    }
    specs
}

fn get_list(map: &HashMap<String, String>, key: &str) -> Vec<String> {
    map.get(&(key.to_string() + "\u{0}list"))
        .map(|s| s.split('\u{0}').map(|s| s.to_string()).collect())
        .unwrap_or_default()
}

fn get_u32(map: &HashMap<String, String>, key: &str, fallback: u32) -> u32 {
    map.get(key)
        .and_then(|s| s.parse::<u32>().ok())
        .filter(|n| *n > 0)
        .unwrap_or(fallback)
}

fn build_spec(map: &HashMap<String, String>) -> Option<WidgetSpec> {
    let kind = map.get("kind")?.clone();
    if kind.is_empty() {
        return None;
    }
    let mut templates = HashMap::new();
    for (k, v) in map {
        if let Some(style) = k.strip_prefix("template_") {
            templates.insert(style.to_string(), v.clone());
        }
    }
    if templates.is_empty() {
        return None;
    }
    Some(WidgetSpec {
        label: map.get("label").cloned().unwrap_or_default(),
        default_w: get_u32(map, "default_w", 8),
        default_h: get_u32(map, "default_h", 1),
        styles: get_list(map, "styles"),
        templates,
        kind,
    })
}

/// Horizontal rule characters a 1-row template can stretch.
fn is_rule(ch: char) -> bool {
    matches!(ch, '─' | '━' | '═' | '-')
}

/// Bake `kind` at `w × h` with `label` (falls back to the spec default
/// when empty and the template needs one) in style `style`. Returns
/// relative `(x, y, ch)` cells, or `None` for unknown kinds. Colors come
/// from the caller (`fg`, `bg`).
pub fn bake(
    kind: &str,
    w: u32,
    h: u32,
    label: &str,
    style: &str,
    fg: PaintColor,
    bg: Option<PaintColor>,
) -> Option<Vec<((i32, i32), Cell)>> {
    let spec = spec(kind)?;
    let w = w.max(1) as usize;
    let h = h.max(1) as usize;
    let label = if label.is_empty() {
        spec.label.as_str()
    } else {
        label
    };
    let cells: Vec<(usize, usize, String)> = match kind {
        "panel" => bake_panel(spec, style, w, h, label),
        "scrollbar" => bake_scrollbar(spec, style, w, h),
        "divider" => bake_divider(spec, style, w, h),
        "progress" => bake_progress(spec, style, w, h, label),
        _ => bake_generic(spec, style, w, h, label),
    };
    Some(
        cells
            .into_iter()
            .map(|(x, y, ch)| {
                (
                    (x as i32, y as i32),
                    Cell::new(ch, fg.clone(), bg.clone()),
                )
            })
            .collect(),
    )
}

/// Substitute `{label}` (caller-resolved default) into a template.
fn with_label(template: &str, label: &str) -> String {
    template.replace("{label}", label)
}

/// Fit one template line to `w`: extend the LAST horizontal-rule run when
/// the line is short, else pad with spaces; truncate when long.
fn fit_line(line: &str, w: usize) -> String {
    let mut chars: Vec<char> = line.chars().collect();
    if chars.len() > w {
        chars.truncate(w);
        return chars.into_iter().collect();
    }
    if chars.len() == w {
        return line.to_string();
    }
    // Find the last rule run and extend it.
    let mut run_start: Option<usize> = None;
    let mut run_end = 0;
    let mut i = 0;
    while i < chars.len() {
        if is_rule(chars[i]) {
            let s = i;
            while i < chars.len() && chars[i] == chars[s] {
                i += 1;
            }
            run_start = Some(s);
            run_end = i;
        } else {
            i += 1;
        }
    }
    if let Some(s) = run_start {
        let need = w - chars.len();
        let fill = chars[s];
        let mut out = Vec::with_capacity(w);
        out.extend_from_slice(&chars[..run_end]);
        out.extend(std::iter::repeat_n(fill, need));
        out.extend_from_slice(&chars[run_end..]);
        out.into_iter().collect()
    } else {
        let mut s = line.to_string();
        while s.chars().count() < w {
            s.push(' ');
        }
        s
    }
}

/// Generic bake: substitute the label, fit every line to `w`, repeat the
/// last line while short, truncate while tall.
fn bake_generic(
    spec: &WidgetSpec,
    style: &str,
    w: usize,
    h: usize,
    label: &str,
) -> Vec<(usize, usize, String)> {
    let rendered = with_label(spec.template(style), label);
    let mut lines: Vec<String> = rendered.lines().map(|l| fit_line(l, w)).collect();
    if lines.is_empty() {
        lines.push(" ".repeat(w));
    }
    while lines.len() < h {
        lines.push(lines.last().cloned().unwrap_or_else(|| " ".repeat(w)));
    }
    lines.truncate(h);
    let mut out = Vec::new();
    for (y, line) in lines.iter().enumerate() {
        for (x, ch) in line.chars().enumerate() {
            if ch != ' ' {
                out.push((x, y, ch.to_string()));
            }
        }
    }
    out
}

/// Panel: rebuilt border box (`╭─ {label} ──[X]─╮` / style set) with
/// repeating `│ │` body rows.
fn bake_panel(
    _spec: &WidgetSpec,
    style: &str,
    w: usize,
    h: usize,
    label: &str,
) -> Vec<(usize, usize, String)> {
    let (tl, tr, bl, br, horiz, vert): (char, char, char, char, char, char) =
        match style {
            "double" => ('╔', '╗', '╚', '╝', '═', '║'),
            "ascii" => ('+', '+', '+', '+', '-', '|'),
            "light" => ('┌', '┐', '└', '┘', '─', '│'),
            _ => ('╭', '╮', '╰', '╯', '─', '│'), // rounded + default
        };
    let mut out = Vec::new();
    if w == 1 {
        for y in 0..h {
            out.push((0, y, vert.to_string()));
        }
        return out;
    }
    // Title row: `╭─ {label} ──[X]─╮` fitted to w.
    let title_core = format!("{tl}─ {label} ");
    let close = "[X]";
    let mut top: Vec<char> = title_core.chars().collect();
    // Fill with horiz until the close box + corner fit.
    while top.len() + close.chars().count() + 2 < w {
        top.push(horiz);
        top.push(horiz);
    }
    // Trim the fill if the label overflows.
    while top.len() + close.chars().count() + 1 > w && top.len() > 2 {
        top.pop();
    }
    top.extend(close.chars());
    while top.len() < w.saturating_sub(1) {
        top.push(horiz);
    }
    top.push(tr);
    while top.len() > w {
        top.pop();
    }
    for (x, ch) in top.into_iter().enumerate() {
        out.push((x, 0, ch.to_string()));
    }
    for y in 1..h.saturating_sub(1) {
        out.push((0, y, vert.to_string()));
        if w > 1 {
            out.push((w - 1, y, vert.to_string()));
        }
    }
    if h > 1 {
        out.push((0, h - 1, bl.to_string()));
        for x in 1..w.saturating_sub(1) {
            out.push((x, h - 1, horiz.to_string()));
        }
        if w > 1 {
            out.push((w - 1, h - 1, br.to_string()));
        }
    }
    // Drop spaces (corners/edges are never spaces here anyway).
    out.into_iter()
        .filter(|(_, _, ch)| ch != " ")
        .collect()
}

/// Scrollbar: vertical column (`▲ █ ░… ▼`) fitted to `h`, or horizontal
/// row (`◄ █ ░… ►`) fitted to `w` when the style or geometry is wide.
fn bake_scrollbar(
    spec: &WidgetSpec,
    style: &str,
    w: usize,
    h: usize,
) -> Vec<(usize, usize, String)> {
    let horizontal = style.contains("horizontal") || (w > h && w > 1);
    if horizontal {
        let (l, thumb, track, r): (char, char, char, char) = if style.contains("ascii") {
            ('<', '#', '-', '>')
        } else {
            ('◄', '█', '░', '►')
        };
        let mut row = vec![l, thumb];
        while row.len() + 1 < w {
            row.push(track);
        }
        if w > 1 {
            row.push(r);
        }
        row.truncate(w);
        row.into_iter()
            .enumerate()
            .map(|(x, ch)| (x, 0, ch.to_string()))
            .collect()
    } else {
        let tpl = with_label(spec.template(style), "");
        let mut glyphs: Vec<char> = tpl.lines().flat_map(|l| l.chars()).collect();
        if glyphs.is_empty() {
            glyphs = vec!['▲', '█', '░', '▼'];
        }
        let (top, bottom) = (glyphs[0], *glyphs.last().unwrap_or(&'▼'));
        let mid: Vec<char> = if glyphs.len() > 2 {
            glyphs[1..glyphs.len() - 1].to_vec()
        } else {
            vec!['█', '░']
        };
        let mut col = vec![top];
        let mut mi = 0;
        while col.len() + 1 < h {
            col.push(mid[mi % mid.len()]);
            mi += 1;
        }
        if h > 1 {
            col.push(bottom);
        }
        col.truncate(h);
        col.into_iter()
            .enumerate()
            .map(|(y, ch)| (0, y, ch.to_string()))
            .collect()
    }
}

/// Divider: the template's first rule char repeated across `w` (× `h`).
fn bake_divider(
    spec: &WidgetSpec,
    style: &str,
    w: usize,
    h: usize,
) -> Vec<(usize, usize, String)> {
    let tpl = with_label(spec.template(style), "");
    let rule = tpl.chars().find(|c| is_rule(*c)).unwrap_or('─');
    let mut out = Vec::new();
    for y in 0..h {
        for x in 0..w {
            out.push((x, y, rule.to_string()));
        }
    }
    out
}

/// Progress: `{bar}` recomputed from the label's trailing `%` (default
/// 50%) across the bracket interior; label appended after `]`.
fn bake_progress(
    _spec: &WidgetSpec,
    _style: &str,
    w: usize,
    h: usize,
    label: &str,
) -> Vec<(usize, usize, String)> {
    let pct: f64 = label
        .trim_end_matches('%')
        .split_whitespace()
        .last()
        .and_then(|s| s.parse::<f64>().ok())
        .map(|n| (n / 100.0).clamp(0.0, 1.0))
        .unwrap_or(0.5);
    let label_part = format!(" {label}");
    // Brackets + space + label must fit; bar takes the rest (min 1).
    let fixed = 2 + label_part.chars().count();
    let bar_w = w.saturating_sub(fixed).max(1);
    let fill = ((pct * bar_w as f64).round() as usize).min(bar_w);
    let mut row = String::from("[");
    for _ in 0..fill {
        row.push('█');
    }
    for _ in fill..bar_w {
        row.push('░');
    }
    row.push(']');
    row.push_str(&label_part);
    let row = fit_line(&row, w);
    let mut out = Vec::new();
    for (x, ch) in row.chars().enumerate() {
        if ch != ' ' {
            out.push((x, 0, ch.to_string()));
        }
    }
    // Taller than one row: repeat the bar row (progress is 1-D).
    for y in 1..h {
        for (x, ch) in row.chars().enumerate() {
            if ch != ' ' {
                out.push((x, y, ch.to_string()));
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_parses_all_kinds() {
        let cat = catalog();
        assert!(cat.len() >= 12, "kinds: {}", cat.len());
        for kind in [
            "button", "input", "dropdown", "radio", "checkbox", "toggle",
            "close", "scrollbar", "progress", "divider", "panel", "tabs",
        ] {
            assert!(spec(kind).is_some(), "missing {kind}");
        }
    }

    #[test]
    fn bake_button_exact_and_scaled() {
        let fg = PaintColor::Ansi(7);
        // Default 8×1 renders `[ OK ]` verbatim.
        let cells = bake("button", 8, 1, "OK", "light", fg.clone(), None).unwrap();
        let row: String = {
            let mut s = vec![' '; 8];
            for ((x, _), c) in &cells {
                s[*x as usize] = c.ch.chars().next().unwrap();
            }
            s.into_iter().collect()
        };
        assert_eq!(row, "[ OK ]  ");
        // Wider keeps the label and grows (rule or padding).
        let cells = bake("button", 12, 1, "OK", "light", fg.clone(), None).unwrap();
        assert!(cells.iter().all(|((x, _), _)| *x < 12));
        assert!(cells.iter().any(|(_, c)| c.ch == "O"));
        // Unknown kind → None.
        assert!(bake("nope", 8, 1, "", "light", fg, None).is_none());
    }

    #[test]
    fn bake_progress_fill_math() {
        let fg = PaintColor::Ansi(7);
        let cells = bake("progress", 14, 1, "60%", "light", fg, None).unwrap();
        let mut row = vec![' '; 14];
        for ((x, _), c) in &cells {
            row[*x as usize] = c.ch.chars().next().unwrap();
        }
        let row: String = row.into_iter().collect();
        // Interior is 14 - ("[]" + " 60%") = 8 wide; 60% of 8 ≈ 5.
        assert_eq!(&row[..1], "[");
        let fill = row.chars().filter(|c| *c == '█').count();
        let track = row.chars().filter(|c| *c == '░').count();
        assert_eq!(fill + track, 8);
        assert!((fill as i32 - 5).abs() <= 1, "fill={fill} row={row}");
        assert!(row.ends_with("60%"));
    }

    #[test]
    fn bake_panel_reflow() {
        let fg = PaintColor::Ansi(7);
        let cells = bake("panel", 24, 5, "Title", "rounded", fg.clone(), None).unwrap();
        let at = |x: usize, y: usize| -> Option<char> {
            cells
                .iter()
                .find(|((cx, cy), _)| *cx as usize == x && *cy as usize == y)
                .and_then(|(_, c)| c.ch.chars().next())
        };
        assert_eq!(at(0, 0), Some('╭'));
        assert_eq!(at(23, 0), Some('╮'));
        assert_eq!(at(0, 4), Some('╰'));
        assert_eq!(at(23, 4), Some('╯'));
        assert_eq!(at(0, 2), Some('│'));
        assert_eq!(at(23, 2), Some('│'));
        // Narrow panel still has four corners.
        let cells = bake("panel", 10, 3, "T", "ascii", fg, None).unwrap();
        let corners = cells
            .iter()
            .filter(|(_, c)| c.ch == "+")
            .count();
        assert_eq!(corners, 4);
    }

    #[test]
    fn bake_scrollbar_and_divider_scale() {
        let fg = PaintColor::Ansi(7);
        let cells = bake("scrollbar", 1, 8, "", "vertical", fg.clone(), None).unwrap();
        assert_eq!(cells.len(), 8);
        assert_eq!(cells[0].1.ch, "▲");
        assert_eq!(cells[7].1.ch, "▼");
        let cells = bake("divider", 20, 1, "", "light", fg, None).unwrap();
        assert_eq!(cells.len(), 20);
        assert!(cells.iter().all(|(_, c)| c.ch == "─"));
    }

    #[test]
    fn bake_label_and_colors() {
        let fg = PaintColor::Theme("red".to_string());
        let cells = bake("checkbox", 9, 1, "no", "unchecked", fg.clone(), None).unwrap();
        let text: String = {
            let mut s = vec![' '; 9];
            for ((x, _), c) in &cells {
                s[*x as usize] = c.ch.chars().next().unwrap();
            }
            s.into_iter().collect()
        };
        assert_eq!(text, "[ ] no   ");
        assert!(cells.iter().all(|(_, c)| c.fg == fg && c.bg.is_none()));
    }
}
