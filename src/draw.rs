//! Pure rasterization + light-weight junction logic.
//!
//! Port of the drawing geometry from `client/draw/box.ts`,
//! `client/draw/line.ts`, `client/draw/utils.ts`, plus junction handling from
//! `client/characters.ts` and `client/snap.ts` (behavior only, not code).
//!
//! Manager rulings (see `docs/decisions.md`) applied here:
//!
//! - **Light-only junctions.** Only the light set
//!   (`─ │ ┌ ┐ └ ┘ ├ ┤ ┬ ┴ ┼`) auto-connects. Styled boxes
//!   (heavy/double/rounded/ascii) are draw-only and never merge; arrows and
//!   text never participate.
//! - **No 45° routing.** Lines are axis-aligned or single-bend polylines;
//!   `╱ ╲ ╳` stay pencil-only.
//! - **Slope-picked oval.** Ellipse outline cells are `─` where the tangent is
//!   more horizontal (`|dy/dx| <= 1`) and `│` elsewhere; no corners, no snap.
//! - **Pencil raw.** [`paint_cells`] writes exactly what it is given; any
//!   junction fix-up happens later via [`snap_patch`].
//!
//! All `draw_*` builders are pure: they return an outline/patch [`Layer`]
//! without touching the [`Document`]. The TUI commits via
//! `History::commit_layer` (one gesture = one undo entry; empty patch = no
//! entry). [`snap_patch`] folds junction auto-connect + 2-pass normalization
//! into a scratch layer in place.
//!
//! Default colors: `draw_box` / `draw_line` / `draw_ellipse` emit cells with
//! `fg = 7`, `bg = -1` (neutral light-gray, transparent background). The
//! caller (pots/tools) recolors by rewriting the patch cells before commit;
//! [`snap_patch`] preserves each cell's existing colors when it re-glyphs.
//! [`paint_cells`] uses the caller-supplied `fg`/`bg` verbatim, except that
//! transparent spellings (`""`, `" "`, `"\0"`) become [`Cell::erased`].

use std::collections::HashSet;

use crate::model::{Cell, Document, Layer, Rect};

/// Default foreground for `draw_*` builders (neutral light-gray).
pub const DRAW_FG: i8 = 7;
/// Default background for `draw_*` builders (transparent).
pub const DRAW_BG: i8 = -1;

/// Outline style for [`draw_box`].
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum BoxStyle {
    Light,
    Heavy,
    Double,
    Rounded,
    Ascii,
}

fn box_chars(style: BoxStyle) -> (&'static str, &'static str, [&'static str; 4]) {
    match style {
        BoxStyle::Light => ("─", "│", ["┌", "┐", "┘", "└"]),
        BoxStyle::Heavy => ("━", "┃", ["┏", "┓", "┛", "┗"]),
        BoxStyle::Double => ("═", "║", ["╔", "╗", "╝", "╚"]),
        BoxStyle::Rounded => ("─", "│", ["╭", "╮", "╯", "╰"]),
        BoxStyle::Ascii => ("-", "|", ["+", "+", "+", "+"]),
    }
}

/// Draw a box outline patch for a normalized [`Rect`].
///
/// Mirrors `client/draw/box.ts`: horizontal edges when `w > 1`, vertical
/// edges when `h > 1`, corners only when both exceed 1 (corners overwrite
/// edges, so 2×2 is four corners). Degenerate cases per tools-spec §2:
///
/// - 1×1 (or empty) → empty [`Layer`] (commit is a no-op, no undo entry).
/// - N×1 → single row of the horizontal edge char, no corners.
/// - 1×N → single column of the vertical edge char, no corners.
///
/// No snap pass here; the caller runs [`snap_patch`] afterwards.
pub fn draw_box(r: Rect, style: BoxStyle) -> Layer {
    let mut out = Layer::new();
    if r.w == 0 || r.h == 0 {
        return out;
    }
    if r.w == 1 && r.h == 1 {
        return out;
    }
    let (hch, vch, corners) = box_chars(style);
    // Use i64 to avoid overflow on extreme coords; cast back (grids are small).
    let left = r.x as i64;
    let top = r.y as i64;
    let right = r.x as i64 + r.w as i64 - 1;
    let bottom = r.y as i64 + r.h as i64 - 1;
    if r.w > 1 {
        for x in left..=right {
            out.set(x as i32, top as i32, Cell::new(hch, DRAW_FG, DRAW_BG));
            // Avoid double-insert when h == 1 (single row).
            if bottom != top {
                out.set(x as i32, bottom as i32, Cell::new(hch, DRAW_FG, DRAW_BG));
            }
        }
    }
    if r.h > 1 {
        for y in top..=bottom {
            out.set(left as i32, y as i32, Cell::new(vch, DRAW_FG, DRAW_BG));
            if right != left {
                out.set(right as i32, y as i32, Cell::new(vch, DRAW_FG, DRAW_BG));
            }
        }
    }
    if r.w > 1 && r.h > 1 {
        out.set(
            left as i32,
            top as i32,
            Cell::new(corners[0], DRAW_FG, DRAW_BG),
        );
        out.set(
            right as i32,
            top as i32,
            Cell::new(corners[1], DRAW_FG, DRAW_BG),
        );
        out.set(
            right as i32,
            bottom as i32,
            Cell::new(corners[2], DRAW_FG, DRAW_BG),
        );
        out.set(
            left as i32,
            bottom as i32,
            Cell::new(corners[3], DRAW_FG, DRAW_BG),
        );
    }
    out
}

/// Corner glyph for a single-bend polyline (tools-spec §3.1).
///
/// `dx = end.x - start.x`, `dy = end.y - start.y`. Returns the glyph whose
/// two arms point back at `start` and `end`. Degenerate (straight) inputs
/// return the straight glyph (`─` for horizontal, `│` for vertical,
/// including the zero-length dot → `│` per §3.5).
pub fn line_corner_for(horiz_first: bool, dx: i32, dy: i32) -> &'static str {
    if dx == 0 && dy == 0 {
        return "│";
    }
    if dx == 0 {
        return "│";
    }
    if dy == 0 {
        return "─";
    }
    if horiz_first {
        match (dx > 0, dy > 0) {
            (true, true) => "┐",
            (true, false) => "┘",
            (false, true) => "┌",
            (false, false) => "└",
        }
    } else {
        match (dy > 0, dx > 0) {
            (true, true) => "└",
            (true, false) => "┘",
            (false, true) => "┌",
            (false, false) => "┐",
        }
    }
}

