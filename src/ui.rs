//! Ratatui layout per plan.md §4.1: toolbar / tools rail / canvas / palette /
//! layers row / status bar. Never crashes on small terminals (min-size guard).
//!
//! All mouse hit-testing goes through [`compute_layout`] + the `*_hit`
//! helpers so `main.rs` maps events with the exact rects used for rendering.

use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
};

use crate::app::{App, COLOR_ROWS, PALETTE_TABS, Tool, palette_chars};
use omaframe::model;
use omaframe::theme;

// ---------------------------------------------------------------------------
// Layout
// ---------------------------------------------------------------------------

pub const MIN_W: u16 = 60;
pub const MIN_H: u16 = 16;
const LEFT_W: u16 = 14;
const PALETTE_W: u16 = 26;
const TOOLS_N: u16 = 11;
const MENU_H: u16 = 3;

/// Menu bar actions (top border of the center column).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum MenuAction {
    New,
    Save,
    Load,
}

/// Screen rects. `tools`, `colors`, `canvas`, `palette_*`, `layers` are
/// inner interaction rects (inside their panel borders); render functions
/// draw the surrounding box in the rows/cols around them.
#[derive(Clone, Copy, Debug, Default)]
pub struct LayoutAreas {
    pub menu_top: Rect,
    pub menu_new: Rect,
    pub menu_save: Rect,
    pub menu_load: Rect,
    pub tools: Rect,
    pub colors: Rect,
    pub canvas: Rect,
    pub palette_tabs: Rect,
    pub palette_grid: Rect,
    pub layers: Rect,
    pub status: Rect,
    pub too_small: bool,
}

/// Pure layout: identical rects for rendering and mouse mapping.
///
/// ```text
/// row 0: ┌─ Tools ─┐ ┌─ New ─┬─ Save ─┬─ Load ─┐ ┌─ Palette ─┐
/// row 1: │ tool    │ │ File: …        │ │ tabs    │
/// row 2: │ …       │ └───────────────┘ │ …       │
/// ```
/// Panels own their border cells; `*_inner` rects below are the content.
pub fn compute_layout(area: Rect) -> LayoutAreas {
    if area.width < MIN_W || area.height < MIN_H {
        return LayoutAreas {
            too_small: true,
            ..Default::default()
        };
    }
    let v = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(TOOLS_N + 3), // main (tools box needs 13+ rows)
            Constraint::Length(1),        // layers row
            Constraint::Length(1),        // status bar
        ])
        .split(area);
    let layers = v[1];
    let status = v[2];
    let h = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Length(LEFT_W),
            Constraint::Min(10),
            Constraint::Length(PALETTE_W),
        ])
        .split(v[0]);
    let left_outer = h[0];
    let center_outer = h[1];
    let palette_outer = h[2];

    // Left column: tools box (fixed) + colors box (rest).
    let tools_outer = Rect::new(
        left_outer.x,
        left_outer.y,
        left_outer.width,
        (TOOLS_N + 2).min(left_outer.height),
    );
    let tools = inset1(tools_outer);
    let colors_outer = Rect::new(
        left_outer.x,
        tools_outer.y + tools_outer.height,
        left_outer.width,
        left_outer.height.saturating_sub(tools_outer.height),
    );
    let colors = inset1(colors_outer);

    // Center column: menu block (fixed) + canvas box (rest).
    let menu_outer = Rect::new(
        center_outer.x,
        center_outer.y,
        center_outer.width,
        MENU_H.min(center_outer.height),
    );
    let menu_top = Rect::new(
        menu_outer.x + 1,
        menu_outer.y,
        menu_outer.width.saturating_sub(2),
        1,
    );
    // Menu items live in the top border (see `menu_top_line`): fixed offsets.
    let menu_new = Rect::new(menu_outer.x + 3, menu_outer.y, 3, 1);
    let menu_save = Rect::new(menu_outer.x + 11, menu_outer.y, 4, 1);
    let menu_load = Rect::new(menu_outer.x + 20, menu_outer.y, 4, 1);
    let canvas_outer = Rect::new(
        center_outer.x,
        menu_outer.y + menu_outer.height,
        center_outer.width,
        center_outer.height.saturating_sub(menu_outer.height),
    );
    let canvas = inset1(canvas_outer);

    // Palette column: one box; tabs + divider + grid inside.
    let palette_tabs = Rect::new(
        palette_outer.x + 1,
        palette_outer.y + 1,
        palette_outer.width.saturating_sub(2),
        7,
    );
    let palette_grid = Rect::new(
        palette_outer.x + 1,
        palette_tabs.y + palette_tabs.height + 1, // +1 skips the divider row
        palette_outer.width.saturating_sub(2),
        palette_outer
            .height
            .saturating_sub(1 + 7 + 1 + 1), // top, tabs, divider, bottom
    );

    LayoutAreas {
        menu_top,
        menu_new,
        menu_save,
        menu_load,
        tools,
        colors,
        canvas,
        palette_tabs,
        palette_grid,
        layers,
        status,
        too_small: false,
    }
}

