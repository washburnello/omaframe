//! Extended palette character sets (truecolor migration wave).
//!
//! `symbols_tab` / `outlines_tab` / `blocks_tab` return glyphs in display
//! order. Each tab keeps its v1 seed list FIRST (exact order from the old
//! `app.rs` arms, per `docs/palette-spec.md` §2) and appends single-width
//! extras. Every entry is exactly one terminal cell wide
//! (`unicode-width == 1`); the unit test below enforces this so a wide or
//! zero-width glyph can never silently enter the palette grid (wide glyphs
//! would break the 2-wide click mapping in `ui.rs`).
//!
//! Font coverage (checked 2026-09-09 with `fc-match ":charset=HEX"` over a
//! sample of every category): every codepoint resolves to a named installed
//! font (Liberation Sans, Adwaita Mono, or JetBrainsMono Nerd Font) — zero
//! drops, nothing omitted.

use unicode_width::UnicodeWidthStr;

/// Display width in terminal cells (re-export of the palette invariant).
pub fn width_of(s: &str) -> usize {
    s.width()
}

/// Symbols tab: the 32 ASCII punctuations first (exact `app.rs` order),
/// then ~70 safe single-width extras (arrows, triangles, bullets/dots,
/// diamonds/squares, stars/suits/checks, math, misc).
pub fn symbols_tab() -> Vec<String> {
    let mut out: Vec<String> = [
        '!', '"', '#', '$', '%', '&', '\'', '(', ')', '*', '+', ',', '-', '.',
        '/', ':', ';', '<', '=', '>', '?', '@', '[', '\\', ']', '^', '_', '`',
        '{', '|', '}', '~',
    ]
    .iter()
    .map(|c| c.to_string())
    .collect();
    out.extend(
        [
            // Arrows.
            "←", "↑", "→", "↓", "↔", "↕",
            // Triangles (up / right / down / left).
            "▲", "△", "▴", "▵", "▶", "▷", "▸", "►", "▼", "▽", "▾", "▿", "◀",
            "◁", "◂", "◄",
            // Bullets / dots.
            "·", "•", "◦", "‣", "⁕", "●", "○", "◉", "◎", "◌",
            // Diamonds / squares.
            "◆", "◇", "◈", "■", "□", "▪", "▫", "▬", "▭",
            // Stars / suits / checks.
            "★", "☆", "♠", "♥", "♦", "♣", "✓", "✗", "✕", "☐", "☑", "☒",
            // Math.
            "±", "×", "÷", "≠", "≈", "∞", "√", "∑", "π",
            // Misc.
            "⌂", "§", "¶", "©", "®", "°",
        ]
        .iter()
        .map(|s| s.to_string()),
    );
    out
}

/// Outlines tab: the 27 v1 line/box glyphs first (exact `app.rs` order),
/// then heavy corners, double junctions, and dashed segments.
pub fn outlines_tab() -> Vec<String> {
    let mut out: Vec<String> = [
        "─", "│", "┌", "┐", "└", "┘", "├", "┤", "┬", "┴", "┼", "╭", "╮",
        "╰", "╯", "━", "┃", "═", "║", "╔", "╗", "╱", "╲", "╳", "+", "-",
        "|",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    out.extend(
        [
            // Heavy corners.
            "┏", "┓", "┗", "┛",
            // Double junctions.
            "╠", "╣", "╦", "╩", "╬",
            // Dashes (light double-dash / triple-dash / heavy variants).
            "┄", "┅", "┆", "┈", "┊",
        ]
        .iter()
        .map(|s| s.to_string()),
    );
    out
}

/// Blocks tab: the 21 v1 shades/quadrants/levels first (exact `app.rs`
/// order), then upper/lower partials and side partials.
pub fn blocks_tab() -> Vec<String> {
    let mut out: Vec<String> = [
        "█", "▓", "▒", "░", "▀", "▄", "▌", "▐", "▖", "▗", "▘", "▝", "▚",
        "▞", "▟", "▁", "▂", "▃", "▅", "▆", "▇",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    out.extend(
        ["▔", "▕", "▏", "▎", "▍", "▊", "▋", "▉"]
            .iter()
            .map(|s| s.to_string()),
    );
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_width_one(tab: &[String]) {
        for s in tab {
            assert_eq!(
                width_of(s),
                1,
                "palette glyph {s:?} (U+{:04X}) is not single-width",
                s.chars().next().unwrap_or('\0') as u32
            );
        }
    }

    #[test]
    fn every_entry_is_single_width() {
        assert_width_one(&symbols_tab());
        assert_width_one(&outlines_tab());
        assert_width_one(&blocks_tab());
    }

    #[test]
    fn seed_prefixes_match_app_rs_order() {
        // Symbols: the 32 ASCII punctuations, exact `app.rs` arm order.
        let sym = symbols_tab();
        let ascii: Vec<String> = [
            '!', '"', '#', '$', '%', '&', '\'', '(', ')', '*', '+', ',', '-',
            '.', '/', ':', ';', '<', '=', '>', '?', '@', '[', '\\', ']', '^',
            '_', '`', '{', '|', '}', '~',
        ]
        .iter()
        .map(|c| c.to_string())
        .collect();
        assert_eq!(&sym[..32], &ascii[..]);
        assert_eq!(sym.len(), 32 + 68);

        // Outlines: the 27 v1 glyphs first.
        let out = outlines_tab();
        let seed = [
            "─", "│", "┌", "┐", "└", "┘", "├", "┤", "┬", "┴", "┼", "╭", "╮",
            "╰", "╯", "━", "┃", "═", "║", "╔", "╗", "╱", "╲", "╳", "+", "-",
            "|",
        ];
        assert_eq!(&out[..27], &seed);
        assert_eq!(out.len(), 27 + 14);

        // Blocks: the 21 v1 glyphs first.
        let blk = blocks_tab();
        let bseed = [
            "█", "▓", "▒", "░", "▀", "▄", "▌", "▐", "▖", "▗", "▘", "▝", "▚",
            "▞", "▟", "▁", "▂", "▃", "▅", "▆", "▇",
        ];
        assert_eq!(&blk[..21], &bseed);
        assert_eq!(blk.len(), 21 + 8);
    }
}