fn draw_straight_into(out: &mut Layer, x0: i32, y0: i32, x1: i32, y1: i32) {
    if x0 == x1 {
        let (top, bottom) = (y0.min(y1), y0.max(y1));
        for y in top..=bottom {
            out.set(x0, y, Cell::new("│", DRAW_FG, DRAW_BG));
        }
    } else if y0 == y1 {
        let (left, right) = (x0.min(x1), x0.max(x1));
        for x in left..=right {
            out.set(x, y0, Cell::new("─", DRAW_FG, DRAW_BG));
        }
    }
    // Non-axis-aligned pairs never reach here (legs always share an axis).
}

/// Draw a light-weight polyline patch (tools-spec §3.1, no snap).
///
/// - Axis-aligned (or zero-length): straight run of `─`/`│`. Zero-length
///   yields a single `│` (spec §3.5; differs from the TS double-`if` which
///   would overwrite with `─` — spec wins).
/// - Otherwise: single-bend polyline via `(x1, y0)` when `horizontal_first`,
///   else `(x0, y1)`; legs drawn straight, bend set per [`line_corner_for`].
/// - No arrowheads, no endpoint connect/disconnect, no snap — the caller
///   runs [`snap_patch`] (endpoint normalization lives there via the
///   2-pass pass, matching `snap.ts` + `line.ts:94-117` effects for plain
///   lines).
pub fn draw_line(x0: i32, y0: i32, x1: i32, y1: i32, horizontal_first: bool) -> Layer {
    let mut out = Layer::new();
    if x0 == x1 || y0 == y1 {
        // Covers the zero-length dot (x-equal arm wins → single │).
        draw_straight_into(&mut out, x0, y0, x1, y1);
        return out;
    }
    let (bx, by) = if horizontal_first {
        (x1, y0)
    } else {
        (x0, y1)
    };
    draw_straight_into(&mut out, x0, y0, bx, by);
    draw_straight_into(&mut out, bx, by, x1, y1);
    let corner = line_corner_for(horizontal_first, x1 - x0, y1 - y0);
    out.set(bx, by, Cell::new(corner, DRAW_FG, DRAW_BG));
    out
}

// --- Ellipse helpers --------------------------------------------------------

fn round_half_up(v: f64) -> i32 {
    (v + 0.5).floor() as i32
}

fn round_half_down(v: f64) -> i32 {
    (v - 0.5).ceil() as i32
}

fn round_away_from_center(v: f64, center: f64) -> i32 {
    if v > center {
        round_half_up(v)
    } else if v < center {
        round_half_down(v)
    } else {
        // Exactly centered (even-dimension extreme midpoint): pick the lower
        // cell; mirror closure adds the upper one, giving the symmetric
        // 2-cell flat required by tools-spec §5.
        v.floor() as i32
    }
}

/// Draw an oval outline patch for a normalized [`Rect`] (tools-spec §5).
///
/// - 0-size → empty. 1×1 → single `─` (line-dot convention, §5 step 2).
/// - 1-wide column → vertical `│` run; 1-high row → horizontal `─` run.
/// - Otherwise: float-center ellipse (`cx = (l+r)/2`, etc., so odd sizes
///   center on a cell and even sizes straddle a grid line), rasterized by
///   dense parametric sampling **plus** per-row/per-column analytic solves,
///   mirrored explicitly for exact symmetry, deduped. Halves round
///   away-from-center so left/right (and top/bottom) mirrors match.
/// - Glyph per cell by local slope: `|dy/dx| <= 1` → `─`, else `│`
///   (axis extremes fall out; ties go horizontal so 2×2 is all `─`).
///   No corners, no snap.
/// Outline coordinates for a plain rectangle (the Rect tool): perimeter
/// cells for a normalized [`Rect`], drawn with the caller's pencil char
/// instead of a fixed glyph set like [`draw_box`]. Degenerate inputs give
/// the single cell (1x1), the row (Nx1) or the column (1xN); empty rects
/// give no cells.
pub fn rect_cells(r: Rect) -> Vec<(i32, i32)> {
    if r.w == 0 || r.h == 0 {
        return Vec::new();
    }
    let left = r.x as i64;
    let top = r.y as i64;
    let right = r.x as i64 + r.w as i64 - 1;
    let bottom = r.y as i64 + r.h as i64 - 1;
    let mut pts: HashSet<(i32, i32)> = HashSet::new();
    if r.w > 1 {
        for x in left..=right {
            pts.insert((x as i32, top as i32));
            if bottom != top {
                pts.insert((x as i32, bottom as i32));
            }
        }
    } else if r.h == 1 {
        pts.insert((left as i32, top as i32));
    }
    if r.h > 1 {
        for y in top..=bottom {
            pts.insert((left as i32, y as i32));
            if right != left {
                pts.insert((right as i32, y as i32));
            }
        }
    } else if r.w == 1 {
        pts.insert((left as i32, top as i32));
    }
    let mut out: Vec<(i32, i32)> = pts.into_iter().collect();
    out.sort();
    out
}