/// Shrink a panel outer rect by its 1-cell border.
fn inset1(r: Rect) -> Rect {
    Rect::new(
        r.x.saturating_add(1),
        r.y.saturating_add(1),
        r.width.saturating_sub(2),
        r.height.saturating_sub(2),
    )
}

// ---------------------------------------------------------------------------
// Hit-testing (screen → action)
// ---------------------------------------------------------------------------

fn contains(r: Rect, col: u16, row: u16) -> bool {
    col >= r.x && row >= r.y && col < r.x + r.width && row < r.y + r.height
}

pub fn hit_tool(areas: &LayoutAreas, col: u16, row: u16) -> Option<Tool> {
    if !contains(areas.tools, col, row) {
        return None;
    }
    let idx = (row - areas.tools.y) as usize;
    Tool::ALL.get(idx).copied()
}

pub fn hit_palette_tab(areas: &LayoutAreas, col: u16, row: u16) -> Option<usize> {
    if !contains(areas.palette_tabs, col, row) {
        return None;
    }
    let idx = (row - areas.palette_tabs.y) as usize;
    if idx < PALETTE_TABS.len() {
        Some(idx)
    } else {
        None
    }
}

/// Grid columns for the active tab (single-char tabs). Widgets tab is a
/// single-column name list (column 0 only).
pub fn palette_grid_cols(areas: &LayoutAreas, tab: usize) -> usize {
    if tab == 6 {
        1
    } else {
        ((areas.palette_grid.width / 2).max(1)) as usize
    }
}

/// Map a click in the palette grid to a char index into `palette_chars(tab)`.
pub fn hit_palette_grid(
    areas: &LayoutAreas,
    app: &App,
    col: u16,
    row: u16,
) -> Option<usize> {
    if !contains(areas.palette_grid, col, row) {
        return None;
    }
    let tab = app.palette_tab;
    let cols = palette_grid_cols(areas, tab);
    let items = palette_chars(tab);
    if tab == 6 {
        let idx = app.palette_scroll + (row - areas.palette_grid.y) as usize;
        return if idx < items.len() { Some(idx) } else { None };
    }
    let rel_row = (row - areas.palette_grid.y) as usize;
    let rel_col = ((col - areas.palette_grid.x) / 2) as usize;
    if rel_col >= cols {
        return None;
    }
    let idx = (app.palette_scroll + rel_row) * cols + rel_col;
    if idx < items.len() {
        Some(idx)
    } else {
        None
    }
}

/// Layers row: `Layers:` prefix (7 cols) then fixed 16-wide segments.
/// Returns `(layer_idx, toggle_eye)` — clicks on the segment's first 2
/// columns toggle visibility, the rest selects.
pub const LAYER_PREFIX_W: u16 = 7;
pub const LAYER_SEG_W: u16 = 16;

pub fn hit_layer(areas: &LayoutAreas, n_layers: usize, col: u16, row: u16) -> Option<(usize, bool)> {
    if !contains(areas.layers, col, row) || col < areas.layers.x + LAYER_PREFIX_W {
        return None;
    }
    let rel = col - areas.layers.x - LAYER_PREFIX_W;
    let idx = (rel / LAYER_SEG_W) as usize;
    if idx >= n_layers {
        return None;
    }
    let eye = (rel % LAYER_SEG_W) < 2;
    Some((idx, eye))
}

