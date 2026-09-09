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
    widgets::{Clear, Paragraph},
};

use crate::app::{
    App, DialogPurpose, DirEntry, FileDialog, PALETTE_TABS, Tool, fs_glyphs, palette_chars,
};
use omaframe::model::{self, PaintColor};
use omaframe::theme;
use omaframe::theme::ColorRow;

// ---------------------------------------------------------------------------
// Layout
// ---------------------------------------------------------------------------

pub const MIN_W: u16 = 60;
pub const MIN_H: u16 = 16;
const LEFT_W: u16 = 15;
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

/// 1-cell breathing gaps between the panel columns (wireframe clarity).
const GAP_W: u16 = 1;

/// Screen rects. `tools`, `colors`, `canvas`, `palette_*`, `layer_rows`
/// are inner interaction rects (inside their panel borders); render
/// functions draw the surrounding box in the rows/cols around them.
/// Gaps belong to no panel: clicks there do nothing.
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
    pub layer_rows: Rect,
    pub too_small: bool,
}

/// Pure layout: identical rects for rendering and mouse mapping.
///
/// ```text
/// row 0: ┌─┐Tools┌─┐ ┌─┐New┌─┐Save┌─┐Load┌─┐ ┌─┐Palette┌─┐
/// row 1: │ tool    │ │ File: …          │ │ tabs      │
/// row 2: │ …       │ └────────────────┘ │ …         │
/// ```
/// Panels own their border cells; `*_inner` rects below are the content.
/// `n_layers` sizes the layers box pinned under the palette.
pub fn compute_layout(area: Rect, n_layers: usize) -> LayoutAreas {
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
        ])
        .split(area);
    let h = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Length(LEFT_W),
            Constraint::Length(GAP_W), // breathing room, tools ↔ canvas
            Constraint::Min(10),
            Constraint::Length(GAP_W), // breathing room, canvas ↔ palette
            Constraint::Length(PALETTE_W),
        ])
        .split(v[0]);
    let left_outer = h[0];
    let center_outer = h[2];
    let palette_outer = h[4];

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
    // Menu items live in the top border (`┌─┐New┌─┐Save┌─┐Load┌──…`):
    // fixed offsets from the outer left edge.
    let menu_new = Rect::new(menu_outer.x + 3, menu_outer.y, 3, 1);
    let menu_save = Rect::new(menu_outer.x + 9, menu_outer.y, 4, 1);
    let menu_load = Rect::new(menu_outer.x + 16, menu_outer.y, 4, 1);
    let canvas_outer = Rect::new(
        center_outer.x,
        menu_outer.y + menu_outer.height,
        center_outer.width,
        center_outer.height.saturating_sub(menu_outer.height),
    );
    let canvas = inset1(canvas_outer);

    // Palette column: palette box on top, layers box pinned to the bottom.
    // Layers rows display topmost-first; the box is rows + top + bottom.
    let layer_box_h = (n_layers as u16 + 2).min(palette_outer.height);
    let layers_outer = Rect::new(
        palette_outer.x,
        palette_outer.y + palette_outer.height.saturating_sub(layer_box_h),
        palette_outer.width,
        layer_box_h,
    );
    let layer_rows = inset1(layers_outer);
    let palette_box_h = palette_outer
        .height
        .saturating_sub(layers_outer.height);
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
        palette_box_h.saturating_sub(1 + 7 + 1 + 1), // top, tabs, divider, bottom
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
        layer_rows,
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

/// Grid scroll affordances (wireframe `▲`/`▼` at the grid's right edge):
/// whether more rows exist above/below the visible window. Render and
/// hit-testing share this so the arrows always agree with clicks.
pub fn palette_scroll_hint(areas: &LayoutAreas, app: &App) -> (bool, bool) {
    let gh = areas.palette_grid.height as usize;
    if gh == 0 {
        return (false, false);
    }
    let len = palette_chars(app.palette_tab).len();
    if app.palette_tab == 6 {
        (
            app.palette_scroll > 0,
            app.palette_scroll + gh < len,
        )
    } else {
        let cols = palette_grid_cols(areas, app.palette_tab);
        (
            app.palette_scroll > 0,
            (app.palette_scroll + gh) * cols < len,
        )
    }
}