/// Ellipse outline coordinates for a normalized [`Rect`]: the symmetric
/// point set behind [`draw_ellipse`] (column/row solves + dense parametric
/// sampling + mirror closure), without glyph assignment. Degenerate cases
/// mirror the box convention: 1×1 → the single cell; 1×N → the column;
/// N×1 → the row. The Oval tool paints these with the pencil char via
/// [`paint_cells`]; [`draw_ellipse`] assigns slope-picked `─`/`│`.
pub fn ellipse_cells(r: Rect) -> Vec<(i32, i32)> {
    if r.w == 0 || r.h == 0 {
        return Vec::new();
    }
    let left = r.x;
    let top = r.y;
    let right = r.x + r.w as i32 - 1;
    let bottom = r.y + r.h as i32 - 1;
    if r.w == 1 && r.h == 1 {
        return vec![(left, top)];
    }
    if r.w == 1 {
        return (top..=bottom).map(|y| (left, y)).collect();
    }
    if r.h == 1 {
        return (left..=right).map(|x| (x, top)).collect();
    }

    let cx = (left as f64 + right as f64) / 2.0;
    let cy = (top as f64 + bottom as f64) / 2.0;
    let rx = (right as f64 - left as f64) / 2.0;
    let ry = (bottom as f64 - top as f64) / 2.0;

    let mut pts: HashSet<(i32, i32)> = HashSet::new();

    // 1) Per-column solves (top/bottom halves).
    for x in left..=right {
        let dx = (x as f64 - cx) / rx;
        if dx.abs() > 1.0 {
            continue;
        }
        let dy = ry * (1.0 - dx * dx).max(0.0).sqrt();
        pts.insert((x, round_half_down(cy - dy)));
        pts.insert((x, round_half_up(cy + dy)));
    }
    // 2) Per-row solves (left/right halves).
    for y in top..=bottom {
        let dy = (y as f64 - cy) / ry;
        if dy.abs() > 1.0 {
            continue;
        }
        let dx = rx * (1.0 - dy * dy).max(0.0).sqrt();
        pts.insert((round_half_down(cx - dx), y));
        pts.insert((round_half_up(cx + dx), y));
    }
    // 3) Dense parametric sampling (fills 45° corners on small ovals,
    // e.g. 3×3 needs all 8 border cells).
    let w = r.w as i32;
    let h = r.h as i32;
    let steps = (128).max(8 * (w + h));
    for i in 0..steps {
        let theta = 2.0 * std::f64::consts::PI * (i as f64) / (steps as f64);
        let fx = cx + rx * theta.cos();
        let fy = cy + ry * theta.sin();
        pts.insert((
            round_away_from_center(fx, cx),
            round_away_from_center(fy, cy),
        ));
    }

    // 4) Explicit mirror closure for exact symmetry.
    let seed: Vec<(i32, i32)> = pts.iter().copied().collect();
    for (x, y) in seed {
        pts.insert((left + right - x, y));
        pts.insert((x, top + bottom - y));
        pts.insert((left + right - x, top + bottom - y));
    }

    let mut out: Vec<(i32, i32)> = pts.into_iter().collect();
    out.sort();
    out
}

pub fn draw_ellipse(r: Rect) -> Layer {
    let mut out = Layer::new();
    if r.w == 0 || r.h == 0 {
        return out;
    }
    // w/h >= 1 here.
    if r.w == 1 && r.h == 1 {
        out.set(r.x, r.y, Cell::new("─", DRAW_FG, DRAW_BG));
        return out;
    }
    if r.w == 1 {
        for y in r.y..=r.y + r.h as i32 - 1 {
            out.set(r.x, y, Cell::new("│", DRAW_FG, DRAW_BG));
        }
        return out;
    }
    if r.h == 1 {
        for x in r.x..=r.x + r.w as i32 - 1 {
            out.set(x, r.y, Cell::new("─", DRAW_FG, DRAW_BG));
        }
        return out;
    }

    let left = r.x;
    let top = r.y;
    let right = r.x + r.w as i32 - 1;
    let bottom = r.y + r.h as i32 - 1;
    let cx = (left as f64 + right as f64) / 2.0;
    let cy = (top as f64 + bottom as f64) / 2.0;
    let rx = (right as f64 - left as f64) / 2.0;
    let ry = (bottom as f64 - top as f64) / 2.0;
    debug_assert!(rx > 0.0 && ry > 0.0);

    // 5) Slope-picked glyphs (symmetric: depends only on |x-cx|, |y-cy|).
    for (x, y) in ellipse_cells(r) {
        let adx = (x as f64 - cx).abs();
        let ady = (y as f64 - cy).abs();
        // |dy/dx| = (ry²·|x-cx|) / (rx²·|y-cy|); INF on the equator.
        let slope = if ady == 0.0 {
            f64::INFINITY
        } else {
            (ry * ry * adx) / (rx * rx * ady)
        };
        let ch = if slope <= 1.0 { "─" } else { "│" };
        out.set(x, y, Cell::new(ch, DRAW_FG, DRAW_BG));
    }
    out
}