/// Map a click in the menu top border to New/Save/Load.
pub fn hit_menu(areas: &LayoutAreas, col: u16, row: u16) -> Option<MenuAction> {
    if contains(areas.menu_new, col, row) {
        Some(MenuAction::New)
    } else if contains(areas.menu_save, col, row) {
        Some(MenuAction::Save)
    } else if contains(areas.menu_load, col, row) {
        Some(MenuAction::Load)
    } else {
        None
    }
}

/// Colors panel rows (see `app::COLOR_ROWS`): row 0 is
/// transparent-background, rows 1–16 are ANSI slots 0–15 (offset by
/// `colors_scroll`). Returns the row (0–17).
pub const COLORS_N: usize = COLOR_ROWS;

pub fn hit_colors(
    areas: &LayoutAreas,
    app: &App,
    col: u16,
    row: u16,
) -> Option<usize> {
    if !contains(areas.colors, col, row) {
        return None;
    }
    let idx = app.colors_scroll + (row - areas.colors.y) as usize;
    if idx < COLORS_N {
        Some(idx)
    } else {
        None
    }
}

pub fn over_colors(areas: &LayoutAreas, col: u16, row: u16) -> bool {
    contains(areas.colors, col, row)
}

/// Map a canvas click to a document cell (viewport-adjusted).
pub fn hit_canvas(areas: &LayoutAreas, app: &App, col: u16, row: u16) -> Option<(i32, i32)> {
    if !contains(areas.canvas, col, row) {
        return None;
    }
    let dx = (col - areas.canvas.x) as i32;
    let dy = (row - areas.canvas.y) as i32;
    Some((app.viewport.0 + dx, app.viewport.1 + dy))
}

pub fn over_palette_grid(areas: &LayoutAreas, col: u16, row: u16) -> bool {
    contains(areas.palette_grid, col, row)
}

pub fn over_canvas(areas: &LayoutAreas, col: u16, row: u16) -> bool {
    contains(areas.canvas, col, row)
}

// ---------------------------------------------------------------------------
// Rendering
// ---------------------------------------------------------------------------

fn chrome_title_style(t: &theme::Theme) -> Style {
    Style::default().fg(t.accent).add_modifier(Modifier::BOLD)
}

// ---------------------------------------------------------------------------
// Panel frames (wireframe chrome: titled box borders around every panel)
// ---------------------------------------------------------------------------

/// Top border with an embedded title: `┌─ {title} ───┐`, exactly `w + 2`
/// cells wide. Title is ASCII-short; truncation keeps the width exact.
fn top_with_title(w: usize, title: &str) -> String {
    let t = truncate_to(title, w.saturating_sub(4));
    let mut s = format!("┌─ {t} ");
    while s.chars().count() < w + 1 {
        s.push('─');
    }
    s.push('┐');
    s
}

/// Bottom border: `└──…──┘`, exactly `w + 2` cells.
fn bottom_border(w: usize) -> String {
    format!("└{}┘", "─".repeat(w))
}

/// Draw the `│` side borders for the rows strictly between the top and
/// bottom border of `outer`. Content renders separately into the inner
/// rect, so no width padding is ever needed (wide glyphs stay safe).
fn panel_sides(f: &mut Frame, outer: Rect, t: &theme::Theme) {
    if outer.width < 2 || outer.height < 3 {
        return;
    }
    let style = Style::default().fg(t.dim);
    for y in (outer.y + 1)..(outer.y + outer.height - 1) {
        f.render_widget(
            Paragraph::new("│").style(style),
            Rect::new(outer.x, y, 1, 1),
        );
        f.render_widget(
            Paragraph::new("│").style(style),
            Rect::new(outer.x + outer.width - 1, y, 1, 1),
        );
    }
}

/// Outer rect around an inner content rect (inverse of layout `inset1`).
fn outer_of(inner: Rect) -> Rect {
    Rect::new(
        inner.x.saturating_sub(1),
        inner.y.saturating_sub(1),
        inner.width + 2,
        inner.height + 2,
    )
}

fn render_top_row(f: &mut Frame, outer: Rect, line: Line, t: &theme::Theme) {
    let _ = t;
    f.render_widget(
        Paragraph::new(line),
        Rect::new(outer.x, outer.y, outer.width, 1),
    );
}

