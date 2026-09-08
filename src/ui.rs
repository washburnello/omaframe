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

use crate::app::{App, PALETTE_TABS, Tool, palette_chars};
use omaframe::theme;

// ---------------------------------------------------------------------------
// Layout
// ---------------------------------------------------------------------------

pub const MIN_W: u16 = 60;
pub const MIN_H: u16 = 12;
const TOOLS_W: u16 = 11;
const PALETTE_W: u16 = 24;
const RULER_LEFT_W: u16 = 5;

/// Screen rects. `tools`, `palette_tabs`, `palette_grid`, `canvas`, `layers`
/// are interaction rects (titles excluded); the render functions draw titles
/// in the rows above them.
#[derive(Clone, Copy, Debug, Default)]
pub struct LayoutAreas {
    pub toolbar: Rect,
    pub tools: Rect,
    pub canvas: Rect,
    pub ruler_top: Rect,
    pub ruler_left: Rect,
    pub palette_title: Rect,
    pub palette_tabs: Rect,
    pub palette_pots: Rect,
    pub palette_grid: Rect,
    pub layers: Rect,
    pub status: Rect,
    pub too_small: bool,
}

fn rows(area: Rect, n: u16) -> Rect {
    Rect::new(area.x, area.y, area.width, area.height.min(n))
}

/// Pure layout: identical rects for rendering and mouse mapping.
pub fn compute_layout(area: Rect, show_rulers: bool) -> LayoutAreas {
    if area.width < MIN_W || area.height < MIN_H {
        return LayoutAreas {
            too_small: true,
            ..Default::default()
        };
    }
    let v = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1), // toolbar
            Constraint::Min(4),    // main
            Constraint::Length(1), // layers row
            Constraint::Length(1), // status bar
        ])
        .split(area);
    let toolbar = v[0];
    let layers = v[2];
    let status = v[3];
    let h = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Length(TOOLS_W),
            Constraint::Min(10),
            Constraint::Length(PALETTE_W),
        ])
        .split(v[1]);
    let tools_outer = h[0];
    let canvas_outer = h[1];
    let palette_outer = h[2];

    // Tools: 1 title row + 9 tool rows.
    let tools_tv = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Min(1)])
        .split(tools_outer);
    let tools = Rect::new(
        tools_tv[1].x,
        tools_tv[1].y,
        tools_tv[1].width,
        tools_tv[1].height.min(9),
    );

    // Canvas: optional rulers consume the top row / left columns.
    let (canvas, ruler_top, ruler_left) = if show_rulers {
        let vv = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(1), Constraint::Min(1)])
            .split(canvas_outer);
        let hh = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Length(RULER_LEFT_W), Constraint::Min(1)])
            .split(vv[1]);
        (hh[1], vv[0], hh[0])
    } else {
        (
            canvas_outer,
            Rect::default(),
            Rect::default(),
        )
    };

    // Palette: title + 7 tab rows + 2 pot rows + grid rest.
    let pv = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Length(7),
            Constraint::Length(2),
            Constraint::Min(1),
        ])
        .split(palette_outer);
    let _ = rows;
    LayoutAreas {
        toolbar,
        tools,
        canvas,
        ruler_top,
        ruler_left,
        palette_title: pv[0],
        palette_tabs: pv[1],
        palette_pots: pv[2],
        palette_grid: pv[3],
        layers,
        status,
        too_small: false,
    }
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