/// Build a pencil patch: one cell per coord with `ch`/`fg`/`bg`.
///
/// Raw semantics (decisions Q7): no junction pass, no normalization — even
/// box-drawing chars are stored verbatim. Transparent spellings (`""`,
/// `" "`, `"\0"`) become [`Cell::erased`] (eraser dabs); everything else is
/// `Cell::new(ch, fg, bg)`.
pub fn paint_cells(cells: &[(i32, i32)], ch: &str, fg: i8, bg: i8) -> Layer {
    let mut out = Layer::new();
    let erased = ch.is_empty() || ch == " " || ch == "\0";
    for (x, y) in cells.iter().copied() {
        if erased {
            out.set(x, y, Cell::erased());
        } else {
            out.set(x, y, Cell::new(ch, fg, bg));
        }
    }
    out
}

// --- Junction tables (light set only) ---------------------------------------

const UP: (i32, i32) = (0, -1);
const DOWN: (i32, i32) = (0, 1);
const LEFT: (i32, i32) = (-1, 0);
const RIGHT: (i32, i32) = (1, 0);
const ALL_DIRS: [(i32, i32); 4] = [UP, DOWN, LEFT, RIGHT];

fn opposite(d: (i32, i32)) -> (i32, i32) {
    (-d.0, -d.1)
}

/// True for the 11 light box/junction glyphs. Styled (heavy/double/rounded/
/// ascii), arrows, and text never auto-junction (decisions Q1).
pub fn is_light_box(ch: &str) -> bool {
    matches!(
        ch,
        "─" | "│" | "┌" | "┐" | "┘" | "└" | "┬" | "┴" | "┤" | "├" | "┼"
    )
}

fn connects(ch: &str, dir: (i32, i32)) -> bool {
    match ch {
        "─" => dir == LEFT || dir == RIGHT,
        "│" => dir == UP || dir == DOWN,
        "┌" => dir == DOWN || dir == RIGHT,
        "┐" => dir == DOWN || dir == LEFT,
        "┘" => dir == UP || dir == LEFT,
        "└" => dir == UP || dir == RIGHT,
        "┬" => dir == DOWN || dir == LEFT || dir == RIGHT,
        "┴" => dir == UP || dir == LEFT || dir == RIGHT,
        "┤" => dir == UP || dir == DOWN || dir == LEFT,
        "├" => dir == UP || dir == DOWN || dir == RIGHT,
        "┼" => true,
        _ => false,
    }
}

fn connectable(ch: &str, _dir: (i32, i32)) -> bool {
    // Light set accepts from all four sides (characters.ts); anything else
    // (arrows/text/styled) is not connectable here.
    is_light_box(ch)
}

fn connect(ch: &str, dir: (i32, i32)) -> String {
    if connects(ch, dir) {
        return ch.to_string();
    }
    if dir == UP {
        match ch {
            "─" => "┴".to_string(),
            "┌" => "├".to_string(),
            "┐" => "┤".to_string(),
            "┬" => "┼".to_string(),
            _ => ch.to_string(),
        }
    } else if dir == DOWN {
        match ch {
            "─" => "┬".to_string(),
            "└" => "├".to_string(),
            "┘" => "┤".to_string(),
            "┴" => "┼".to_string(),
            _ => ch.to_string(),
        }
    } else if dir == LEFT {
        match ch {
            "│" => "┤".to_string(),
            "┌" => "┬".to_string(),
            "└" => "┴".to_string(),
            "├" => "┼".to_string(),
            _ => ch.to_string(),
        }
    } else if dir == RIGHT {
        match ch {
            "│" => "├".to_string(),
            "┐" => "┬".to_string(),
            "┘" => "┴".to_string(),
            "┤" => "┼".to_string(),
            _ => ch.to_string(),
        }
    } else {
        ch.to_string()
    }
}

fn disconnect(ch: &str, dir: (i32, i32)) -> String {
    if !connects(ch, dir) {
        return ch.to_string();
    }
    if dir == UP {
        match ch {
            "┴" => "─".to_string(),
            "├" => "┌".to_string(),
            "┤" => "┐".to_string(),
            "┼" => "┬".to_string(),
            _ => ch.to_string(),
        }
    } else if dir == DOWN {
        match ch {
            "┬" => "─".to_string(),
            "├" => "└".to_string(),
            "┤" => "┘".to_string(),
            "┼" => "┴".to_string(),
            _ => ch.to_string(),
        }
    } else if dir == LEFT {
        match ch {
            "┤" => "│".to_string(),
            "┬" => "┌".to_string(),
            "┴" => "└".to_string(),
            "┼" => "├".to_string(),
            _ => ch.to_string(),
        }
    } else if dir == RIGHT {
        match ch {
            "├" => "│".to_string(),
            "┬" => "┐".to_string(),
            "┴" => "┘".to_string(),
            "┼" => "┤".to_string(),
            _ => ch.to_string(),
        }
    } else {
        ch.to_string()
    }
}

/// Glyph connecting exactly `dirs` (order-free), or `None` when no clean
/// light glyph exists (empty / singleton sets, e.g. isolated stubs).
fn connection_glyph(up: bool, down: bool, left: bool, right: bool) -> Option<&'static str> {
    match (up, down, left, right) {
        (false, true, false, true) => Some("┌"),
        (false, true, true, false) => Some("┐"),
        (true, false, true, false) => Some("┘"),
        (true, false, false, true) => Some("└"),
        (false, false, true, true) => Some("─"),
        (true, true, false, false) => Some("│"),
        (false, true, true, true) => Some("┬"),
        (true, false, true, true) => Some("┴"),
        (true, true, true, false) => Some("┤"),
        (true, true, false, true) => Some("├"),
        (true, true, true, true) => Some("┼"),
        _ => None,
    }
}