fn render_bottom_row(f: &mut Frame, outer: Rect, w: usize, t: &theme::Theme) {
    f.render_widget(
        Paragraph::new(bottom_border(w)).style(Style::default().fg(t.dim)),
        Rect::new(outer.x, outer.y + outer.height - 1, outer.width, 1),
    );
}

fn render_menu(f: &mut Frame, areas: &LayoutAreas, app: &App, t: &theme::Theme) {
    let outer = outer_of(areas.menu_top);
    let w = areas.menu_top.width as usize;
    let dim = Style::default().fg(t.dim);
    let item = Style::default().fg(t.fg).add_modifier(Modifier::BOLD);
    // Item labels sit at fixed offsets (see compute_layout): the spans
    // below must reproduce them exactly for click mapping.
    let mut spans = vec![
        Span::styled("┌─ ", dim),
        Span::styled("New", item),
        Span::styled(" ─┬─ ", dim),
        Span::styled("Save", item),
        Span::styled(" ─┬─ ", dim),
        Span::styled("Load", item),
        Span::styled(" ─", dim),
    ];
    let used: usize = "┌─ New ─┬─ Save ─┬─ Load ─".chars().count();
    if w + 2 > used {
        spans.push(Span::styled(
            "─".repeat(w + 2 - used - 1) + "┐",
            dim,
        ));
    } else {
        spans.push(Span::styled("┐", dim));
    }
    render_top_row(f, outer, Line::from(spans), t);
    // File row: full path with dirty star, `~`-shortened.
    let file_line = Line::from(vec![
        Span::styled("│ File: ", dim),
        Span::styled(
            truncate_to(&app.full_path_label(), w.saturating_sub(8)),
            Style::default().fg(t.fg),
        ),
    ]);
    f.render_widget(
        Paragraph::new(file_line),
        Rect::new(outer.x, outer.y + 1, outer.width, 1),
    );
    panel_sides(f, outer, t);
    render_bottom_row(f, outer, w, t);
}

fn truncate_to(s: &str, w: usize) -> String {
    if w == 0 {
        return String::new();
    }
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i >= w {
            break;
        }
        out.push(c);
    }
    out
}

fn render_tools(f: &mut Frame, areas: &LayoutAreas, app: &App, t: &theme::Theme) {
    let outer = outer_of(areas.tools);
    let w = areas.tools.width as usize;
    render_top_row(
        f,
        outer,
        Line::from(Span::styled(
            top_with_title(w, "Tools"),
            Style::default().fg(t.dim),
        )),
        t,
    );
    let mut lines: Vec<Line> = Vec::new();
    for tool in Tool::ALL {
        let active = tool == app.tool;
        let marker = if active { ">" } else { " " };
        let style = if active {
            Style::default().bg(t.accent).fg(t.bg).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(t.fg)
        };
        lines.push(Line::from(vec![Span::styled(
            format!("{}{} {}", marker, tool.shortcut(), tool.label()),
            style,
        )]));
    }
    // Box-state line under the 11 tools when space allows.
    f.render_widget(Paragraph::new(lines), areas.tools);
    let outer = outer_of(areas.tools);
    panel_sides(f, outer, t);
    render_bottom_row(f, outer, areas.tools.width as usize, t);
}

fn is_guard(doc: &omaframe::model::Document, scratch: &omaframe::model::Layer, x: i32, y: i32) -> bool {
    if doc.is_continuation(x, y) {
        return true;
    }
    // Scratch-side wide anchors (pencil typing CJK before commit).
    if x > 0 {
        if let Some(c) = scratch.get(x - 1, y) {
            if c.width() >= 2 {
                return true;
            }
        }
    }
    false
}

