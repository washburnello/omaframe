//! Interactive TUI application state (tools, gestures, palette, file).
//!
//! Programs against the sibling-owned APIs `omaframe::draw`,
//! `omaframe::theme` and `omaframe::clipboard` (re-exported here as
//! `app::draw` / `app::theme` / `app::clipboard` so `ui.rs` and `main.rs`
//! have a single import point). Tinting (`draw_*` placeholders → fg/bg pots)
//! and the [`box_style_next`]/[`box_style_label`] helpers live here because
//! the `draw` module intentionally carries no color/pot state.

use std::path::PathBuf;

use omaframe::draw;
use omaframe::model::{Cell, Document, History, Layer, Rect};
use unicode_width::UnicodeWidthStr;

/// Cycle light → heavy → double → rounded → ascii (`L` key).
pub fn box_style_next(s: draw::BoxStyle) -> draw::BoxStyle {
    match s {
        draw::BoxStyle::Light => draw::BoxStyle::Heavy,
        draw::BoxStyle::Heavy => draw::BoxStyle::Double,
        draw::BoxStyle::Double => draw::BoxStyle::Rounded,
        draw::BoxStyle::Rounded => draw::BoxStyle::Ascii,
        draw::BoxStyle::Ascii => draw::BoxStyle::Light,
    }
}

/// Short label for the status bar.
pub fn box_style_label(s: draw::BoxStyle) -> &'static str {
    match s {
        draw::BoxStyle::Light => "light",
        draw::BoxStyle::Heavy => "heavy",
        draw::BoxStyle::Double => "double",
        draw::BoxStyle::Rounded => "rounded",
        draw::BoxStyle::Ascii => "ascii",
    }
}

// ---------------------------------------------------------------------------
// Palette data (docs/palette-spec.md exact lists)
// ---------------------------------------------------------------------------

/// Palette tab titles in normative order.
pub const PALETTE_TABS: [&str; 7] = [
    "Letters",
    "Numbers",
    "Symbols",
    "Outlines",
    "Blocks",
    "Nerds",
    "Widgets",
];

/// Stable lowercase tab ids (persisted in config / per-file `paletteTab`).
pub const PALETTE_TAB_IDS: [&str; 7] = [
    "letters",
    "numbers",
    "symbols",
    "outlines",
    "blocks",
    "nerds",
    "widgets",
];

/// Map a stored `paletteTab` id (e.g. `"outlines"`) to a tab index.
pub fn palette_tab_index(id: &str) -> usize {
    let lower = id.to_ascii_lowercase();
    PALETTE_TAB_IDS
        .iter()
        .position(|t| *t == lower)
        .unwrap_or(3)
}

fn nerd_glyphs() -> Vec<String> {
    const NERD_SRC: &str = include_str!("../assets/nerd.txt");
    let mut out = Vec::new();
    for line in NERD_SRC.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some(first) = line.split_whitespace().next() {
            out.push(first.to_string());
        }
    }
    out
}