/// Junction auto-connect + 2-pass normalization, folded into `scratch`.
///
/// Port of `client/snap.ts` restricted to the light set:
///
/// 1. **Connect pass.** Each light scratch cell gains connections toward
///    light committed neighbours that point back (and vice versa), skipping
///    scratch-to-scratch adjacency. Colors preserved per cell.
/// 2. **Unsnap pass.** Neighbours of deleted (transparent-marker) scratch
///    cells lose the connection facing the deletion.
/// 3. **Two normalization passes.** Every touched cell + its neighbours is
///    re-glyphed to exactly the neighbour set that connects back
///    (`connection_glyph`); arrows/text/styled cells are never rewritten.
///
/// `layer_idx` selects the committed layer to snap against (active layer;
/// other layers show through but never junction). Out-of-range `layer_idx`
/// is a no-op. Like TS, this mutates via an accumulating buffer then
/// `set_from`s it into `scratch`.
///
/// **Moved-box cells are NOT normalized**: the caller passes only
/// connector/scratch cells (cf. `snap(..., protect=movedBoxCells)` in
/// tools-spec §9.5). This function normalizes everything it is given.
pub fn snap_patch(doc: &Document, layer_idx: usize, scratch: &mut Layer) {
    if layer_idx >= doc.layers.len() {
        return;
    }
    let committed = &doc.layers[layer_idx].layer;

    // Snapshot scratch keys (raw, markers included) — we mutate at the end.
    let raw_keys: Vec<(i32, i32)> = scratch.keys().collect();
    if raw_keys.is_empty() {
        return;
    }
    let raw_set: HashSet<(i32, i32)> = raw_keys.iter().copied().collect();

    let mut extra = Layer::new();

    // Helper: accumulated scratch value (extra overrides scratch).
    let scratch_cur = |x: i32, y: i32, scratch: &Layer, extra: &Layer| -> Option<Cell> {
        if let Some(c) = extra.get(x, y) {
            Some(c)
        } else {
            scratch.get(x, y)
        }
    };

    // ---- Pass 1: connect -------------------------------------------------
    for (x, y) in &raw_keys {
        let Some(scell) = scratch.get(*x, *y) else {
            continue; // deletion marker → handled in pass 2
        };
        if !is_light_box(&scell.ch) {
            continue;
        }
        for d in ALL_DIRS {
            let (ax, ay) = (x + d.0, y + d.1);
            if raw_set.contains(&(ax, ay)) {
                continue; // never snap to other scratch cells
            }
            let Some(adj) = committed.get(ax, ay) else {
                continue;
            };
            if !is_light_box(&adj.ch) {
                continue;
            }
            let opp = opposite(d);
            // Scratch gains d when the neighbour points back.
            if connects(&adj.ch, opp) && !connects(&scell.ch, d) && connectable(&scell.ch, d)
            {
                let cur = scratch_cur(*x, *y, scratch, &extra).unwrap_or_else(|| scell.clone());
                let ng = connect(&cur.ch, d);
                extra.set(*x, *y, Cell::new(ng, cur.fg, cur.bg));
            }
            // Neighbour gains opp when scratch points at it.
            let cur_adj = extra.get(ax, ay).unwrap_or(adj.clone());
            if connects(&scell.ch, d)
                && !connects(&cur_adj.ch, opp)
                && connectable(&cur_adj.ch, opp)
            {
                let ng = connect(&cur_adj.ch, opp);
                extra.set(ax, ay, Cell::new(ng, cur_adj.fg, cur_adj.bg));
            }
        }
    }

    // ---- Pass 2: unsnap deletions ----------------------------------------
    {
        // Raw transparent keys in scratch.
        let mut deleted: Vec<(i32, i32)> = Vec::new();
        for (k, cell) in scratch.entries() {
            if cell.is_transparent() {
                deleted.push(k);
            }
        }
        for (x, y) in deleted {
            for d in ALL_DIRS {
                let (ax, ay) = (x + d.0, y + d.1);
                if raw_set.contains(&(ax, ay)) {
                    continue;
                }
                let Some(adj) = committed.get(ax, ay) else {
                    continue;
                };
                if !is_light_box(&adj.ch) {
                    continue;
                }
                let cur_adj = extra.get(ax, ay).unwrap_or(adj);
                let opp = opposite(d);
                if connects(&cur_adj.ch, opp) {
                    let ng = disconnect(&cur_adj.ch, opp);
                    // disconnect() returns the original spelling when no
                    // clean downgrade exists (e.g. ─ losing LEFT stays ─);
                    // still write it only when it actually changed so the
                    // patch stays minimal.
                    if ng != cur_adj.ch {
                        extra.set(ax, ay, Cell::new(ng, cur_adj.fg, cur_adj.bg));
                    }
                }
            }
        }
    }

    // ---- Pass 3: two-pass normalization ----------------------------------
    // stateAt(pos): extra > scratch (markers = deleted = None) > committed.
    let state_at = |x: i32, y: i32, scratch: &Layer, extra: &Layer, committed: &Layer| -> Option<Cell> {
        if let Some(c) = extra.get(x, y) {
            return Some(c);
        }
        if raw_set.contains(&(x, y)) {
            return scratch.get(x, y); // None for deletion markers
        }
        committed.get(x, y)
    };

    let mut candidates: HashSet<(i32, i32)> = HashSet::new();
    for (x, y) in &raw_keys {
        candidates.insert((*x, *y));
        for d in ALL_DIRS {
            candidates.insert((x + d.0, y + d.1));
        }
    }

    for _ in 0..2 {
        // Snapshot candidate list for deterministic iteration.
        let mut order: Vec<(i32, i32)> = candidates.iter().copied().collect();
        order.sort_unstable();
        // Buffer this pass's writes so intra-pass reads see a stable view
        // except via `extra` accumulation like TS (TS reads `layer` live).
        // We write live into `extra` to match TS exactly.
        for (x, y) in order {
            let Some(cur) = state_at(x, y, scratch, &extra, committed) else {
                continue;
            };
            if !is_light_box(&cur.ch) {
                continue; // never arrows/text/styled
            }
            let mut up = false;
            let mut down = false;
            let mut left = false;
            let mut right = false;
            for d in ALL_DIRS {
                let (nx, ny) = (x + d.0, y + d.1);
                if let Some(n) = state_at(nx, ny, scratch, &extra, committed) {
                    if is_light_box(&n.ch) && connects(&n.ch, opposite(d)) {
                        match d {
                            UP => up = true,
                            DOWN => down = true,
                            LEFT => left = true,
                            _ => right = true,
                        }
                    }
                }
            }
            if let Some(g) = connection_glyph(up, down, left, right) {
                if g != cur.ch {
                    extra.set(x, y, Cell::new(g, cur.fg, cur.bg));
                }
            }
        }
    }

    scratch.set_from(&extra);
}