fn render_canvas(f: &mut Frame, areas: &LayoutAreas, app: &App, t: &theme::Theme) {
    let cw = areas.canvas.width as i32;
    let ch = areas.canvas.height as i32;
    if cw <= 0 || ch <= 0 {
        return;
    }
    // Boxed canvas with the viewport origin in the top border (infinite
    // canvas: the origin drifts as you scroll).
    let outer = outer_of(areas.canvas);
    let w = areas.canvas.width as usize;
    let origin = format!("{},{}", app.viewport.0, app.viewport.1);
    render_top_row(
        f,
        outer,
        Line::from(Span::styled(
            top_with_title(w, &origin),
            Style::default().fg(t.dim),
        )),
        t,
    );
    // Empty cells preview on the same surface as transparent-styled cells.
    let empty_style = theme::cell_style(7, -1, t, app.preview_dark);
    let cursor_style = Style::default().bg(t.highlight).fg(t.bg).add_modifier(Modifier::BOLD);
    let sel = app.history.selection();
    let mut lines: Vec<Line> = Vec::with_capacity(ch as usize);
    for dy in 0..ch {
        let doc_y = app.viewport.1 + dy;
        let mut spans: Vec<Span> = Vec::new();
        for dx in 0..cw {
            let doc_x = app.viewport.0 + dx;
            if is_guard(&app.doc, &app.scratch, doc_x, doc_y) {
                continue; // wide anchor already occupies both terminal cols
            }
            let on_cursor = (doc_x, doc_y) == app.cursor;
            let in_sel = sel.is_some_and(|r| r.contains(doc_x, doc_y));
            // Eraser preview: erased markers compose as transparent (the old
            // content shows through), so flag them for inversion — otherwise
            // the drag rectangle is invisible until commit. (`get` filters
            // markers out, hence the raw read.)
            let erased_preview = app
                .scratch
                .get_raw(doc_x, doc_y)
                .is_some_and(|c| c.is_transparent());
            let composed = app.doc.compose(&app.scratch, doc_x, doc_y);
            let mut st = match &composed {
                Some(c) => theme::cell_style(c.fg, c.bg, t, app.preview_dark),
                None => empty_style,
            };
            let glyph = match &composed {
                Some(c) => c.ch.clone(),
                None => " ".to_string(),
            };
            if on_cursor {
                st = cursor_style;
            } else if in_sel || erased_preview {
                // Rubber-band selections (select tool, eraser rect) render
                // inverted: swap fg/bg at the terminal level, glyph kept.
                st = st.add_modifier(Modifier::REVERSED);
            }
            spans.push(Span::styled(glyph, st));
        }
        lines.push(Line::from(spans));
    }
    f.render_widget(Paragraph::new(lines), areas.canvas);
    panel_sides(f, outer, t);
    render_bottom_row(f, outer, w, t);
}

fn render_palette(f: &mut Frame, areas: &LayoutAreas, app: &App, t: &theme::Theme) {
    // Boxed panel: title border, tabs, divider, then the char grid (the old
    // fg/bg pots moved to the Colors panel).
    let outer = Rect::new(
        areas.palette_tabs.x.saturating_sub(1),
        areas.palette_tabs.y.saturating_sub(1),
        areas.palette_tabs.width + 2,
        areas.palette_tabs.height
            + 1 // top border
            + 1 // divider
            + areas.palette_grid.height
            + 1, // bottom border
    );
    let w = outer.width.saturating_sub(2) as usize;
    render_top_row(
        f,
        outer,
        Line::from(Span::styled(
            top_with_title(w, "Palette"),
            Style::default().fg(t.dim),
        )),
        t,
    );
    // Tabs.
    let mut tab_lines = Vec::new();
    for (i, name) in PALETTE_TABS.iter().enumerate() {
        let active = i == app.palette_tab;
        let style = if active {
            Style::default().bg(t.accent).fg(t.bg).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(t.fg)
        };
        tab_lines.push(Line::from(vec![Span::styled(
            format!("{} {}", if active { ">" } else { " " }, name),
            style,
        )]));
    }
    f.render_widget(Paragraph::new(tab_lines), areas.palette_tabs);
    // Grid.
    let items = palette_chars(app.palette_tab);
    let cols = palette_grid_cols(areas, app.palette_tab);
    let gh = areas.palette_grid.height as usize;
    let mut lines: Vec<Line> = Vec::new();
    if app.palette_tab == 6 {
        for r in 0..gh {
            let idx = app.palette_scroll + r;
            if idx >= items.len() {
                lines.push(Line::from(""));
                continue;
            }
            let style = Style::default().fg(t.fg);
            lines.push(Line::from(vec![Span::styled(items[idx].clone(), style)]));
        }
    } else {
        for r in 0..gh {
            let mut spans: Vec<Span> = Vec::new();
            for c in 0..cols {
                let idx = (app.palette_scroll + r) * cols + c;
                if idx >= items.len() {
                    break;
                }
                let active = items[idx] == app.ch;
                let style = if active {
                    Style::default().bg(t.accent).fg(t.bg).add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(t.fg)
                };
                // Each char in a 2-wide cell so clicks map 1:1.
                spans.push(Span::styled(format!("{:<2}", items[idx]), style));
            }
            lines.push(Line::from(spans));
        }
    }
    f.render_widget(Paragraph::new(lines), areas.palette_grid);
    panel_sides(f, outer, t);
    render_bottom_row(f, outer, w, t);
    // Divider between tabs and grid last so its ├┤ ends win over the sides.
    f.render_widget(
        Paragraph::new(Line::from(Span::styled(
            format!("├{}┤", "─".repeat(w)),
            Style::default().fg(t.dim),
        ))),
        Rect::new(outer.x, areas.palette_tabs.y + 7, outer.width, 1),
    );
}