fn toolbar_text(app: &App) -> String {
    format!(
        " File:Ctrl-S Save | Undo:Ctrl-Z Redo:Ctrl-Y | Prev:Ctrl-P Rules:Ctrl-R | Exp:Ctrl-E | Cmd-K | Quit:Ctrl-Q || {} ",
        app.file_label()
    )
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

fn render_toolbar(f: &mut Frame, areas: &LayoutAreas, app: &App, t: &theme::Theme) {
    let w = areas.toolbar.width as usize;
    let p = Paragraph::new(truncate_to(&toolbar_text(app), w))
        .style(Style::default().bg(t.bg).fg(t.fg));
    f.render_widget(p, areas.toolbar);
}

fn render_tools(f: &mut Frame, areas: &LayoutAreas, app: &App, t: &theme::Theme) {
    // Title row lives directly above `areas.tools`.
    let title_rect = Rect::new(
        areas.tools.x,
        areas.tools.y.saturating_sub(1),
        areas.tools.width,
        1,
    );
    f.render_widget(
        Paragraph::new("Tools").style(chrome_title_style(t)),
        title_rect,
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
    // Box-style + arrow state under the 9 tools when space allows.
    f.render_widget(Paragraph::new(lines), areas.tools);
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

fn render_rulers(f: &mut Frame, areas: &LayoutAreas, app: &App, t: &theme::Theme) {
    if !app.show_rulers {
        return;
    }
    let dim = Style::default().fg(t.dim);
    if areas.ruler_top.width > 0 {
        let mut spans = Vec::new();
        for i in 0..areas.ruler_top.width {
            let doc_x = app.viewport.0 + i as i32;
            let digit = ((doc_x % 10 + 10) % 10).to_string();
            spans.push(Span::styled(digit, dim));
        }
        f.render_widget(Paragraph::new(Line::from(spans)), areas.ruler_top);
    }
    if areas.ruler_left.width > 0 && areas.ruler_left.height > 0 {
        let mut lines = Vec::new();
        for i in 0..areas.ruler_left.height {
            let doc_y = app.viewport.1 + i as i32;
            lines.push(Line::from(vec![Span::styled(
                format!("{:>4} ", doc_y),
                dim,
            )]));
        }
        f.render_widget(Paragraph::new(lines), areas.ruler_left);
    }
}

fn render_canvas(f: &mut Frame, areas: &LayoutAreas, app: &App, t: &theme::Theme) {
    let cw = areas.canvas.width as i32;
    let ch = areas.canvas.height as i32;
    if cw <= 0 || ch <= 0 {
        return;
    }
    let (gw, gh) = (app.doc.grid.0 as i32, app.doc.grid.1 as i32);
    // Empty cells preview on the same surface as transparent-styled cells.
    let empty_style = theme::cell_style(7, -1, t, app.preview_dark);
    let cursor_style = Style::default().bg(t.highlight).fg(t.bg).add_modifier(Modifier::BOLD);
    let mut lines: Vec<Line> = Vec::with_capacity(ch as usize);
    for dy in 0..ch {
        let doc_y = app.viewport.1 + dy;
        let mut spans: Vec<Span> = Vec::new();
        if doc_y < 0 || doc_y >= gh {
            spans.push(Span::styled("~".repeat(cw.max(0) as usize), Style::default().fg(t.dim)));
            lines.push(Line::from(spans));
            continue;
        }
        for dx in 0..cw {
            let doc_x = app.viewport.0 + dx;
            if doc_x < 0 || doc_x >= gw {
                spans.push(Span::styled(" ", empty_style));
                continue;
            }
            if is_guard(&app.doc, &app.scratch, doc_x, doc_y) {
                continue; // wide anchor already occupies both terminal cols
            }
            let on_cursor = (doc_x, doc_y) == app.cursor;
            match app.doc.compose(&app.scratch, doc_x, doc_y) {
                Some(c) => {
                    let mut st = theme::cell_style(c.fg, c.bg, t, app.preview_dark);
                    if on_cursor {
                        st = cursor_style;
                    }
                    spans.push(Span::styled(c.ch.clone(), st));
                }
                None => {
                    if on_cursor {
                        spans.push(Span::styled(" ", cursor_style));
                    } else {
                        spans.push(Span::styled(" ", empty_style));
                    }
                }
            }
        }
        lines.push(Line::from(spans));
    }
    f.render_widget(Paragraph::new(lines), areas.canvas);
}

fn ansi_swatch(idx: i8, t: &theme::Theme) -> Style {
    if idx < 0 {
        Style::default().fg(t.dim)
    } else {
        Style::default().fg(t.ansi[(idx as usize).min(15)])
    }
}

fn render_palette(f: &mut Frame, areas: &LayoutAreas, app: &App, t: &theme::Theme) {
    f.render_widget(
        Paragraph::new("Palette").style(chrome_title_style(t)),
        areas.palette_title,
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
    // Pots.
    let fg_line = Line::from(vec![
        Span::styled("fg ", Style::default().fg(t.fg)),
        Span::styled(format!("{:>2} ", app.fg), Style::default().fg(t.fg)),
        Span::styled("■", ansi_swatch(app.fg, t)),
    ]);
    let bg_label = if app.bg < 0 { "-".to_string() } else { app.bg.to_string() };
    let bg_line = Line::from(vec![
        Span::styled("bg ", Style::default().fg(t.fg)),
        Span::styled(format!("{:>2} ", bg_label), Style::default().fg(t.fg)),
        Span::styled("■", ansi_swatch(app.bg, t)),
    ]);
    f.render_widget(Paragraph::new(vec![fg_line, bg_line]), areas.palette_pots);
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
    let (w, h) = (app.doc.grid.0, app.doc.grid.1);
    let bg_label = if app.bg < 0 { "-".to_string() } else { app.bg.to_string() };
    let sel = match app.history.selection() {
        Some(r) => format!(" sel {}x{}@{},{}", r.w, r.h, r.x, r.y),
        None => String::new(),
    };
    let msg = format!(
        "cell {},{} · {} · fg {} bg {} · {} · {}x{}{} · {} [{}] · {}",
        app.cursor.0,
        app.cursor.1,
        app.active_layer_name(),
        app.fg,
        bg_label,
        app.file_label(),
        w,
        h,
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
    let areas = compute_layout(area, app.show_rulers);
    if areas.too_small {
        let msg = format!(
            "terminal too small: need {}x{}, have {}x{} — enlarge to draw",
            MIN_W, MIN_H, area.width, area.height
        );
        f.render_widget(Paragraph::new(msg), area);
        return;
    }
    render_toolbar(f, &areas, app, t);
    render_tools(f, &areas, app, t);
    render_rulers(f, &areas, app, t);
    render_canvas(f, &areas, app, t);
    render_palette(f, &areas, app, t);
    render_layers(f, &areas, app, t);
    render_status(f, &areas, app, t);
}