/// Map a click on a scroll arrow to its direction (`-1` up, `+1` down).
/// Must be consulted before [`hit_palette_grid`]: arrows overwrite item
/// cells. Row 0's last slot scrolls up; the last row's last slot scrolls
/// down (up wins when the grid is a single row).
pub fn hit_palette_scroll(
    areas: &LayoutAreas,
    app: &App,
    col: u16,
    row: u16,
) -> Option<i32> {
    if !contains(areas.palette_grid, col, row) {
        return None;
    }
    let gh = areas.palette_grid.height as usize;
    if gh == 0 {
        return None;
    }
    let (up, down) = palette_scroll_hint(areas, app);
    if !up && !down {
        return None;
    }
    let rel_row = (row - areas.palette_grid.y) as usize;
    if app.palette_tab == 6 {
        // Widgets tab: arrows prefix the row (first 2 columns).
        if col >= areas.palette_grid.x + 2 {
            return None;
        }
    } else {
        // Grid tabs: arrows own the last 2-wide slot.
        let cols = palette_grid_cols(areas, app.palette_tab);
        let rel_col = ((col - areas.palette_grid.x) / 2) as usize;
        if rel_col + 1 != cols {
            return None;
        }
    }
    if up && rel_row == 0 {
        return Some(-1);
    }
    if down && rel_row == gh - 1 && !(gh == 1 && up) {
        return Some(1);
    }
    None
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

/// Layers panel rows (bottom-right box): display topmost-first, so display
/// row `r` is layer `len - 1 - r`. Returns `(layer_idx, toggle_eye)` —
/// clicks on the row's first 2 columns toggle visibility, the rest selects.
pub fn hit_layer_row(
    areas: &LayoutAreas,
    n_layers: usize,
    col: u16,
    row: u16,
) -> Option<(usize, bool)> {
    if !contains(areas.layer_rows, col, row) || n_layers == 0 {
        return None;
    }
    let r = (row - areas.layer_rows.y) as usize;
    if r >= n_layers {
        return None;
    }
    let idx = n_layers - 1 - r;
    let eye = col < areas.layer_rows.x + 2;
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

/// Total grouped Colors-panel rows (see `theme::color_rows`). The group
/// structure (Backgrounds, Foregrounds, Accent, Colors, Brights, Pico-8,
/// Picotron) is theme-independent, so the default theme gives the canonical
/// count. `main.rs` passes this to `App::scroll_colors` as `total_rows`.
pub fn colors_total_rows() -> usize {
    theme::color_rows(&theme::defaults()).len()
}

/// Scrollbar geometry for the Colors panel: `(thumb_start, thumb_len)` in
/// track coordinates (the track is the inner height minus the ▲▼ rows).
/// `None` when everything fits (no bar) or there is no room for one.
/// Pure math, unit-tested below.
pub fn scrollbar_geom(total: usize, scroll: usize, visible: usize) -> Option<(usize, usize)> {
    if total <= visible || visible < 3 {
        return None;
    }
    let track = visible - 2;
    let max_scroll = total - visible;
    let start = scroll.min(max_scroll);
    let thumb_len = ((visible * track) / total).max(1).min(track);
    let thumb_start = if max_scroll == 0 || track <= thumb_len {
        0
    } else {
        start * (track - thumb_len) / max_scroll
    };
    Some((thumb_start, thumb_len))
}

/// Colors-panel scrollbar clicks: arrows step, track jumps. `Jump` carries
/// the 0-based track row (below the ▲).
pub enum ColorBarHit {
    Up,
    Down,
    Jump(usize),
}

pub fn hit_colors_bar(
    areas: &LayoutAreas,
    app: &App,
    col: u16,
    row: u16,
) -> Option<ColorBarHit> {
    let gh = areas.colors.height as usize;
    let geom = scrollbar_geom(colors_total_rows(), app.colors_scroll, gh)?;
    if !contains(areas.colors, col, row) {
        return None;
    }
    if col != areas.colors.x + areas.colors.width - 1 {
        return None;
    }
    let r = (row - areas.colors.y) as usize;
    if r == 0 {
        return Some(ColorBarHit::Up);
    }
    if r + 1 == gh {
        return Some(ColorBarHit::Down);
    }
    let (tstart, tlen) = geom;
    let tr = r - 1;
    if tr >= tstart && tr < tstart + tlen {
        return None; // on the thumb: nothing to jump to (no drag support)
    }
    Some(ColorBarHit::Jump(tr))
}

/// Target scroll for a track click: proportional position, ends exact.
pub fn colors_bar_target(total: usize, visible: usize, track_row: usize) -> usize {
    let max = total.saturating_sub(visible.max(1));
    let track = visible.saturating_sub(2);
    if track <= 1 {
        return if track_row == 0 { 0 } else { max };
    }
    (track_row * max / (track - 1)).min(max)
}

/// Map a click in the Colors panel to a ROW index into `theme::color_rows`
/// (scroll-adjusted via `app.colors_scroll`, clamped to the row count).
/// `main.rs` maps the row via `color_rows`: `Header` rows are not clickable
/// (ignore the click), `Transparent` clears the bg pot, `Entry` sets a pot.
pub fn hit_colors(
    areas: &LayoutAreas,
    app: &App,
    col: u16,
    row: u16,
) -> Option<usize> {
    if !contains(areas.colors, col, row) {
        return None;
    }
    // The scrollbar owns the last column when visible (see hit_colors_bar).
    let gh = areas.colors.height as usize;
    if scrollbar_geom(colors_total_rows(), app.colors_scroll, gh).is_some()
        && col >= areas.colors.x + areas.colors.width - 1
    {
        return None;
    }
    let idx = app.colors_scroll + (row - areas.colors.y) as usize;
    if idx < colors_total_rows() {
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
// File dialog overlay (pure geometry shared by render + main.rs clicks)
// ---------------------------------------------------------------------------

/// Modal cap sizes: the dialog is `min(72, w-4)` × `min(22, h-2)`.
pub const DLG_MAX_W: u16 = 72;
pub const DLG_MAX_H: u16 = 22;
/// Places-sidebar width (inner columns).
pub const DLG_SIDEBAR_W: u16 = 16;
const DLG_CANCEL_W: u16 = 6; // "Cancel"
const DLG_FOLDER_W: u16 = 7; // "+Folder"
const DLG_SAVE_W: u16 = 4; // "Open" / "Save"

/// Screen rects for the modal file dialog. `cancel`/`folder`/`save` are the
/// title-row buttons; `path_row` is the cwd/filename row; `sidebar`/`list`
/// are the places + entries content rects (header row at `outer.y + 2` is
/// static and has no hit rect).
#[derive(Clone, Copy, Debug)]
pub struct DialogLayout {
    pub outer: Rect,
    pub cancel: Rect,
    pub folder: Rect,
    pub save: Rect,
    pub path_row: Rect,
    pub sidebar: Rect,
    pub list: Rect,
}

/// Centered modal `min(72, w-4)` × `min(22, h-2)`; `None` when the area is
/// below the 60×16 minimum (then nothing renders and no hits apply).
///
/// Exact geometry (title row = `outer.y`):
/// `┌─┐Cancel┌──…┬+Folder┬──┬Open|Save┌─┐` with `Cancel` at `x+3` (6 wide),
/// `+Folder` centered at `x+(dw-7)/2` (7 wide), `Open`/`Save` at `x+dw-7`
/// (4 wide, trailing `┌─┐` fills the last 3 cells). `path_row` =
/// `(x+1, y+1, dw-2, 1)`; `sidebar` =
/// `(x+1, y+3, 16, dh-4)`; `list` = `(x+18, y+3, dw-19, dh-4)`.
pub fn dialog_layout(area: Rect) -> Option<DialogLayout> {
    if area.width < MIN_W || area.height < MIN_H {
        return None;
    }
    let dw = DLG_MAX_W.min(area.width.saturating_sub(4));
    let dh = DLG_MAX_H.min(area.height.saturating_sub(2));
    if dw < 10 || dh < 6 {
        return None;
    }
    let x = area.x + (area.width - dw) / 2;
    let y = area.y + (area.height - dh) / 2;
    let folder_x = x + (dw - DLG_FOLDER_W) / 2;
    Some(DialogLayout {
        outer: Rect::new(x, y, dw, dh),
        cancel: Rect::new(x + 3, y, DLG_CANCEL_W, 1),
        folder: Rect::new(folder_x, y, DLG_FOLDER_W, 1),
        save: Rect::new(x + dw.saturating_sub(3 + DLG_SAVE_W), y, DLG_SAVE_W, 1),
        path_row: Rect::new(x + 1, y + 1, dw.saturating_sub(2), 1),
        sidebar: Rect::new(
            x + 1,
            y + 3,
            DLG_SIDEBAR_W.min(dw.saturating_sub(2)),
            dh.saturating_sub(4),
        ),
        list: Rect::new(
            x + 1 + DLG_SIDEBAR_W + 1,
            y + 3,
            dw.saturating_sub(DLG_SIDEBAR_W + 3),
            dh.saturating_sub(4),
        ),
    })
}

/// Which title-row button a click hit, if any.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum DialogButton {
    Cancel,
    Folder,
    Save,
}

/// Map a click to a title-row button (`Cancel` / `+Folder` / `Open|Save`).
/// Consult before sidebar/list: the buttons own row `outer.y`.
pub fn hit_dialog_button(
    layout: &DialogLayout,
    col: u16,
    row: u16,
) -> Option<DialogButton> {
    if contains(layout.cancel, col, row) {
        Some(DialogButton::Cancel)
    } else if contains(layout.folder, col, row) {
        Some(DialogButton::Folder)
    } else if contains(layout.save, col, row) {
        Some(DialogButton::Save)
    } else {
        None
    }
}

/// Map a click in the Places sidebar to a `FileDialog::sidebar` index.
pub fn hit_dialog_sidebar(
    layout: &DialogLayout,
    dlg: &FileDialog,
    col: u16,
    row: u16,
) -> Option<usize> {
    if !contains(layout.sidebar, col, row) {
        return None;
    }
    let idx = (row - layout.sidebar.y) as usize;
    if idx < dlg.sidebar.len() {
        Some(idx)
    } else {
        None
    }
}

/// Map a click in the file list to a `FileDialog::entries` index.
pub fn hit_dialog_list(
    layout: &DialogLayout,
    dlg: &FileDialog,
    col: u16,
    row: u16,
) -> Option<usize> {
    if !contains(layout.list, col, row) {
        return None;
    }
    let idx = (row - layout.list.y) as usize;
    if idx < dlg.entries.len() {
        Some(idx)
    } else {
        None
    }
}

// ---------------------------------------------------------------------------
// Rendering
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// Panel frames (wireframe chrome: titled box borders around every panel)
// ---------------------------------------------------------------------------

/// Top border with a corner-flanked title: `┌─┐{title}┌──…──┐`, exactly
/// `w + 2` cells wide (wireframe chrome: the title sits between corners).
/// Title is ASCII-short; truncation keeps the width exact.
fn top_titled(w: usize, title: &str) -> String {
    let t = truncate_to(title, w.saturating_sub(4));
    let mut s = format!("┌─┐{t}┌");
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

/// Canvas bottom border carrying the content size:
/// Canvas top border: viewport origin left, cursor cell right —
/// `┌─┐{ox},{oy}┌──…──{cx},{cy}┌─┐`, exactly `w + 2` cells wide. The
/// right-hand readout replaces the old status-bar cursor line.
fn canvas_top(w: usize, ox: i32, oy: i32, cx: i32, cy: i32) -> String {
    let left = format!("┌─┐{ox},{oy}┌");
    let right = format!("{cx},{cy}┌─┐");
    let ln = left.chars().count();
    let rn = right.chars().count();
    if ln + rn + 1 > w + 2 {
        return truncate_to(&format!("{left}{right}"), w + 2);
    }
    let mut s = left;
    while s.chars().count() < w + 2 - rn {
        s.push('─');
    }
    s + &right
}

/// `└─┘Size:{WxH} · {message}└──…──┘` (`"empty"` when there is no content),
/// exactly `w + 2` cells wide like [`bottom_border`]. The trailing message
/// replaces the old status bar: transient feedback lives here now.
fn canvas_bottom(w: usize, size: &str, msg: &str) -> String {
    let mut head = format!("└─┘Size:{size}");
    let m = msg.trim();
    if !m.is_empty() {
        head = format!("{head} · {m}");
    }
    // Break at a word boundary so the message never ends mid-word.
    let max = w.saturating_sub(1);
    let mut head = truncate_to(&head, max);
    if head.chars().count() >= max && !head.ends_with([' ', '·']) {
        if let Some(i) = head.rfind(' ') {
            head.truncate(i);
        }
    }
    let mut s = head;
    while s.chars().count() < w + 1 {
        s.push('─');
    }
    s.push('┘');
    s
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
    // Wireframe menu chrome (`┌─┐New┌─┐Save┌─┐Load┌──…`): the label spans
    // below must reproduce these offsets exactly for click mapping.
    let mut spans = vec![
        Span::styled("┌─┐", dim),
        Span::styled("New", item),
        Span::styled("┌─┐", dim),
        Span::styled("Save", item),
        Span::styled("┌─┐", dim),
        Span::styled("Load", item),
        Span::styled("┌", dim),
    ];
    let used: usize = "┌─┐New┌─┐Save┌─┐Load┌".chars().count();
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

/// Tool icon column: the pencil gets its nerd glyph (U+F040, verified in
/// `assets/nerd.txt`); every other tool gets a blank cell so labels align.
fn tool_icon(tool: Tool) -> &'static str {
    match tool {
        Tool::Pencil => "\u{F040}",
        _ => " ",
    }
}

/// Hotkey letter to highlight inside the label (btop discipline: the
/// highlighted letter IS the key). `None` when the shortcut appears nowhere
/// in the label — those rows get a dim `(key)` suffix instead.
fn tool_hotkey(tool: Tool) -> Option<char> {
    let s = tool.shortcut();
    if tool.label().chars().any(|c| c.eq_ignore_ascii_case(&s)) {
        return Some(s);
    }
    // 's' also selects (see `Tool::from_shortcut`).
    if tool == Tool::Select {
        return Some('s');
    }
    None
}

/// Suffix for tools without an in-label hotkey: the literal key.
fn tool_key_suffix(tool: Tool) -> &'static str {
    match tool {
        // Space is the discoverable Pan key ('_' works too).
        Tool::Pan => " (space)",
        Tool::Rect => " (d)",
        _ => "",
    }
}

fn render_tools(f: &mut Frame, areas: &LayoutAreas, app: &App, t: &theme::Theme) {
    let outer = outer_of(areas.tools);
    let w = areas.tools.width as usize;
    render_top_row(
        f,
        outer,
        Line::from(Span::styled(
            top_titled(w, "Tools"),
            Style::default().fg(t.dim),
        )),
        t,
    );
    let mut lines: Vec<Line> = Vec::new();
    for tool in Tool::ALL {
        let active = tool == app.tool;
        let base = if active {
            Style::default().bg(t.accent).fg(t.bg).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(t.fg)
        };
        // Hotkey letter contrasts against the row background either way.
        let hot = if active {
            Style::default()
                .bg(t.accent)
                .fg(t.bg)
                .add_modifier(Modifier::BOLD | Modifier::UNDERLINED)
        } else {
            Style::default()
                .fg(t.accent)
                .add_modifier(Modifier::BOLD | Modifier::UNDERLINED)
        };
        let mut spans = vec![Span::styled(tool_icon(tool), base), Span::styled(" ", base)];
        let label = tool.label();
        match tool_hotkey(tool) {
            Some(h) => {
                let pos = label
                    .chars()
                    .position(|c| c.eq_ignore_ascii_case(&h))
                    .unwrap_or(0);
                let pre: String = label.chars().take(pos).collect();
                let hot_c: String = label.chars().skip(pos).take(1).collect();
                let post: String = label.chars().skip(pos + 1).collect();
                // Capitalize the hotkey like btop (`Pencil`, `Box`, …).
                spans.push(Span::styled(pre, base));
                spans.push(Span::styled(hot_c.to_uppercase(), hot));
                spans.push(Span::styled(post, base));
            }
            None => {
                spans.push(Span::styled(label.to_string(), base));
                spans.push(Span::styled(tool_key_suffix(tool), Style::default().fg(t.dim)));
            }
        }
        lines.push(Line::from(spans));
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
    // Boxed canvas: viewport origin left, cursor cell right (the old
    // status-bar cursor line now lives in the border).
    let outer = outer_of(areas.canvas);
    let w = areas.canvas.width as usize;
    render_top_row(
        f,
        outer,
        Line::from(Span::styled(
            canvas_top(
                w,
                app.viewport.0,
                app.viewport.1,
                app.cursor.0,
                app.cursor.1,
            ),
            Style::default().fg(t.dim),
        )),
        t,
    );
    // Empty cells preview on the same surface as transparent-styled cells.
    let empty_style = theme::cell_style(PaintColor::Ansi(7), None, t, app.preview_dark);
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
                Some(c) => theme::cell_style(c.fg.clone(), c.bg.clone(), t, app.preview_dark),
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
    // Bottom border carries the content size plus the latest status message
    // (the old status bar lives here now).
    let size = match model::content_bounds(&app.doc) {
        Some(b) => format!("{}x{}", b.w, b.h),
        None => "empty".to_string(),
    };
    f.render_widget(
        Paragraph::new(canvas_bottom(w, &size, &app.status_msg))
            .style(Style::default().fg(t.dim)),
        Rect::new(outer.x, outer.y + outer.height - 1, outer.width, 1),
    );
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
            top_titled(w, "Palette"),
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
    // Grid (scroll arrows overwrite the last slot of the first/last row).
    let items = palette_chars(app.palette_tab);
    let cols = palette_grid_cols(areas, app.palette_tab);
    let gh = areas.palette_grid.height as usize;
    let (up, down) = palette_scroll_hint(areas, app);
    let down_row_only = down && !(gh == 1 && up);
    let arrow_style = Style::default()
        .fg(t.accent)
        .add_modifier(Modifier::BOLD);
    let mut lines: Vec<Line> = Vec::new();
    if app.palette_tab == 6 {
        let gw = areas.palette_grid.width as usize;
        for r in 0..gh {
            let idx = app.palette_scroll + r;
            if idx >= items.len() {
                lines.push(Line::from(""));
                continue;
            }
            let style = Style::default().fg(t.fg);
            if up && r == 0 {
                lines.push(Line::from(vec![
                    Span::styled("▲ ", arrow_style),
                    Span::styled(truncate_to(&items[idx], gw.saturating_sub(2)), style),
                ]));
            } else if down_row_only && r == gh - 1 {
                lines.push(Line::from(vec![
                    Span::styled("▼ ", arrow_style),
                    Span::styled(truncate_to(&items[idx], gw.saturating_sub(2)), style),
                ]));
            } else {
                lines.push(Line::from(vec![Span::styled(items[idx].clone(), style)]));
            }
        }
    } else {
        for r in 0..gh {
            let mut spans: Vec<Span> = Vec::new();
            let arrow_row = (up && r == 0) || (down_row_only && r == gh - 1);
            for c in 0..cols {
                if arrow_row && c + 1 == cols {
                    spans.push(Span::styled(if up && r == 0 { "▲ " } else { "▼ " }, arrow_style));
                    continue;
                }
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

/// Colors panel: grouped swatch rows from `theme::color_rows` under a scroll
/// window (`app.colors_scroll`). Left-click sets fg, right-click sets bg
/// (`main.rs` maps the clicked row via `color_rows`). Headers render dim and
/// are not clickable; markers: `>` fg pot, `*` bg pot, `#` both.
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
            top_titled(w, "Colors"),
            Style::default().fg(t.dim),
        )),
        t,
    );
    let rows = theme::color_rows(t);
    let groups = theme::color_groups(t);
    let gh = areas.colors.height as usize;
    // Content width reserves the scrollbar column when visible.
    let bar_here =
        scrollbar_geom(rows.len(), app.colors_scroll, gh).is_some();
    let cw = if bar_here {
        w.saturating_sub(1)
    } else {
        w
    };
    let mut lines: Vec<Line> = Vec::new();
    for r in 0..gh {
        let idx = app.colors_scroll + r;
        let Some(row) = rows.get(idx) else {
            lines.push(Line::from(""));
            continue;
        };
        match row {
            ColorRow::Header(name) => {
                lines.push(Line::from(vec![Span::styled(
                    truncate_to(&format!("-{name}-"), cw),
                    Style::default().fg(t.dim),
                )]));
            }
            ColorRow::Transparent => {
                let mark = if app.bg.is_none() { "●" } else { " " };
                lines.push(Line::from(vec![
                    Span::styled(mark.to_string(), Style::default().fg(t.fg)),
                    Span::styled(" -none- ", Style::default().fg(t.dim)),
                ]));
            }
            ColorRow::Entry { group, index } => {
                let color = groups
                    .get(*group)
                    .and_then(|g| g.entries.get(*index))
                    .map(|e| e.color.clone())
                    .unwrap_or(PaintColor::Ansi(7));
                // Pot markers: ▶ fg, ● bg, ◉ both (radio-bullet family).
                let is_fg = app.fg == color;
                let is_bg = matches!(&app.bg, Some(c) if c == &color);
                let mark = match (is_fg, is_bg) {
                    (true, true) => "◉",
                    (true, false) => "▶",
                    (false, true) => "●",
                    (false, false) => " ",
                };
                lines.push(Line::from(vec![
                    Span::styled(mark.to_string(), Style::default().fg(t.fg)),
                    Span::styled(
                        "█████████",
                        Style::default()
                            .fg(theme::resolve(color, t))
                            .bg(t.bg),
                    ),
                ]));
            }
        }
    }
    // Colors scrollbar (last column): ▲▼ ends + ░ track with █ thumb so
    // the scroll position is always visible.
    if let Some((tstart, tlen)) = scrollbar_geom(rows.len(), app.colors_scroll, gh) {
        let arrow = Style::default()
            .fg(t.accent)
            .add_modifier(Modifier::BOLD);
        for (r, line) in lines.iter_mut().enumerate() {
            let cell = if r == 0 {
                "▲"
            } else if r + 1 == gh {
                "▼"
            } else {
                let tr = r - 1;
                if tr >= tstart && tr < tstart + tlen {
                    "█"
                } else {
                    "░"
                }
            };
            let style = if cell == "░" {
                Style::default().fg(t.dim)
            } else if cell == "█" {
                Style::default().fg(t.fg)
            } else {
                arrow
            };
            line.spans.push(Span::styled(cell, style));
        }
    }
    f.render_widget(Paragraph::new(lines), areas.colors);
    panel_sides(f, outer, t);
    render_bottom_row(f, outer, w, t);
}

/// Layers panel (bottom-right box, wireframe `┌─┐Layers┌──┐`): one row per
/// layer, topmost first — `▶` active, `●` visible / `○` hidden.
fn render_layers_panel(f: &mut Frame, areas: &LayoutAreas, app: &App, t: &theme::Theme) {
    let outer = outer_of(areas.layer_rows);
    let w = areas.layer_rows.width as usize;
    if outer.height < 3 || w == 0 {
        return;
    }
    render_top_row(
        f,
        outer,
        Line::from(Span::styled(
            top_titled(w, "Layers"),
            Style::default().fg(t.dim),
        )),
        t,
    );
    let n = app.doc.layers.len();
    let mut lines: Vec<Line> = Vec::new();
    for r in 0..areas.layer_rows.height as usize {
        if r >= n {
            lines.push(Line::from(""));
            continue;
        }
        let idx = n - 1 - r;
        let nl = &app.doc.layers[idx];
        let active = idx == app.doc.active;
        // Bullet-family markers: ▶ active layer, ●/○ visibility.
        let mark = if active { "▶" } else { " " };
        let eye = if nl.visible { "●" } else { "○" };
        let style = if active {
            Style::default().bg(t.accent).fg(t.bg).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(t.fg)
        };
        lines.push(Line::from(vec![Span::styled(
            truncate_to(&format!("{mark}{eye} {}", nl.name), w),
            style,
        )]));
    }
    f.render_widget(Paragraph::new(lines), areas.layer_rows);
    panel_sides(f, outer, t);
    render_bottom_row(f, outer, w, t);
}

/// Top-level render. Never panics on small terminals: below the minimum it
/// shows a one-line guard message instead of the full layout.
pub fn render(f: &mut Frame, app: &mut App, t: &theme::Theme) {
    let area = f.area();
    // Clear first: panel gaps belong to no widget and must not smear on
    // resize.
    f.render_widget(Clear, area);
    let areas = compute_layout(area, app.doc.layers.len());
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
    render_layers_panel(f, &areas, app, t);
    // Modal file dialog on top (nothing when closed or too small).
    if app.file_dialog.is_some() {
        render_dialog(f, area, app, t);
    }
}

// ---------------------------------------------------------------------------
// File dialog render (modal overlay; rects mirror `dialog_layout` exactly)
// ---------------------------------------------------------------------------

/// Pad/truncate `s` to exactly `w` display columns (char-based; dialog
/// content is narrow glyphs, same discipline as the rest of the chrome).
fn fit_to(s: &str, w: usize) -> String {
    let mut out = truncate_to(s, w);
    while out.chars().count() < w {
        out.push(' ');
    }
    out
}

/// Human size for the dialog list (`"—"` for dirs, `B`/`KB`/`MB`).
fn human_size(size: u64, is_dir: bool) -> String {
    if is_dir {
        return "—".to_string();
    }
    if size < 1024 {
        format!("{size} B")
    } else if size < 1024 * 1024 {
        format!("{:.1} KB", size as f64 / 1024.0)
    } else {
        format!("{:.1} MB", size as f64 / 1_048_576.0)
    }
}

/// Type column for the dialog list: `"dir"`, the file extension, or `"file"`.
fn entry_type(e: &DirEntry) -> String {
    if e.is_dir {
        return "dir".to_string();
    }
    match e.name.rsplit('.').next() {
        Some(ext) if !ext.is_empty() && ext.len() < e.name.len() => {
            truncate_to(ext, 8).to_string()
        }
        _ => "file".to_string(),
    }
}

/// Title row `┌─┐Cancel┌──…┬+Folder┬──┬Open|Save┌─┐`, exactly `dw` cells.
/// Button spans sit at EXACTLY the [`dialog_layout`] rects: `Cancel` at
/// `+3`, `+Folder` at `+(dw-7)/2`, save label at `+dw-7` (`dialog_title_text`
/// builds the same string; this splits it into styled spans).
fn dialog_title_line_styled(
    dw: usize,
    save_label: &str,
    dim: Style,
    btn: Style,
    save_style: Style,
) -> Line<'static> {
    let raw = dialog_title_text(dw, save_label);
    let folder_off = (dw.saturating_sub(DLG_FOLDER_W as usize)) / 2;
    let save_off = dw.saturating_sub(3 + DLG_SAVE_W as usize);
    let chars: Vec<char> = raw.chars().collect();
    let slice = |a: usize, b: usize| -> String {
        chars.get(a..b).unwrap_or(&[]).iter().collect()
    };
    Line::from(vec![
        Span::styled(slice(0, 3), dim),
        Span::styled(slice(3, 3 + DLG_CANCEL_W as usize), btn),
        Span::styled(slice(3 + DLG_CANCEL_W as usize, folder_off), dim),
        Span::styled(
            slice(folder_off, folder_off + DLG_FOLDER_W as usize),
            btn,
        ),
        Span::styled(
            slice(folder_off + DLG_FOLDER_W as usize, save_off),
            dim,
        ),
        Span::styled(
            slice(save_off, save_off + DLG_SAVE_W as usize),
            save_style,
        ),
        Span::styled(slice(save_off + DLG_SAVE_W as usize, dw), dim),
    ])
}

/// Plain-text title row (same offsets as [`dialog_layout`]); the styled
/// renderer above splits this text at the button boundaries.
fn dialog_title_text(dw: usize, save_label: &str) -> String {
    let folder_off = (dw.saturating_sub(DLG_FOLDER_W as usize)) / 2;
    let save_off = dw.saturating_sub(3 + DLG_SAVE_W as usize);
    let mut s = String::from("┌─┐Cancel┌");
    while s.chars().count() + 1 < folder_off {
        s.push('─');
    }
    s.push('┬');
    s.push_str("+Folder┬");
    while s.chars().count() + 1 < save_off {
        s.push('─');
    }
    s.push('┬');
    s.push_str(save_label);
    s.push_str("┌─┐");
    fit_to(&s, dw)
}

fn render_dialog(f: &mut Frame, area: Rect, app: &App, t: &theme::Theme) {
    let Some(dlg) = app.file_dialog.as_ref() else {
        return;
    };
    let Some(l) = dialog_layout(area) else {
        return;
    };
    let dw = l.outer.width as usize;
    let dh = l.outer.height as usize;
    if dw < 10 || dh < 6 {
        return;
    }
    let dim = Style::default().fg(t.dim);
    let btn = Style::default().fg(t.fg);
    let save_style = Style::default()
        .fg(t.accent)
        .add_modifier(Modifier::BOLD);
    let hl = Style::default()
        .bg(t.accent)
        .fg(t.bg)
        .add_modifier(Modifier::BOLD);

    let save_label = if matches!(dlg.purpose, DialogPurpose::Load) {
        "Open"
    } else {
        "Save"
    };

    // Title row with the three buttons at the `dialog_layout` rects.
    let title = dialog_title_line_styled(dw, save_label, dim, btn, save_style);
    f.render_widget(
        Paragraph::new(title),
        Rect::new(l.outer.x, l.outer.y, l.outer.width, 1),
    );

    // Path row: `│ {cwd}/{filename or selected} │` (cursor block in saves).
    let cwd = dlg.cwd.display().to_string();
    let path_content = if matches!(dlg.purpose, DialogPurpose::Load) {
        match dlg.selected.and_then(|s| dlg.entries.get(s)) {
            Some(e) if e.is_dir => format!("{cwd}/{}/", e.name),
            Some(e) => format!("{cwd}/{}", e.name),
            None => cwd,
        }
    } else {
        format!("{cwd}/{}█", dlg.filename)
    };
    let path_line = Line::from(vec![
        Span::styled("│ ", dim),
        Span::styled(
            fit_to(&path_content, dw.saturating_sub(4)),
            Style::default().fg(t.fg),
        ),
        Span::styled(" │", dim),
    ]);
    f.render_widget(Paragraph::new(path_line), l.path_row);

    // Column header (static sort indicator): `│ Places │ Name ▼ │ … │`.
    let list_w = l.list.width as usize;
    let header_right = fit_to(" Name ▼  Size    Type  Modified", list_w);
    let header_line = Line::from(vec![
        Span::styled("│ ", dim),
        Span::styled(fit_to("Places", DLG_SIDEBAR_W as usize), dim),
        Span::styled("│", dim),
        Span::styled(header_right, dim),
        Span::styled("│", dim),
    ]);
    f.render_widget(
        Paragraph::new(header_line),
        Rect::new(l.outer.x, l.outer.y + 2, l.outer.width, 1),
    );

    // Sidebar + list rows.
    let (folder_glyph, file_glyph) = fs_glyphs();
    let side_w = l.sidebar.width as usize;
    for r in 0..l.sidebar.height {
        let y = l.sidebar.y + r;
        let i = r as usize;
        let (text, style) = match dlg.sidebar.get(i) {
            // Sidebar tuple is `(label, icon, path)`; highlight the row
            // whose path is the dialog cwd.
            Some((label, icon, path)) => {
                let s = fit_to(&format!("{icon} {label}"), side_w);
                if *path == dlg.cwd {
                    (s, hl)
                } else {
                    (s, Style::default().fg(t.fg))
                }
            }
            None => (fit_to("", side_w), Style::default().fg(t.fg)),
        };
        f.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled("│", dim),
                Span::styled(text, style),
            ])),
            Rect::new(l.outer.x, y, side_w as u16 + 1, 1),
        );
        let (ltext, lstyle) = match dlg.entries.get(i) {
            Some(e) => {
                let glyph = if e.is_dir {
                    folder_glyph.clone()
                } else {
                    file_glyph.clone()
                };
                let s = fit_to(
                    &format!(
                        " {glyph} {} {} {} {}",
                        e.name,
                        human_size(e.size, e.is_dir),
                        entry_type(e),
                        e.modified
                    ),
                    list_w,
                );
                if dlg.selected == Some(i) {
                    (s, hl)
                } else {
                    (s, Style::default().fg(t.fg))
                }
            }
            None => (fit_to("", list_w), Style::default().fg(t.fg)),
        };
        f.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled("│", dim),
                Span::styled(ltext, lstyle),
                Span::styled("│", dim),
            ])),
            Rect::new(l.list.x - 1, y, l.list.width + 2, 1),
        );
    }

    // Sides + bottom border.
    panel_sides(f, l.outer, t);
    render_bottom_row(f, l.outer, dw.saturating_sub(2), t);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::{DialogMode, DialogPurpose, DirEntry};
    use omaframe::model::{Cell, Document};
    use ratatui::{Terminal, backend::TestBackend};
    use std::path::PathBuf;

    fn harness() -> (App, theme::Theme) {
        let mut doc = Document::new("t", 80, 24);
        doc.active_layer_mut()
            .set(2, 1, Cell::new("x", PaintColor::Ansi(7), None));
        (App::new(doc, None), theme::defaults())
    }

    fn render_to_buf(app: &mut App, t: &theme::Theme) -> ratatui::buffer::Buffer {
        render_to_buf_sized(app, t, 80, 24)
    }

    fn render_to_buf_sized(
        app: &mut App,
        t: &theme::Theme,
        w: u16,
        h: u16,
    ) -> ratatui::buffer::Buffer {
        let backend = TestBackend::new(w, h);
        let mut term = Terminal::new(backend).unwrap();
        term.draw(|f| render(f, app, t)).unwrap();
        term.backend().buffer().clone()
    }

    /// Screen position of a document cell with the default viewport.
    fn screen_of(doc_x: i32, doc_y: i32) -> (u16, u16) {
        let areas = compute_layout(ratatui::layout::Rect::new(0, 0, 80, 24), 3);
        (
            areas.canvas.x + doc_x as u16,
            areas.canvas.y + doc_y as u16,
        )
    }

    #[test]
    fn wireframe_layout_menu_panels_canvas() {
        // 80x24: the wireframe arrangement — menu block over the canvas,
        // tools + colors stacked left, palette right.
        let a = compute_layout(ratatui::layout::Rect::new(0, 0, 80, 24), 3);
        assert!(!a.too_small);
        // Menu items sit inside the top border at fixed offsets.
        assert_eq!((a.menu_new.x, a.menu_new.y), (a.menu_top.x + 2, a.menu_top.y));
        assert_eq!(a.menu_new.width, 3);
        assert_eq!((a.menu_save.x, a.menu_save.width), (a.menu_top.x + 8, 4));
        assert_eq!((a.menu_load.x, a.menu_load.width), (a.menu_top.x + 15, 4));
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
        let probe = App::new(
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
    fn palette_scroll_arrows_render_and_hit() {
        // Glyphs tab overflows a short grid: ▼ on the last row, nothing up.
        let (mut app, t) = harness();
        app.set_palette_tab(5);
        let buf = render_to_buf_sized(&mut app, &t, 80, 20);
        let areas = compute_layout(ratatui::layout::Rect::new(0, 0, 80, 20), 3);
        let (up, down) = palette_scroll_hint(&areas, &app);
        assert!(!up && down);
        let gx = areas.palette_grid.x;
        let cols = palette_grid_cols(&areas, 5);
        let arrow_x = gx + ((cols - 1) * 2) as u16;
        let last_y = areas.palette_grid.y + areas.palette_grid.height - 1;
        assert_eq!(buf[(arrow_x, last_y)].symbol(), "▼");
        assert_eq!(
            hit_palette_scroll(&areas, &app, arrow_x, last_y),
            Some(1)
        );
        // Plain item cells still pick.
        assert_eq!(hit_palette_scroll(&areas, &app, gx, last_y), None);
        assert!(hit_palette_grid(&areas, &app, gx, last_y).is_some());
        // Scrolled to the bottom: ▲ appears, ▼ disappears.
        app.scroll_palette(10_000, cols, areas.palette_grid.height as usize);
        let (up2, down2) = palette_scroll_hint(&areas, &app);
        assert!(up2 && !down2);
        let buf2 = render_to_buf_sized(&mut app, &t, 80, 20);
        assert_eq!(buf2[(arrow_x, areas.palette_grid.y)].symbol(), "▲");
        assert_eq!(
            hit_palette_scroll(&areas, &app, arrow_x, areas.palette_grid.y),
            Some(-1)
        );
    }

    #[test]
    fn colors_scrollbar_geometry() {
        // Fits: no bar. Too short for arrows + track: no bar.
        assert_eq!(scrollbar_geom(5, 0, 9), None);
        assert_eq!(scrollbar_geom(40, 0, 2), None);
        // 40 rows, 9 visible: track 7, thumb 1, starts at top.
        assert_eq!(scrollbar_geom(40, 0, 9), Some((0, 1)));
        // Bottom: thumb pinned to the track end.
        assert_eq!(scrollbar_geom(40, 31, 9), Some((6, 1)));
        // Middle scales proportionally: 15*6/31 = 2.
        assert_eq!(scrollbar_geom(40, 15, 9), Some((2, 1)));
        // Single-row track edge: thumb fills it.
        assert_eq!(scrollbar_geom(40, 7, 3), Some((0, 1)));
        // Jump targets hit both ends exactly.
        assert_eq!(colors_bar_target(40, 9, 0), 0);
        assert_eq!(colors_bar_target(40, 9, 6), 31);
    }

    #[test]
    fn layers_panel_maps_topmost_first() {
        let (harness_app, _) = harness();
        // Harness doc has the default layers; rows show topmost first.
        let n = harness_app.doc.layers.len();
        let a = compute_layout(ratatui::layout::Rect::new(0, 0, 80, 24), n);
        assert_eq!(a.layer_rows.height, n as u16);
        let y0 = a.layer_rows.y;
        // Row 0 → top layer (idx n-1); last row → Background (idx 0).
        assert_eq!(
            hit_layer_row(&a, n, a.layer_rows.x + 5, y0),
            Some((n - 1, false))
        );
        assert_eq!(
            hit_layer_row(&a, n, a.layer_rows.x + 5, y0 + n as u16 - 1),
            Some((0, false))
        );
        // First two columns toggle the eye.
        assert_eq!(
            hit_layer_row(&a, n, a.layer_rows.x, y0 + 1),
            Some((n - 2, true))
        );
        assert_eq!(hit_layer_row(&a, n, a.layer_rows.x, y0 + n as u16), None);
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

    fn screen_text(buf: &ratatui::buffer::Buffer) -> String {
        let (w, h) = (buf.area.width, buf.area.height);
        (0..h)
            .map(|y| {
                (0..w)
                    .map(|x| buf[(x, y)].symbol().to_string())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn grouped_colors_rows_render() {
        let t = theme::defaults();
        let rows = theme::color_rows(&t);
        // 5 headers + Transparent + 25 live-variable entries.
        assert_eq!(rows.len(), 5 + 1 + 25);
        assert_eq!(rows.len(), colors_total_rows());
        // A Colors group header exists per the theme contract.
        let colors = rows
            .iter()
            .position(|r| {
                matches!(r, ColorRow::Header(n) if n == "Colors")
            })
            .expect("Colors header present");
        let colors_name = match &rows[colors] {
            ColorRow::Header(n) => n.clone(),
            _ => unreachable!(),
        };
        // Scroll the header to the top of the window and check the dim
        // `-Name-` row renders.
        let (mut app, t) = (App::new(Document::new("t", 80, 24), None), t);
        app.colors_scroll = colors;
        let screen = screen_text(&render_to_buf(&mut app, &t));
        assert!(
            screen.contains(&format!("-{colors_name}-")),
            "missing header -{colors_name}-:\n{screen}"
        );
        // Transparent row renders ` -none- ` with the `*` bg marker.
        let transp = rows
            .iter()
            .position(|r| matches!(r, ColorRow::Transparent))
            .expect("transparent row present");
        app.bg = None;
        app.colors_scroll = transp;
        let screen = screen_text(&render_to_buf(&mut app, &t));
        assert!(screen.contains("-none-"), "missing -none-:\n{screen}");
        // Entry row: fg pot marker `▶` plus a 9-block swatch.
        let (entry_idx, entry_color) = rows
            .iter()
            .enumerate()
            .find_map(|(i, r)| match r {
                ColorRow::Entry { group, index } => theme::color_groups(&t)
                    .get(*group)
                    .and_then(|g| g.entries.get(*index))
                    .map(|e| (i, e.color.clone())),
                _ => None,
            })
            .expect("at least one entry row");
        app.fg = entry_color.clone();
        app.bg = None;
        app.colors_scroll = entry_idx;
        let areas = compute_layout(ratatui::layout::Rect::new(0, 0, 80, 24), 3);
        assert_eq!(
            hit_colors(&areas, &app, areas.colors.x, areas.colors.y),
            Some(entry_idx)
        );
        let buf = render_to_buf(&mut app, &t);
        assert_eq!(buf[(areas.colors.x, areas.colors.y)].symbol(), "▶");
        let row_text: String = (0..areas.colors.width)
            .map(|x| buf[(areas.colors.x + x, areas.colors.y)].symbol().to_string())
            .collect();
        assert!(
            row_text.contains("█████████"),
            "missing swatch in {row_text:?}"
        );
        // Both pots on the entry → `◉`; bg pot only → `●`.
        app.bg = Some(entry_color.clone());
        let buf = render_to_buf(&mut app, &t);
        assert_eq!(buf[(areas.colors.x, areas.colors.y)].symbol(), "◉");
        app.fg = PaintColor::Ansi(0);
        if entry_color != PaintColor::Ansi(0) {
            let buf = render_to_buf(&mut app, &t);
            assert_eq!(buf[(areas.colors.x, areas.colors.y)].symbol(), "●");
        }
    }

    #[test]
    fn tools_rail_btop_hotkeys_and_pencil_icon() {
        let (mut app, t) = harness();
        let (accent, bg) = (t.accent, t.bg);
        let buf = render_to_buf(&mut app, &t);
        let areas = compute_layout(ratatui::layout::Rect::new(0, 0, 80, 24), 3);
        // Pencil row: nerd pencil icon + `P` hotkey underlined. The active
        // row inverts the hotkey (bg-colored on the accent row) so it stays
        // legible — contrast either way.
        let pencil = &buf[(areas.tools.x + 2, areas.tools.y)];
        assert_eq!(pencil.symbol(), "P", "hotkey letter: {pencil:?}");
        assert!(
            pencil.modifier.contains(Modifier::UNDERLINED),
            "hotkey underlined: {pencil:?}"
        );
        assert_eq!(pencil.fg, bg, "active-row hotkey contrast: {pencil:?}");
        assert_eq!(buf[(areas.tools.x, areas.tools.y)].symbol(), "\u{F040}");
        let screen = screen_text(&buf);
        assert!(screen.contains("Pencil"), "label:\n{screen}");
        assert!(screen.contains("> Outline"), "tabs keep >:\n{screen}");
        // Inactive rows use the accent hotkey.
        app.set_tool(Tool::Pan);
        let buf = render_to_buf(&mut app, &t);
        let pencil = &buf[(areas.tools.x + 2, areas.tools.y)];
        assert_eq!(pencil.fg, accent, "inactive-row hotkey: {pencil:?}");
        assert_eq!(buf[(areas.tools.x, areas.tools.y)].symbol(), "\u{F040}");
        let screen = screen_text(&buf);
        assert!(screen.contains("Pencil"), "label:\n{screen}");
        assert!(screen.contains("> Outline"), "tabs keep >:\n{screen}");
        // Tools without an in-label hotkey get a dim suffix instead.
        app.set_tool(Tool::Rect);
        let screen = screen_text(&render_to_buf(&mut app, &t));
        assert!(screen.contains("rect (d)"), "rect suffix:\n{screen}");
        app.set_tool(Tool::Pan);
        let screen = screen_text(&render_to_buf(&mut app, &t));
        assert!(screen.contains("pan (space)"), "pan suffix:\n{screen}");
    }

    #[test]
    fn canvas_bottom_border_shows_content_size() {
        let (mut app, t) = harness();
        // Harness paints one cell at (2,1) → 1×1 content.
        let screen = screen_text(&render_to_buf(&mut app, &t));
        assert!(screen.contains("Size:1x1"), "missing Size:1x1:\n{screen}");
        let (mut app, t) = (App::new(Document::new("t", 80, 24), None), t);
        let screen = screen_text(&render_to_buf(&mut app, &t));
        assert!(
            screen.contains("Size:empty"),
            "missing Size:empty:\n{screen}"
        );
    }

    fn fake_dialog(purpose: DialogPurpose) -> FileDialog {
        FileDialog {
            mode: if matches!(purpose, DialogPurpose::Load) {
                DialogMode::Open
            } else {
                DialogMode::Save
            },
            purpose,
            cwd: PathBuf::from("/tmp/oma"),
            entries: vec![
                DirEntry {
                    name: "docs".to_string(),
                    is_dir: true,
                    size: 0,
                    modified: "2d ago".to_string(),
                },
                DirEntry {
                    name: "a.omaframe.json".to_string(),
                    is_dir: false,
                    size: 2048,
                    modified: "5m ago".to_string(),
                },
            ],
            selected: Some(1),
            filename: "new.omaframe.json".to_string(),
            sidebar: vec![
                ("Home".to_string(), "D".to_string(), PathBuf::from("/home/u")),
                ("Root".to_string(), "D".to_string(), PathBuf::from("/")),
            ],
        }
    }

    #[test]
    fn dialog_layout_geometry_and_hits() {
        let area = ratatui::layout::Rect::new(0, 0, 80, 24);
        let l = dialog_layout(area).expect("80x24 fits a dialog");
        // Centered 72×22: origin (4, 1).
        assert_eq!(
            (l.outer.x, l.outer.y, l.outer.width, l.outer.height),
            (4, 1, 72, 22)
        );
        assert_eq!((l.cancel.x, l.cancel.y, l.cancel.width), (7, 1, 6));
        assert_eq!((l.folder.x, l.folder.y, l.folder.width), (36, 1, 7));
        assert_eq!((l.save.x, l.save.y, l.save.width), (69, 1, 4));
        assert_eq!(
            (l.path_row.x, l.path_row.y, l.path_row.width, l.path_row.height),
            (5, 2, 70, 1)
        );
        assert_eq!(
            (
                l.sidebar.x,
                l.sidebar.y,
                l.sidebar.width,
                l.sidebar.height
            ),
            (5, 4, 16, 18)
        );
        assert_eq!(
            (l.list.x, l.list.y, l.list.width, l.list.height),
            (22, 4, 53, 18)
        );
        // Buttons.
        assert_eq!(hit_dialog_button(&l, 7, 1), Some(DialogButton::Cancel));
        assert_eq!(hit_dialog_button(&l, 12, 1), Some(DialogButton::Cancel));
        assert_eq!(hit_dialog_button(&l, 36, 1), Some(DialogButton::Folder));
        assert_eq!(hit_dialog_button(&l, 69, 1), Some(DialogButton::Save));
        assert_eq!(hit_dialog_button(&l, 72, 1), Some(DialogButton::Save));
        assert_eq!(hit_dialog_button(&l, 20, 1), None);
        assert_eq!(hit_dialog_button(&l, 7, 2), None);
        // Sidebar + list (fake FileDialog struct literal — fields are pub).
        let dlg = fake_dialog(DialogPurpose::SaveAs);
        assert_eq!(hit_dialog_sidebar(&l, &dlg, 5, 4), Some(0));
        assert_eq!(hit_dialog_sidebar(&l, &dlg, 5, 5), Some(1));
        assert_eq!(hit_dialog_sidebar(&l, &dlg, 5, 6), None);
        assert_eq!(hit_dialog_sidebar(&l, &dlg, 22, 4), None);
        assert_eq!(hit_dialog_list(&l, &dlg, 22, 4), Some(0));
        assert_eq!(hit_dialog_list(&l, &dlg, 22, 5), Some(1));
        assert_eq!(hit_dialog_list(&l, &dlg, 22, 6), None);
        assert_eq!(hit_dialog_list(&l, &dlg, 5, 4), None);
        // Too small → None.
        assert!(dialog_layout(ratatui::layout::Rect::new(0, 0, 59, 24)).is_none());
        assert!(dialog_layout(ratatui::layout::Rect::new(0, 0, 80, 15)).is_none());
        // Title text places the buttons at exactly the hit rects.
        let text = dialog_title_text(72, "Save");
        assert_eq!(text.chars().count(), 72);
        let chars: Vec<char> = text.chars().collect();
        let s: String = chars[3..9].iter().collect();
        assert_eq!(s, "Cancel");
        let s: String = chars[36 - 4..36 - 4 + 7].iter().collect();
        assert_eq!(s, "+Folder");
        let s: String = chars[69 - 4..69 - 4 + 4].iter().collect();
        assert_eq!(s, "Save");
    }

    #[test]
    fn dialog_snapshot_contains_chrome() {
        let (mut app, t) = harness();
        app.file_dialog = Some(fake_dialog(DialogPurpose::SaveAs));
        let screen = screen_text(&render_to_buf(&mut app, &t));
        for needle in [
            "Cancel",
            "Save",
            "+Folder",
            "Places",
            "Name",
            "new.omaframe.json",
            "a.omaframe.json",
            "docs",
            "Home",
        ] {
            assert!(screen.contains(needle), "missing {needle:?}:\n{screen}");
        }
        // Load purpose labels the confirm button Open and shows the selected
        // name in the path row instead of the filename box.
        app.file_dialog = Some(fake_dialog(DialogPurpose::Load));
        let screen = screen_text(&render_to_buf(&mut app, &t));
        assert!(screen.contains("Open"), "missing Open:\n{screen}");
        assert!(
            screen.contains("a.omaframe.json"),
            "missing selected name:\n{screen}"
        );
    }
}