/// Colors panel: 17 clickable rows — transparent background, then ANSI
/// slots 0–15 as wide swatches. Left-click sets fg, right-click sets bg.
/// Markers: `>` fg pot, `*` bg pot, `#` both.
fn render_colors(f: &mut Frame, areas: &LayoutAreas, app: &App, t: &theme::Theme) {
    let outer = outer_of(areas.colors);
    let w = areas.colors.width as usize;
    if outer.height < 3 || w == 0 {
        return;
    }
    render_top_row(
        f,
        outer,
        Line::from(Span::styled(
            top_with_title(w, "Colors"),
            Style::default().fg(t.dim),
        )),
        t,
    );
    let gh = areas.colors.height as usize;
    let mut lines: Vec<Line> = Vec::new();
    for r in 0..gh {
        let idx = app.colors_scroll + r;
        if idx >= COLORS_N {
            lines.push(Line::from(""));
            continue;
        }
        if idx == 0 {
            let mark = if app.bg < 0 { "*" } else { " " };
            lines.push(Line::from(vec![
                Span::styled(mark.to_string(), Style::default().fg(t.fg)),
                Span::styled(" –none– ", Style::default().fg(t.dim)),
            ]));
            continue;
        }
        let slot = (idx - 1) as i8;
        let mark = match (app.fg == slot, app.bg == slot) {
            (true, true) => "#",
            (true, false) => ">",
            (false, true) => "*",
            (false, false) => " ",
        };
        lines.push(Line::from(vec![
            Span::styled(mark.to_string(), Style::default().fg(t.fg)),
            Span::styled(
                "████████",
                Style::default().fg(t.ansi[slot as usize]).bg(t.bg),
            ),
        ]));
    }
    f.render_widget(Paragraph::new(lines), areas.colors);
    panel_sides(f, outer, t);
    render_bottom_row(f, outer, w, t);
}

fn render_layers(f: &mut Frame, areas: &LayoutAreas, app: &App, t: &theme::Theme) {
    let mut spans = vec![Span::styled("Layers:", chrome_title_style(t))];
    for (i, nl) in app.doc.layers.iter().enumerate() {
        let eye = if nl.visible { "▣" } else { "▢" };
        let active = i == app.doc.active;
        let seg = format!("{} {:<10}{} ", eye, truncate_to(&nl.name, 10), if active { "*" } else { " " });
        let seg = truncate_to(&seg, LAYER_SEG_W as usize);
        let padded = format!("{:<width$}", seg, width = LAYER_SEG_W as usize);
        let style = if active {
            Style::default().bg(t.accent).fg(t.bg).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(t.fg)
        };
        spans.push(Span::styled(padded, style));
    }
    let w = areas.layers.width as usize;
    let line = Line::from(spans);
    let text = line.to_string();
    let _ = (w, text);
    f.render_widget(Paragraph::new(line), areas.layers);
}