/// Exact v1 seed lists per `docs/palette-spec.md` §2. Widgets tab holds stamp
/// names (multi-char); every other tab holds single chars.
pub fn palette_chars(tab: usize) -> Vec<String> {
    match tab % 7 {
        0 => ('A'..='Z')
            .chain('a'..='z')
            .map(|c| c.to_string())
            .collect(),
        1 => ('0'..='9').map(|c| c.to_string()).collect(),
        2 => [
            '!', '"', '#', '$', '%', '&', '\'', '(', ')', '*', '+', ',', '-', '.',
            '/', ':', ';', '<', '=', '>', '?', '@', '[', '\\', ']', '^', '_', '`',
            '{', '|', '}', '~',
        ]
        .iter()
        .map(|c| c.to_string())
        .collect(),
        3 => [
            "─", "│", "┌", "┐", "└", "┘", "├", "┤", "┬", "┴", "┼", "╭", "╮",
            "╯", "╰", "━", "┃", "═", "║", "╔", "╗", "╱", "╲", "╳", "+", "-",
            "|",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect(),
        4 => [
            "█", "▓", "▒", "░", "▀", "▄", "▌", "▐", "▖", "▗", "▘", "▝", "▚",
            "▞", "▟", "▁", "▂", "▃", "▅", "▆", "▇",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect(),
        5 => nerd_glyphs(),
        _ => [
            "button",
            "button-focused",
            "input",
            "dropdown",
            "radio-on",
            "radio-off",
            "checkbox-on",
            "checkbox-off",
            "toggle",
            "close",
            "scrollbar",
            "progress",
            "divider",
            "panel",
            "tabs",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect(),
    }
}

// ---------------------------------------------------------------------------
// Tools + App state
// ---------------------------------------------------------------------------

/// The 9 tools on the left rail (plan §4.1).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Tool {
    Pencil,
    Box,
    Line,
    RoundedRect,
    Oval,
    Text,
    Eraser,
    Grab,
    Select,
}

impl Tool {
    pub const ALL: [Tool; 9] = [
        Tool::Pencil,
        Tool::Box,
        Tool::Line,
        Tool::RoundedRect,
        Tool::Oval,
        Tool::Text,
        Tool::Eraser,
        Tool::Grab,
        Tool::Select,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Tool::Pencil => "pencil",
            Tool::Box => "box",
            Tool::Line => "line",
            Tool::RoundedRect => "rrect",
            Tool::Oval => "oval",
            Tool::Text => "text",
            Tool::Eraser => "erase",
            Tool::Grab => "grab",
            Tool::Select => "select",
        }
    }

    pub fn shortcut(self) -> char {
        match self {
            Tool::Pencil => 'p',
            Tool::Box => 'b',
            Tool::Line => 'l',
            Tool::RoundedRect => 'r',
            Tool::Oval => 'o',
            Tool::Text => 't',
            Tool::Eraser => 'e',
            Tool::Grab => 'g',
            Tool::Select => 'v',
        }
    }

    pub fn from_shortcut(c: char) -> Option<Tool> {
        match c.to_ascii_lowercase() {
            'p' => Some(Tool::Pencil),
            'b' => Some(Tool::Box),
            'l' => Some(Tool::Line),
            'r' => Some(Tool::RoundedRect),
            'o' => Some(Tool::Oval),
            't' => Some(Tool::Text),
            'e' => Some(Tool::Eraser),
            'g' => Some(Tool::Grab),
            'v' | 's' => Some(Tool::Select),
            _ => None,
        }
    }
}

/// Interactive state. `doc` + `history` are the source of truth; `scratch`
/// is the live gesture preview rendered above the stack and committed on
/// release (tools-spec §0).
pub struct App {
    pub doc: Document,
    pub history: History,
    pub scratch: Layer,
    pub tool: Tool,
    pub ch: String,
    pub fg: i8,
    pub bg: i8,
    pub box_style: draw::BoxStyle,
    pub arrow: bool,
    pub cursor: (i32, i32),
    pub viewport: (i32, i32),
    pub palette_tab: usize,
    pub palette_scroll: usize,
    pub show_rulers: bool,
    pub preview_dark: bool,
    pub status_msg: String,
    pub text_buffer: String,
    // --- session state (not in the brief's field list, but required) ---
    pub file_path: Option<PathBuf>,
    pub dirty: bool,
    pub should_quit: bool,
    anchor: Option<(i32, i32)>,
    drawing: bool,
    text_start: Option<(i32, i32)>,
}

impl App {
    pub fn new(doc: Document, file_path: Option<PathBuf>) -> Self {
        let palette_tab = palette_tab_index(&doc.palette_tab);
        let preview_dark = doc.preview_dark;
        Self {
            doc,
            history: History::new(),
            scratch: Layer::new(),
            tool: Tool::Pencil,
            ch: "─".to_string(),
            fg: 7,
            bg: -1,
            box_style: draw::BoxStyle::Light,
            arrow: false,
            cursor: (0, 0),
            viewport: (0, 0),
            palette_tab,
            palette_scroll: 0,
            show_rulers: true,
            preview_dark,
            status_msg: "click-drag to draw · right-click grabs · Ctrl-S saves · Ctrl-K commands"
                .to_string(),
            text_buffer: String::new(),
            file_path,
            dirty: false,
            should_quit: false,
            anchor: None,
            drawing: false,
            text_start: None,
        }
    }

    pub fn set_status(&mut self, msg: impl Into<String>) {
        self.status_msg = msg.into();
    }

    pub fn file_label(&self) -> String {
        match &self.file_path {
            Some(p) => format!(
                "{}{}",
                p.file_name()
                    .and_then(|s| s.to_str())
                    .unwrap_or("untitled"),
                if self.dirty { "*" } else { "" }
            ),
            None => format!("untitled{}", if self.dirty { "*" } else { "" }),
        }
    }

    // --- cursor / viewport ---

    /// Clamp the cursor into the grid and off continuation guards.
    pub fn clamp_cursor(&mut self) {
        let (w, h) = (self.doc.grid.0 as i32, self.doc.grid.1 as i32);
        self.cursor.0 = self.cursor.0.clamp(0, (w - 1).max(0));
        self.cursor.1 = self.cursor.1.clamp(0, (h - 1).max(0));
        if self.doc.is_continuation(self.cursor.0, self.cursor.1) && self.cursor.0 > 0 {
            self.cursor.0 -= 1;
        }
    }

    /// Wide-aware step (horizontal skips continuation guards via the model).
    pub fn move_cursor(&mut self, dx: i32, dy: i32) {
        if dx != 0 {
            self.cursor.0 = self.doc.cursor_step_x(self.cursor.0, self.cursor.1, dx);
        }
        if dy != 0 {
            self.cursor.1 += dy;
        }
        self.clamp_cursor();
    }

    /// Scroll the viewport just enough to keep the cursor visible.
    pub fn ensure_cursor_visible(&mut self, canvas_w: i32, canvas_h: i32) {
        if canvas_w <= 0 || canvas_h <= 0 {
            return;
        }
        let (cx, cy) = self.cursor;
        let (ox, oy) = self.viewport;
        let mut nox = ox;
        let mut noy = oy;
        if cx < ox {
            nox = cx;
        } else if cx >= ox + canvas_w {
            nox = cx - canvas_w + 1;
        }
        if cy < oy {
            noy = cy;
        } else if cy >= oy + canvas_h {
            noy = cy - canvas_h + 1;
        }
        let max_ox = (self.doc.grid.0 as i32 - canvas_w).max(0);
        let max_oy = (self.doc.grid.1 as i32 - canvas_h).max(0);
        self.viewport = (nox.clamp(0, max_ox), noy.clamp(0, max_oy));
    }

    // --- palette / pots / chrome toggles ---

    pub fn cycle_palette_tab(&mut self, dir: i32) {
        let n = PALETTE_TABS.len() as i32;
        self.palette_tab = (self.palette_tab as i32 + dir).rem_euclid(n) as usize;
        self.palette_scroll = 0;
        self.doc.palette_tab = PALETTE_TAB_IDS[self.palette_tab].to_string();
        self.set_status(format!("palette: {}", PALETTE_TABS[self.palette_tab]));
    }

    pub fn set_palette_tab(&mut self, idx: usize) {
        if idx < PALETTE_TABS.len() {
            self.palette_tab = idx;
            self.palette_scroll = 0;
            self.doc.palette_tab = PALETTE_TAB_IDS[idx].to_string();
        }
    }

    pub fn scroll_palette(&mut self, dir: i32) {
        let next = self.palette_scroll as i32 + dir;
        self.palette_scroll = next.max(0) as usize;
    }

    pub fn cycle_box_style(&mut self) {
        self.box_style = box_style_next(self.box_style);
        self.set_status(format!("box style: {}", box_style_label(self.box_style)));
    }

    pub fn toggle_arrow(&mut self) {
        self.arrow = !self.arrow;
        self.set_status(if self.arrow {
            "arrowheads: on"
        } else {
            "arrowheads: off"
        });
    }

    pub fn toggle_preview(&mut self) {
        self.preview_dark = !self.preview_dark;
        self.doc.preview_dark = self.preview_dark;
        self.set_status(if self.preview_dark {
            "preview: dark"
        } else {
            "preview: light"
        });
    }

    pub fn toggle_rulers(&mut self) {
        self.show_rulers = !self.show_rulers;
    }

    pub fn cycle_fg(&mut self) {
        self.fg = (self.fg + 1).rem_euclid(16);
        self.set_status(format!("fg: {}", self.fg));
    }

    pub fn cycle_bg(&mut self) {
        self.bg = if self.bg < 0 {
            0
        } else if self.bg >= 15 {
            -1
        } else {
            self.bg + 1
        };
        self.set_status(format!(
            "bg: {}",
            if self.bg < 0 {
                "-".to_string()
            } else {
                self.bg.to_string()
            }
        ));
    }

    pub fn set_tool(&mut self, t: Tool) {
        if self.tool == t {
            return;
        }
        // tools-spec §6 cleanup: switching away from text commits.
        if self.tool == Tool::Text && self.text_start.is_some() {
            self.commit_text();
        } else if self.drawing {
            self.scratch.clear();
            self.anchor = None;
            self.drawing = false;
        }
        if self.tool == Tool::Select && t != Tool::Select {
            self.history.clear_selection();
        }
        self.tool = t;
        self.set_status(format!("tool: {}", t.label()));
    }

    pub fn set_active_layer(&mut self, idx: usize) {
        if self.doc.set_active(idx) {
            self.set_status(format!("layer: {}", self.doc.layers[idx].name));
        }
    }

    pub fn cycle_layer(&mut self, dir: i32) {
        let n = self.doc.layers.len() as i32;
        if n == 0 {
            return;
        }
        let next = (self.doc.active as i32 + dir).rem_euclid(n) as usize;
        self.set_active_layer(next);
    }

    pub fn toggle_layer_visible(&mut self, idx: usize) {
        let (name, visible) = match self.doc.layers.get_mut(idx) {
            Some(nl) => {
                nl.visible = !nl.visible;
                (nl.name.clone(), nl.visible)
            }
            None => return,
        };
        self.set_status(format!(
            "layer {} {}",
            name,
            if visible { "shown" } else { "hidden" }
        ));
    }

    // --- undo / save ---

    pub fn undo(&mut self) {
        if self.history.undo(&mut self.doc) {
            self.dirty = true;
            self.set_status("undo");
        } else {
            self.set_status("nothing to undo");
        }
    }

    pub fn redo(&mut self) {
        if self.history.redo(&mut self.doc) {
            self.dirty = true;
            self.set_status("redo");
        } else {
            self.set_status("nothing to redo");
        }
    }

    pub fn save(&mut self) -> bool {
        let Some(path) = self.file_path.clone() else {
            self.set_status("no file path — run as `omaframe [FILE]`");
            return false;
        };
        match omaframe::model::save_file(&self.doc, &path) {
            Ok(()) => {
                self.dirty = false;
                self.set_status(format!("saved {}", path.display()));
                true
            }
            Err(e) => {
                self.set_status(format!("save failed: {e}"));
                false
            }
        }
    }

    pub fn autosave(&mut self) {
        if self.dirty && self.file_path.is_some() {
            let _ = self.save();
        }
    }

    // --- shift-constrain helpers (pure, unit-tested) ---

    /// Square constraint, anchor corner fixed (tools-spec §2): expand the
    /// shorter axis away from the anchor.
    pub fn constrain_square_anchor(
        anchor: (i32, i32),
        cur: (i32, i32),
    ) -> (i32, i32) {
        let dx = cur.0 - anchor.0;
        let dy = cur.1 - anchor.1;
        let side = dx.abs().max(dy.abs());
        let nx = anchor.0 + side * dx.signum();
        let ny = anchor.1 + side * dy.signum();
        (nx, ny)
    }

    /// Axis-only line constraint (decisions.md Q2): dominant axis wins.
    pub fn constrain_line_axis(anchor: (i32, i32), cur: (i32, i32)) -> (i32, i32) {
        let dx = (cur.0 - anchor.0).abs();
        let dy = (cur.1 - anchor.1).abs();
        if dx >= dy {
            (cur.0, anchor.1)
        } else {
            (anchor.0, cur.1)
        }
    }

    fn resolve_guard(&self, x: i32, y: i32) -> (i32, i32) {
        if x > 0 && self.doc.is_continuation(x, y) {
            (x - 1, y)
        } else {
            (x, y)
        }
    }

    /// Recolor a `draw_*` patch (placeholder `DRAW_FG`/`DRAW_BG`) to the
    /// fg/bg pots; transparent markers pass through as erasures.
    fn tint(&self, mut layer: Layer) -> Layer {
        if layer.is_empty() {
            return layer;
        }
        // Collect first (apply order is deterministic anyway).
        let entries: Vec<((i32, i32), Cell)> = layer.entries().collect();
        layer.clear();
        let mut tinted = Layer::new();
        for ((x, y), c) in entries {
            if c.is_transparent() {
                tinted.set(x, y, Cell::erased());
            } else {
                tinted.set(x, y, Cell::new(c.ch, self.fg, self.bg));
            }
        }
        tinted
    }

    fn arrowhead_for(x0: i32, y0: i32, x1: i32, y1: i32, horizontal_first: bool) -> &'static str {
        if x0 == x1 {
            if y1 < y0 {
                "▲"
            } else {
                "▼"
            }
        } else if y0 == y1 {
            if x1 < x0 { "◄" } else { "►" }
        } else if horizontal_first {
            if y1 < y0 { "▲" } else { "▼" }
        } else if x1 < x0 {
            "◄"
        } else {
            "►"
        }
    }

    // --- gestures ---

    /// Begin a left-drag gesture at a document cell.
    pub fn start_stroke(&mut self, x: i32, y: i32, _shift: bool) {
        let (x, y) = self.resolve_guard(x, y);
        self.cursor = (x, y);
        self.clamp_cursor();
        match self.tool {
            Tool::Grab => {
                self.grab_at(x, y);
            }
            Tool::Text => {
                self.scratch.clear();
                self.text_buffer.clear();
                self.text_start = Some(self.cursor);
                self.anchor = None;
                self.drawing = false;
                self.set_status("text: type, Enter commits, Esc commits, Shift+Enter newline");
            }
            Tool::Select => {
                self.anchor = Some((x, y));
                self.drawing = true;
                self.scratch.clear();
            }
            _ => {
                self.anchor = Some((x, y));
                self.drawing = true;
                self.scratch.clear();
                // Paint the anchor immediately so single clicks leave a mark
                // (pencil dot, line dot, eraser tick). Shape previews that
                // are legitimately empty (1×1 box) stay empty.
                self.update_stroke(x, y, _shift);
            }
        }
    }

    /// Extend the live gesture. Rebuilds shape previews purely from
    /// anchor+cursor; pencil dabs accumulate (tools-spec §0/§1).
    pub fn update_stroke(&mut self, x: i32, y: i32, shift: bool) {
        let (x, y) = self.resolve_guard(x, y);
        self.cursor = (x, y);
        // Note: no clamp here — drawing off-grid is legal; the viewport
        // clips at render. The keyboard cursor stays clamped via
        // move_cursor/clamp_cursor.
        let Some(a) = self.anchor else {
            return;
        };
        if !self.drawing {
            return;
        }
        match self.tool {
            Tool::Pencil => {
                let dab = draw::paint_cells(&[(x, y)], &self.ch, self.fg, self.bg);
                self.scratch.set_from(&dab);
            }
            Tool::Eraser => {
                // Filled-rect erase (tools-spec §7), rebuilt idempotently.
                let r = Rect::from_points(a.0, a.1, x, y);
                let mut patch = Layer::new();
                for yy in r.y..r.y + r.h as i32 {
                    for xx in r.x..r.x + r.w as i32 {
                        patch.set(xx, yy, Cell::erased());
                    }
                }
                self.scratch = patch;
            }
            Tool::Box => {
                let c = if shift {
                    Self::constrain_square_anchor(a, (x, y))
                } else {
                    (x, y)
                };
                let r = Rect::from_points(a.0, a.1, c.0, c.1);
                let mut patch = self.tint(draw::draw_box(r, self.box_style));
                let idx = self.doc.active;
                draw::snap_patch(&self.doc, idx, &mut patch);
                self.scratch = patch;
            }
            Tool::RoundedRect => {
                let c = if shift {
                    Self::constrain_square_anchor(a, (x, y))
                } else {
                    (x, y)
                };
                let r = Rect::from_points(a.0, a.1, c.0, c.1);
                let mut patch = self.tint(draw::draw_box(r, draw::BoxStyle::Rounded));
                let idx = self.doc.active;
                draw::snap_patch(&self.doc, idx, &mut patch);
                self.scratch = patch;
            }
            Tool::Line => {
                let c = if shift {
                    // decisions.md Q2: shift = axis-only.
                    Self::constrain_line_axis(a, (x, y))
                } else {
                    (x, y)
                };
                // NOTE: full cellContext axis heuristic (tools-spec §3.2) is
                // deferred; vertical-first default matches the no-context case.
                let horizontal_first = false;
                let mut patch = self.tint(draw::draw_line(a.0, a.1, c.0, c.1, horizontal_first));
                if self.arrow {
                    let head = Self::arrowhead_for(a.0, a.1, c.0, c.1, horizontal_first);
                    patch.set(c.0, c.1, Cell::new(head, self.fg, self.bg));
                }
                let idx = self.doc.active;
                draw::snap_patch(&self.doc, idx, &mut patch);
                self.scratch = patch;
            }
            Tool::Oval => {
                let c = if shift {
                    Self::constrain_square_anchor(a, (x, y))
                } else {
                    (x, y)
                };
                let r = Rect::from_points(a.0, a.1, c.0, c.1);
                // Ovals never snap (tools-spec §5).
                self.scratch = self.tint(draw::draw_ellipse(r));
            }
            Tool::Select => {
                let r = Rect::from_points(a.0, a.1, x, y);
                self.history.set_selection(Some(r));
                self.set_status(format!("select {}x{}", r.w, r.h));
            }
            Tool::Text | Tool::Grab => {}
        }
    }

    /// Release: commit one undo entry (or nothing for no-ops).
    pub fn end_stroke(&mut self) {
        if !self.drawing {
            return;
        }
        match self.tool {
            Tool::Pencil | Tool::Eraser | Tool::Box | Tool::RoundedRect | Tool::Line
            | Tool::Oval => {
                if self.scratch.is_empty() {
                    self.anchor = None;
                    self.drawing = false;
                    return;
                }
                let patch = std::mem::take(&mut self.scratch);
                if self.history.commit(&mut self.doc, &patch) {
                    self.dirty = true;
                }
                self.anchor = None;
                self.drawing = false;
            }
            Tool::Select => {
                // Click without drag clears; rubber-band keeps the rect and
                // commits nothing (tools-spec §11).
                if let Some(a) = self.anchor {
                    if a == self.cursor {
                        self.history.clear_selection();
                        self.set_status("select: cleared");
                    }
                }
                self.anchor = None;
                self.drawing = false;
            }
            Tool::Text | Tool::Grab => {
                self.anchor = None;
                self.drawing = false;
            }
        }
    }

    /// Esc: cancel a shape gesture, commit pending text (decisions.md),
    /// else clear the selection.
    pub fn cancel_stroke(&mut self) {
        if self.text_start.is_some() {
            self.commit_text();
            return;
        }
        if self.drawing {
            self.scratch.clear();
            self.anchor = None;
            self.drawing = false;
            self.set_status("cancelled");
            return;
        }
        if self.history.selection().is_some() {
            self.history.clear_selection();
            self.set_status("select: cleared");
        }
    }

    /// Eyedropper (decisions.md Q4): copy ch+fg+bg into the pencil.
    pub fn grab_at(&mut self, x: i32, y: i32) -> bool {
        let (x, y) = self.resolve_guard(x, y);
        match self.doc.cell(x, y) {
            Some(c) => {
                self.ch = c.ch.clone();
                self.fg = c.fg;
                self.bg = c.bg;
                if self.tool != Tool::Pencil {
                    self.tool = Tool::Pencil;
                }
                self.set_status(format!(
                    "grabbed '{}' fg {} bg {}",
                    self.ch,
                    self.fg,
                    if self.bg < 0 {
                        "-".to_string()
                    } else {
                        self.bg.to_string()
                    }
                ));
                true
            }
            None => {
                self.set_status("grab: empty");
                false
            }
        }
    }

    // --- text tool (tools-spec §6, decisions.md Q5) ---

    pub fn type_char(&mut self, c: char) {
        if self.tool != Tool::Text {
            return;
        }
        if c.is_control() {
            return;
        }
        if self.text_start.is_none() {
            self.text_start = Some(self.cursor);
            self.text_buffer.clear();
        }
        let s = c.to_string();
        if s.width() == 0 {
            return;
        }
        let dab = draw::paint_cells(&[(self.cursor.0, self.cursor.1)], &s, self.fg, self.bg);
        self.scratch.set_from(&dab);
        self.text_buffer.push(c);
        self.cursor.0 += s.width().max(1) as i32;
        self.clamp_cursor();
    }

    pub fn text_newline(&mut self) {
        if self.tool != Tool::Text || self.text_start.is_none() {
            return;
        }
        let sx = self.text_start.unwrap().0;
        self.cursor.1 += 1;
        self.cursor.0 = sx;
        self.clamp_cursor();
        self.text_buffer.push('\n');
    }

    /// Backspace clamps at the session left edge and never eats
    /// pre-existing drawing (decisions.md Q5).
    pub fn text_backspace(&mut self) {
        let Some(start) = self.text_start else {
            return;
        };
        if self.cursor.0 <= start.0 {
            self.set_status("text: at session start");
            return;
        }
        self.cursor.0 = self
            .doc
            .cursor_step_x(self.cursor.0, self.cursor.1, -1);
        if self.cursor.0 < start.0 {
            self.cursor.0 = start.0;
        }
        self.scratch
            .set(self.cursor.0, self.cursor.1, Cell::erased());
        self.text_buffer.pop();
    }

    /// Delete erases at the cursor without moving (tools-spec §6).
    pub fn text_delete(&mut self) {
        if self.text_start.is_none() {
            return;
        }
        self.scratch
            .set(self.cursor.0, self.cursor.1, Cell::erased());
    }

    /// Enter and Esc both commit (decisions.md); the whole session is one
    /// undo entry.
    pub fn commit_text(&mut self) {
        if self.text_start.is_none() && self.scratch.is_empty() {
            return;
        }
        self.text_start = None;
        self.text_buffer.clear();
        if self.scratch.is_empty() {
            return;
        }
        let patch = std::mem::take(&mut self.scratch);
        if self.history.commit(&mut self.doc, &patch) {
            self.dirty = true;
            self.set_status("text committed");
        }
    }

    // --- erase / selection cut ---

    /// Delete clears the live selection if any, else one cell at the cursor.
    pub fn delete_at_cursor(&mut self) {
        if self.tool == Tool::Text && self.text_start.is_some() {
            self.text_delete();
            return;
        }
        if let Some(r) = self.history.selection() {
            let mut patch = Layer::new();
            for y in r.y..r.y + r.h as i32 {
                for x in r.x..r.x + r.w as i32 {
                    patch.set(x, y, Cell::erased());
                }
            }
            if self.history.commit(&mut self.doc, &patch) {
                self.dirty = true;
                self.set_status("selection cut");
            }
            self.history.clear_selection();
            return;
        }
        let mut patch = Layer::new();
        patch.set(self.cursor.0, self.cursor.1, Cell::erased());
        if self.history.commit(&mut self.doc, &patch) {
            self.dirty = true;
            self.set_status("cleared 1 cell");
        }
    }

    /// Palette click: load the char into the pencil (pots unchanged).
    pub fn pick_palette_char(&mut self, s: &str) {
        self.ch = s.to_string();
        if self.tool != Tool::Pencil {
            self.tool = Tool::Pencil;
        }
        self.set_status(format!("ch '{s}' fg {} tab {}", self.fg, PALETTE_TABS[self.palette_tab]));
    }

    pub fn active_layer_name(&self) -> &str {
        self.doc
            .layers
            .get(self.doc.active)
            .map(|l| l.name.as_str())
            .unwrap_or("-")
    }

    pub fn is_text_session(&self) -> bool {
        self.text_start.is_some()
    }
}