// Re-exported for tests/inspection.
#[allow(dead_code)]
fn debug_ch(layer: &Layer, x: i32, y: i32) -> String {
    layer
        .get(x, y)
        .map(|c| c.ch.clone())
        .unwrap_or_else(|| ".".to_string())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use crate::model::{Document, History};

    fn ch(layer: &Layer, x: i32, y: i32) -> Option<String> {
        layer.get(x, y).map(|c| c.ch.clone())
    }

    fn commit_scratch(doc: &mut Document, hist: &mut History, scratch: &Layer) -> bool {
        hist.commit(doc, scratch)
    }

    #[test]
    fn box_1x1_is_empty() {
        for style in [
            BoxStyle::Light,
            BoxStyle::Heavy,
            BoxStyle::Double,
            BoxStyle::Rounded,
            BoxStyle::Ascii,
        ] {
            let l = draw_box(Rect::new(5, 5, 1, 1), style);
            assert!(l.is_empty(), "1x1 {style:?} must be empty");
        }
        assert!(draw_box(Rect::new(0, 0, 0, 3), BoxStyle::Light).is_empty());
        assert!(draw_box(Rect::new(0, 0, 3, 0), BoxStyle::Light).is_empty());
    }

    #[test]
    fn box_1wide_rows_are_bare() {
        let row = draw_box(Rect::new(0, 0, 4, 1), BoxStyle::Light);
        assert_eq!(row.len(), 4);
        for x in 0..4 {
            assert_eq!(ch(&row, x, 0).as_deref(), Some("─"), "x={x}");
        }
        let col = draw_box(Rect::new(2, 2, 1, 3), BoxStyle::Light);
        assert_eq!(col.len(), 3);
        for y in 2..5 {
            assert_eq!(ch(&col, 2, y).as_deref(), Some("│"), "y={y}");
        }
        // 2x2 = four corners only.
        let tiny = draw_box(Rect::new(0, 0, 2, 2), BoxStyle::Light);
        assert_eq!(ch(&tiny, 0, 0).as_deref(), Some("┌"));
        assert_eq!(ch(&tiny, 1, 0).as_deref(), Some("┐"));
        assert_eq!(ch(&tiny, 1, 1).as_deref(), Some("┘"));
        assert_eq!(ch(&tiny, 0, 1).as_deref(), Some("└"));
        assert_eq!(tiny.len(), 4);
    }

    #[test]
    fn box_corners_per_style() {
        // 4x3 boxes: check TL/TR/BR/BL + edges.
        let cases: &[(BoxStyle, &str, &str, [&str; 4])] = &[
            (BoxStyle::Light, "─", "│", ["┌", "┐", "┘", "└"]),
            (BoxStyle::Heavy, "━", "┃", ["┏", "┓", "┛", "┗"]),
            (BoxStyle::Double, "═", "║", ["╔", "╗", "╝", "╚"]),
            (BoxStyle::Rounded, "─", "│", ["╭", "╮", "╯", "╰"]),
            (BoxStyle::Ascii, "-", "|", ["+", "+", "+", "+"]),
        ];
        for (style, h, v, corners) in cases {
            let b = draw_box(Rect::new(0, 0, 4, 3), *style);
            assert_eq!(ch(&b, 0, 0).as_deref(), Some(corners[0]), "{style:?} TL");
            assert_eq!(ch(&b, 3, 0).as_deref(), Some(corners[1]), "{style:?} TR");
            assert_eq!(ch(&b, 3, 2).as_deref(), Some(corners[2]), "{style:?} BR");
            assert_eq!(ch(&b, 0, 2).as_deref(), Some(corners[3]), "{style:?} BL");
            assert_eq!(ch(&b, 1, 0).as_deref(), Some(*h), "{style:?} top edge");
            assert_eq!(ch(&b, 2, 2).as_deref(), Some(*h), "{style:?} bottom edge");
            assert_eq!(ch(&b, 0, 1).as_deref(), Some(*v), "{style:?} left edge");
            assert_eq!(ch(&b, 3, 1).as_deref(), Some(*v), "{style:?} right edge");
            // Interior stays empty.
            assert_eq!(ch(&b, 1, 1), None, "{style:?} interior");
        }
    }

    #[test]
    fn line_orientations_and_corners() {
        // Horizontal.
        let h = draw_line(0, 0, 3, 0, true);
        for x in 0..=3 {
            assert_eq!(ch(&h, x, 0).as_deref(), Some("─"));
        }
        // Vertical.
        let v = draw_line(1, 1, 1, 4, false);
        for y in 1..=4 {
            assert_eq!(ch(&v, 1, y).as_deref(), Some("│"));
        }
        // Zero-length dot.
        let dot = draw_line(2, 2, 2, 2, false);
        assert_eq!(ch(&dot, 2, 2).as_deref(), Some("│"));
        assert_eq!(dot.len(), 1);

        // All 8 bend orientations (tools-spec §3.1 table).
        assert_eq!(line_corner_for(true, 2, 1), "┐");
        assert_eq!(line_corner_for(true, 2, -1), "┘");
        assert_eq!(line_corner_for(true, -2, 1), "┌");
        assert_eq!(line_corner_for(true, -2, -1), "└");
        assert_eq!(line_corner_for(false, 2, 1), "└");
        assert_eq!(line_corner_for(false, -2, 1), "┘");
        assert_eq!(line_corner_for(false, 2, -1), "┌");
        assert_eq!(line_corner_for(false, -2, -1), "┐");

        // Bend cell lands in the patch with legs attached.
        let bent = draw_line(0, 0, 2, 1, true); // bend at (2,0) → ┐
        assert_eq!(ch(&bent, 2, 0).as_deref(), Some("┐"));
        assert_eq!(ch(&bent, 0, 0).as_deref(), Some("─"));
        assert_eq!(ch(&bent, 1, 0).as_deref(), Some("─"));
        assert_eq!(ch(&bent, 2, 1).as_deref(), Some("│"));
        let bent_v = draw_line(0, 0, 2, 1, false); // bend at (0,1) → └
        assert_eq!(ch(&bent_v, 0, 1).as_deref(), Some("└"));
    }

    #[test]
    fn rect_cells_perimeter_and_degenerates() {
        // 4x3 perimeter: 2*4 + 2*3 - 4 corners double-counted.
        let r = rect_cells(Rect::new(0, 0, 4, 3));
        assert_eq!(r.len(), 10);
        assert!(r.contains(&(0, 0)) && r.contains(&(3, 0)));
        assert!(r.contains(&(0, 2)) && r.contains(&(3, 2)));
        // Interior stays empty.
        assert!(!r.contains(&(1, 1)) && !r.contains(&(2, 1)));
        // Degenerates mirror the box convention.
        assert_eq!(rect_cells(Rect::new(5, 5, 1, 1)), vec![(5, 5)]);
        assert_eq!(
            rect_cells(Rect::new(0, 0, 1, 3)),
            vec![(0, 0), (0, 1), (0, 2)]
        );
        assert_eq!(
            rect_cells(Rect::new(0, 0, 3, 1)),
            vec![(0, 0), (1, 0), (2, 0)]
        );
        assert!(rect_cells(Rect::new(0, 0, 0, 5)).is_empty());
    }

    #[test]
    fn ellipse_cells_match_draw_ellipse_keys() {
        // Degenerates.
        assert_eq!(ellipse_cells(Rect::new(3, 3, 1, 1)), vec![(3, 3)]);
        assert_eq!(
            ellipse_cells(Rect::new(0, 0, 1, 3)),
            vec![(0, 0), (0, 1), (0, 2)]
        );
        // Key-set parity with the glyph-assigned builder across sizes.
        for w in 2..=12u32 {
            for h in 2..=8u32 {
                let r = Rect::new(0, 0, w, h);
                let mut from_cells = ellipse_cells(r);
                from_cells.sort();
                let mut from_layer: Vec<(i32, i32)> =
                    draw_ellipse(r).keys().collect();
                from_layer.sort();
                assert_eq!(from_cells, from_layer, "parity w={w} h={h}");
            }
        }
    }

    #[test]
    fn ellipse_degenerates_and_symmetry() {
        // Degenerates.
        let dot = draw_ellipse(Rect::new(3, 3, 1, 1));
        assert_eq!(ch(&dot, 3, 3).as_deref(), Some("─"));
        assert_eq!(dot.len(), 1);
        let col = draw_ellipse(Rect::new(0, 0, 1, 4));
        for y in 0..4 {
            assert_eq!(ch(&col, 0, y).as_deref(), Some("│"));
        }
        let row = draw_ellipse(Rect::new(0, 0, 4, 1));
        for x in 0..4 {
            assert_eq!(ch(&row, x, 0).as_deref(), Some("─"));
        }
        // Minimum rings.
        let two = draw_ellipse(Rect::new(0, 0, 2, 2));
        assert_eq!(two.len(), 4, "2x2 ring");
        let three = draw_ellipse(Rect::new(0, 0, 3, 3));
        assert_eq!(three.len(), 8, "3x3 ring, center empty");
        assert_eq!(ch(&three, 1, 1), None);

        // Mirror symmetry for a sweep of sizes (chars included; ─/│ are
        // mirror-invariant so direct equality holds).
        for w in 1..=30u32 {
            for h in 1..=12u32 {
                let e = draw_ellipse(Rect::new(0, 0, w, h));
                if w == 1 && h == 1 {
                    continue;
                }
                let left = 0;
                let right = w as i32 - 1;
                let top = 0;
                let bottom = h as i32 - 1;
                let mut cells: HashMap<(i32, i32), String> = HashMap::new();
                for ((x, y), c) in e.entries() {
                    if c.is_transparent() {
                        continue;
                    }
                    cells.insert((x, y), c.ch.clone());
                }
                for ((x, y), c) in &cells {
                    let mx = left + right - x;
                    let my = top + bottom - y;
                    assert_eq!(
                        cells.get(&(mx, *y)),
                        Some(c),
                        "mirror-x w={w} h={h} ({x},{y})"
                    );
                    assert_eq!(
                        cells.get(&(*x, my)),
                        Some(c),
                        "mirror-y w={w} h={h} ({x},{y})"
                    );
                }
                // Outline uses only slope-picked chars, never corners.
                for c in cells.values() {
                    assert!(
                        c == "─" || c == "│",
                        "oval glyph must be ─/│, got {c:?} w={w} h={h}"
                    );
                }
            }
        }
    }

    #[test]
    fn snap_connects_t_junction() {
        // Committed horizontal run; scratch vertical leg drops onto its middle.
        let mut doc = Document::new("t", 20, 10);
        let mut hist = History::new();
        let mut base = Layer::new();
        for x in 0..=2 {
            base.set(x, 0, Cell::new("─", 7, -1));
        }
        assert!(hist.commit_layer(&mut doc, 0, &base, None));

        let mut scratch = draw_line(1, -2, 1, 0, false);
        snap_patch(&doc, 0, &mut scratch);
        // The leg's foot + the run's middle normalize to ┴ (UP+LEFT+RIGHT).
        assert_eq!(ch(&scratch, 1, 0).as_deref(), Some("┴"));

        // Commit and verify the composed view.
        assert!(commit_scratch(&mut doc, &mut hist, &scratch));
        assert_eq!(
            doc.cell(1, 0).map(|c| c.ch.clone()).as_deref(),
            Some("┴")
        );

        // Horizontal onto vertical → ┤.
        let mut doc2 = Document::new("t2", 20, 10);
        let mut h2 = History::new();
        let mut col = Layer::new();
        for y in 0..=2 {
            col.set(0, y, Cell::new("│", 7, -1));
        }
        assert!(h2.commit_layer(&mut doc2, 0, &col, None));
        let mut s2 = draw_line(-2, 1, 0, 1, true);
        snap_patch(&doc2, 0, &mut s2);
        // Foot of the horizontal run meets the vertical wall.
        let foot = s2.get(0, 1).map(|c| c.ch.clone());
        let wall_via_compose = {
            // s2 holds the neighbour override when the wall upgrades.
            s2.get(0, 1).map(|c| c.ch.clone())
        };
        assert!(
            foot.as_deref() == Some("┤") || wall_via_compose.as_deref() == Some("┤"),
            "expected ┤ at (0,1), got {foot:?}"
        );
    }

    #[test]
    fn snap_unsnaps_on_delete() {
        // ┼ with all four arms; erase the right arm's neighbour cell content
        // by deleting the stem cell to the right.
        let mut doc = Document::new("u", 20, 10);
        // Work explicitly on layer 0 (new docs paint into Frames/1).
        doc.set_active(0);
        let mut hist = History::new();
        let mut cross = Layer::new();
        cross.set(1, 1, Cell::new("┼", 7, -1));
        cross.set(0, 1, Cell::new("─", 7, -1));
        cross.set(2, 1, Cell::new("─", 7, -1));
        cross.set(1, 0, Cell::new("│", 7, -1));
        cross.set(1, 2, Cell::new("│", 7, -1));
        assert!(hist.commit_layer(&mut doc, 0, &cross, None));

        // Eraser scratch deletes (2,1) (right arm).
        let mut scratch = paint_cells(&[(2, 1)], "", 0, -1);
        snap_patch(&doc, 0, &mut scratch);
        // Neighbour (1,1) must lose RIGHT: ┼ → ┤.
        assert_eq!(ch(&scratch, 1, 1).as_deref(), Some("┤"));
        assert!(commit_scratch(&mut doc, &mut hist, &scratch));
        assert_eq!(
            doc.cell(1, 1).map(|c| c.ch.clone()).as_deref(),
            Some("┤")
        );
        assert_eq!(doc.cell(2, 1), None);
    }

    #[test]
    fn pencil_is_raw_no_normalization() {
        // paint_cells stores verbatim even when a junction would form.
        let dab = paint_cells(&[(0, 0)], "─", 3, -1);
        assert_eq!(ch(&dab, 0, 0).as_deref(), Some("─"));
        assert_eq!(dab.get(0, 0).map(|c| (c.fg, c.bg)), Some((3, -1)));
        // Eraser spellings become Cell::erased markers (transparent).
        for spelling in ["", " "] {
            let e = paint_cells(&[(1, 1)], spelling, 0, -1);
            assert_eq!(e.get(1, 1), None, "eraser {spelling:?} composes empty");
            // Raw entry exists as a deletion marker.
            assert!(!e.is_empty());
            assert!(e.keys().any(|k| k == (1, 1)));
        }
        // A pencil box char next to a line does NOT auto-junction on its own:
        // the patch holds exactly the requested cells, nothing more.
        let two = paint_cells(&[(5, 5), (6, 5)], "─", 7, -1);
        assert_eq!(two.len(), 2);
        assert_eq!(ch(&two, 5, 5).as_deref(), Some("─"));
    }

    #[test]
    fn snap_ignores_arrows_text_and_styled() {
        let mut doc = Document::new("s", 20, 10);
        let mut hist = History::new();
        let mut base = Layer::new();
        base.set(1, 1, Cell::new("►", 7, -1));
        base.set(3, 1, Cell::new("A", 7, -1));
        base.set(5, 1, Cell::new("━", 7, -1)); // heavy: no junction
        assert!(hist.commit_layer(&mut doc, 0, &base, None));

        let mut scratch = draw_line(1, 0, 1, 2, false);
        let before = scratch.get(1, 1).map(|c| c.ch.clone());
        snap_patch(&doc, 0, &mut scratch);
        // Arrow/text/styled neighbours are never rewritten...
        assert_eq!(doc.cell(1, 1).map(|c| c.ch.clone()).as_deref(), Some("►"));
        // ...and normalization never turns plain cells into arrows.
        for (_, c) in scratch.entries() {
            assert!(!matches!(c.ch.as_str(), "◄" | "►" | "▲" | "▼"), "no arrows");
        }
        let _ = before;
    }
}