fn render_status(f: &mut Frame, areas: &LayoutAreas, app: &App, t: &theme::Theme) {
    // Inline path prompt (menu Load / Save As) takes over the status bar.
    if let Some(kind) = app.prompt {
        let msg = format!(
            "{}: {}█   (Enter ok · Esc cancel)",
            kind.caption(),
            app.prompt_buf
        );
        f.render_widget(
            Paragraph::new(truncate_to(&msg, areas.status.width as usize))
                .style(Style::default().bg(t.highlight).fg(t.fg)),
            areas.status,
        );
        return;
    }
    let size = match model::content_bounds(&app.doc) {
        Some(b) => format!("{}x{}", b.w, b.h),
        None => "empty".to_string(),
    };
    let bg_label = if app.bg < 0 { "-".to_string() } else { app.bg.to_string() };
    let sel = match app.history.selection() {
        Some(r) => format!(" sel {}x{}@{},{}", r.w, r.h, r.x, r.y),
        None => String::new(),
    };
    let msg = format!(
        "cell {},{} · {} · fg {} bg {} · {} · {}{} · [{}] {} · {}",
        app.cursor.0,
        app.cursor.1,
        app.active_layer_name(),
        app.fg,
        bg_label,
        app.file_label(),
        size,
        sel,
        app.tool.label(),
        app.ch,
        app.status_msg,
    );
    let msg = truncate_to(&msg, areas.status.width as usize);
    f.render_widget(
        Paragraph::new(msg).style(Style::default().bg(t.bg).fg(t.fg)),
        areas.status,
    );
}

/// Top-level render. Never panics on small terminals: below the minimum it
/// shows a one-line guard message instead of the full layout.
pub fn render(f: &mut Frame, app: &mut App, t: &theme::Theme) {
    let area = f.area();
    let areas = compute_layout(area);
    if areas.too_small {
        let msg = format!(
            "terminal too small: need {}x{}, have {}x{} — enlarge to draw",
            MIN_W, MIN_H, area.width, area.height
        );
        f.render_widget(Paragraph::new(msg), area);
        return;
    }
    render_menu(f, &areas, app, t);
    render_tools(f, &areas, app, t);
    render_colors(f, &areas, app, t);
    render_canvas(f, &areas, app, t);
    render_palette(f, &areas, app, t);
    render_layers(f, &areas, app, t);
    render_status(f, &areas, app, t);
}

#[cfg(test)]
mod tests {
    use super::*;
    use omaframe::model::{Cell, Document};
    use ratatui::{Terminal, backend::TestBackend};

    fn harness() -> (App, theme::Theme) {
        let mut doc = Document::new("t", 80, 24);
        doc.active_layer_mut()
            .set(2, 1, Cell::new("x", 7, -1));
        (App::new(doc, None), theme::defaults())
    }

    fn render_to_buf(app: &mut App, t: &theme::Theme) -> ratatui::buffer::Buffer {
        let backend = TestBackend::new(80, 24);
        let mut term = Terminal::new(backend).unwrap();
        term.draw(|f| render(f, app, t)).unwrap();
        term.backend().buffer().clone()
    }

    /// Screen position of a document cell with the default viewport.
    fn screen_of(doc_x: i32, doc_y: i32) -> (u16, u16) {
        let areas = compute_layout(ratatui::layout::Rect::new(0, 0, 80, 24));
        (
            areas.canvas.x + doc_x as u16,
            areas.canvas.y + doc_y as u16,
        )
    }