// ---------------------------------------------------------------------------
// Tests (no TTY required)
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn test_doc() -> Document {
        Document::new("t", 80, 24)
    }

    #[test]
    fn square_constrain_expands_short_axis_from_anchor() {
        assert_eq!(App::constrain_square_anchor((2, 2), (5, 3)), (5, 5));
        assert_eq!(App::constrain_square_anchor((5, 5), (2, 3)), (2, 2));
        assert_eq!(App::constrain_square_anchor((0, 0), (0, 0)), (0, 0));
        assert_eq!(App::constrain_square_anchor((4, 4), (4, 9)), (4, 9));
        // Negative direction keeps the anchor fixed.
        assert_eq!(App::constrain_square_anchor((10, 10), (7, 9)), (7, 7));
    }

    #[test]
    fn line_constrain_picks_dominant_axis() {
        assert_eq!(App::constrain_line_axis((0, 0), (5, 2)), (5, 0));
        assert_eq!(App::constrain_line_axis((0, 0), (2, 5)), (0, 5));
        // Ties go horizontal.
        assert_eq!(App::constrain_line_axis((3, 3), (6, 6)), (6, 3));
        assert_eq!(App::constrain_line_axis((3, 3), (3, 3)), (3, 3));
    }

    #[test]
    fn cursor_clamps_to_grid_and_off_guards() {
        let mut app = App::new(test_doc(), None);
        app.cursor = (999, -5);
        app.clamp_cursor();
        assert_eq!(app.cursor, (79, 0));
        app.cursor = (-3, 99);
        app.clamp_cursor();
        assert_eq!(app.cursor, (0, 23));

        // Wide-char guard resolves to its anchor.
        let mut patch = Layer::new();
        patch.set(5, 5, Cell::new("漢", 1, -1));
        assert!(app.history.commit(&mut app.doc, &patch));
        app.cursor = (6, 5); // continuation guard
        app.clamp_cursor();
        assert_eq!(app.cursor, (5, 5));

        // Wide-aware stepping jumps over guards both ways.
        assert_eq!(app.doc.cursor_step_x(4, 5, 1), 5);
        assert_eq!(app.doc.cursor_step_x(5, 5, 1), 7);
        assert_eq!(app.doc.cursor_step_x(7, 5, -1), 5);
    }

    #[test]
    fn palette_tabs_cycle_and_wrap() {
        let mut app = App::new(test_doc(), None);
        assert_eq!(app.palette_tab, 3); // doc default "outlines"
        app.cycle_palette_tab(1);
        assert_eq!(app.palette_tab, 4);
        app.cycle_palette_tab(-1);
        assert_eq!(app.palette_tab, 3);
        app.set_palette_tab(6);
        app.cycle_palette_tab(1);
        assert_eq!(app.palette_tab, 0);
        app.cycle_palette_tab(-1);
        assert_eq!(app.palette_tab, 6);
        // Exact seed counts per docs/palette-spec.md.
        assert_eq!(palette_chars(0).len(), 52);
        assert_eq!(palette_chars(1).len(), 10);
        assert_eq!(palette_chars(2).len(), 32);
        assert_eq!(palette_chars(3).len(), 27);
        assert_eq!(palette_chars(4).len(), 21);
        assert!(!palette_chars(5).is_empty()); // nerds from assets/nerd.txt
    }

    #[test]
    fn text_backspace_clamps_at_session_start() {
        let mut app = App::new(test_doc(), None);
        app.set_tool(Tool::Text);
        app.start_stroke(5, 5, false);
        app.type_char('h');
        app.type_char('i');
        assert_eq!(app.cursor.0, 7);
        app.text_backspace();
        assert_eq!(app.cursor.0, 6);
        app.text_backspace();
        assert_eq!(app.cursor.0, 5);
        // Clamped: further backspaces stay at the session left edge.
        app.text_backspace();
        assert_eq!(app.cursor.0, 5);
        app.text_backspace();
        assert_eq!(app.cursor.0, 5);
        // Enter commits the session as a single gesture.
        let undo_before = app.history.undo_len();
        app.commit_text();
        assert!(!app.is_text_session());
        assert!(app.history.undo_len() >= undo_before);
    }

    #[test]
    fn pencil_drag_commits_one_undo_entry_and_undo_clears_it() {
        let mut app = App::new(test_doc(), None);
        app.start_stroke(1, 1, false);
        app.update_stroke(2, 1, false);
        app.update_stroke(3, 1, false);
        assert!(!app.scratch.is_empty());
        app.end_stroke();
        assert!(app.scratch.is_empty());
        assert_eq!(app.history.undo_len(), 1);
        assert!(app.doc.cell(1, 1).is_some());
        assert!(app.doc.cell(3, 1).is_some());
        app.undo();
        assert_eq!(app.doc.cell(1, 1), None);
        assert_eq!(app.doc.cell(3, 1), None);
        app.redo();
        assert!(app.doc.cell(2, 1).is_some());
    }

    #[test]
    fn grab_copies_cell_into_pencil() {
        let mut app = App::new(test_doc(), None);
        let mut patch = Layer::new();
        patch.set(4, 4, Cell::new("Z", 9, 2));
        assert!(app.history.commit(&mut app.doc, &patch));
        assert!(app.grab_at(4, 4));
        assert_eq!(app.ch, "Z");
        assert_eq!(app.fg, 9);
        assert_eq!(app.bg, 2);
        assert_eq!(app.tool, Tool::Pencil);
        assert!(!app.grab_at(70, 20)); // empty: changes nothing
        assert_eq!(app.ch, "Z");
    }
}