    #[test]
    fn wireframe_layout_menu_panels_canvas() {
        // 80x24: the wireframe arrangement — menu block over the canvas,
        // tools + colors stacked left, palette right.
        let a = compute_layout(ratatui::layout::Rect::new(0, 0, 80, 24));
        assert!(!a.too_small);
        // Menu items sit inside the top border at fixed offsets.
        assert_eq!((a.menu_new.x, a.menu_new.y), (a.menu_top.x + 2, a.menu_top.y));
        assert_eq!(a.menu_new.width, 3);
        assert_eq!((a.menu_save.x, a.menu_save.width), (a.menu_top.x + 10, 4));
        assert_eq!((a.menu_load.x, a.menu_load.width), (a.menu_top.x + 19, 4));
        assert_eq!(hit_menu(&a, a.menu_new.x + 1, a.menu_new.y), Some(MenuAction::New));
        assert_eq!(hit_menu(&a, a.menu_save.x, a.menu_save.y), Some(MenuAction::Save));
        assert_eq!(hit_menu(&a, a.menu_load.x + 3, a.menu_load.y), Some(MenuAction::Load));
        assert_eq!(hit_menu(&a, 0, 0), None);
        // Tools: 11 rows; colors panel directly below the tools box.
        assert_eq!(a.tools.height, 11);
        assert_eq!(a.colors.y, a.tools.y + 11 + 2); // + bottom/top borders
        assert!(a.colors.height >= 3);
        // Palette: 7 tab rows, divider, then grid.
        assert_eq!(a.palette_tabs.height, 7);
        assert_eq!(a.palette_grid.y, a.palette_tabs.y + 7 + 1);
        // Colors rows map: 0 transparent, then ANSI slots; clicks outside
        // the visible rows miss.
        let mut probe = App::new(
            omaframe::model::Document::new("t", 80, 24),
            None,
        );
        assert_eq!(hit_colors(&a, &probe, a.colors.x, a.colors.y), Some(0));
        let last_visible = a.colors.height as usize - 1;
        assert_eq!(
            hit_colors(&a, &probe, a.colors.x, a.colors.y + last_visible as u16),
            Some(last_visible)
        );
        assert_eq!(
            hit_colors(&a, &probe, a.colors.x, a.colors.y + a.colors.height),
            None
        );
        assert!(over_colors(&a, a.colors.x, a.colors.y));
    }

    #[test]
    fn wireframe_chrome_snapshot() {
        // The user-supplied wireframe: menu bar, boxed Tools/Colors/Palette,
        // origin-titled canvas. Renders the demo doc headless at 80x24.
        let doc =
            omaframe::model::load_json(include_str!("../testdata/demo.omaframe.json"))
                .expect("demo");
        let (mut app, t) = (App::new(doc, None), theme::defaults());
        let buf = render_to_buf(&mut app, &t);
        let rows: Vec<String> = (0..24)
            .map(|y| {
                (0..80)
                    .map(|x| buf[(x, y)].symbol().to_string())
                    .collect::<String>()
            })
            .collect();
        let screen = rows.join("\n");
        for needle in [
            "New", "Save", "Load", // menu bar
            "Tools", "pan",        // tools box + Pan tool
            "Colors", "none",      // colors box + transparent row
            "Palette", "Outline", "Glyphs", // palette box + renamed tabs
            "0,0",                 // canvas origin in its border
            "untitled",            // file row
            "┌", "┐", "└", "┘", "│", "─", // box chrome
        ] {
            assert!(screen.contains(needle), "missing {needle:?}:\n{screen}");
        }
    }

    #[test]
    fn eraser_drag_inverts_target_cells() {
        let (mut app, t) = harness();
        app.set_tool(Tool::Eraser);
        app.start_stroke(0, 0, false);
        app.update_stroke(3, 2, false);
        let buf = render_to_buf(&mut app, &t);
        // Painted cell under the eraser rect renders inverted (not silently
        // transparent-looking).
        let (sx, sy) = screen_of(2, 1);
        let cell = &buf[(sx, sy)];
        assert_eq!(cell.symbol(), "x");
        assert!(
            cell.modifier.contains(Modifier::REVERSED),
            "erased cell not inverted: {cell:?}"
        );
        // Empty cell inside the rect inverts too.
        let (ex, ey) = screen_of(0, 0);
        assert!(buf[(ex, ey)].modifier.contains(Modifier::REVERSED));
        // Untouched cell outside the rect does not.
        let (ox, oy) = screen_of(10, 10);
        assert!(!buf[(ox, oy)].modifier.contains(Modifier::REVERSED));
    }

    #[test]
    fn select_rubber_band_inverts() {
        let (mut app, t) = harness();
        app.set_tool(Tool::Select);
        app.start_stroke(5, 5, false);
        app.update_stroke(7, 6, false);
        let buf = render_to_buf(&mut app, &t);
        let (sx, sy) = screen_of(5, 5);
        assert!(buf[(sx, sy)].modifier.contains(Modifier::REVERSED));
        let (ox, oy) = screen_of(0, 0);
        assert!(!buf[(ox, oy)].modifier.contains(Modifier::REVERSED));
    }
}
