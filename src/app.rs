//! Interactive TUI application state (tools, gestures, palette, file).
//!
//! Programs against the sibling-owned APIs `omaframe::draw`,
//! `omaframe::theme` and `omaframe::clipboard` (re-exported here as
//! `app::draw` / `app::theme` / `app::clipboard` so `ui.rs` and `main.rs`
//! have a single import point). Tinting (`draw_*` placeholders → fg/bg pots)
//! and the [`box_style_next`]/[`box_style_label`] helpers live here because
//! the `draw` module intentionally carries no color/pot state.
//!
//! Paint pots are truecolor-capable: [`App::fg`] is a [`PaintColor`]
//! (default `Ansi(7)`), [`App::bg`] is an optional [`PaintColor`] (`None` =
//! transparent, shown as `"-"` in status labels).
//!
//! File picking goes through the [`FileDialog`] state machine (no inline
//! path prompts anywhere): `ui.rs` renders `app.file_dialog` when `Some`,
//! and `main.rs` forwards dialog keys/mouse to the `dialog_*` delegates.

use std::path::PathBuf;
use std::time::SystemTime;

use omaframe::chars;
use omaframe::draw;
use omaframe::model::{Cell, Document, History, Layer, PaintColor, Rect};
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

/// Palette tab titles in normative order (wireframe labels).
pub const PALETTE_TABS: [&str; 7] = [
    "Letters",
    "Numbers",
    "Symbols",
    "Outline",
    "Blocks",
    "Glyphs",
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

/// Palette contents per tab. Letters/numbers/nerds/widgets are seeded here;
/// symbols/outlines/blocks come from the sibling `omaframe::chars` module
/// (single chars; widgets tab holds multi-char stamp names).
pub fn palette_chars(tab: usize) -> Vec<String> {
    match tab % 7 {
        0 => ('A'..='Z')
            .chain('a'..='z')
            .map(|c| c.to_string())
            .collect(),
        1 => ('0'..='9').map(|c| c.to_string()).collect(),
        2 => chars::symbols_tab(),
        3 => chars::outlines_tab(),
        4 => chars::blocks_tab(),
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

/// The 11 tools on the left rail: 10 draw tools + Pan.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Tool {
    Pencil,
    Box,
    Line,
    RoundedRect,
    Rect,
    Oval,
    Text,
    Eraser,
    Grab,
    Select,
    Pan,
}

impl Tool {
    pub const ALL: [Tool; 11] = [
        Tool::Pencil,
        Tool::Box,
        Tool::Line,
        Tool::RoundedRect,
        Tool::Rect,
        Tool::Oval,
        Tool::Text,
        Tool::Eraser,
        Tool::Grab,
        Tool::Select,
        Tool::Pan,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Tool::Pencil => "pencil",
            Tool::Box => "box",
            Tool::Line => "line",
            Tool::RoundedRect => "rrect",
            Tool::Rect => "rect",
            Tool::Oval => "oval",
            Tool::Text => "text",
            Tool::Eraser => "erase",
            Tool::Grab => "grab",
            Tool::Select => "select",
            Tool::Pan => "pan",
        }
    }

    pub fn shortcut(self) -> char {
        match self {
            Tool::Pencil => 'p',
            Tool::Box => 'b',
            Tool::Line => 'l',
            Tool::RoundedRect => 'r',
            Tool::Rect => 'd',
            Tool::Oval => 'o',
            Tool::Text => 't',
            Tool::Eraser => 'e',
            Tool::Grab => 'g',
            Tool::Select => 'v',
            Tool::Pan => '_',
        }
    }

    pub fn from_shortcut(c: char) -> Option<Tool> {
        match c.to_ascii_lowercase() {
            'p' => Some(Tool::Pencil),
            'b' => Some(Tool::Box),
            'l' => Some(Tool::Line),
            'r' => Some(Tool::RoundedRect),
            'd' => Some(Tool::Rect),
            'o' => Some(Tool::Oval),
            't' => Some(Tool::Text),
            'e' => Some(Tool::Eraser),
            'g' => Some(Tool::Grab),
            'v' | 's' => Some(Tool::Select),
            '_' => Some(Tool::Pan),
            _ => None,
        }
    }
}

// ---------------------------------------------------------------------------
// File dialog (modal Open / Save picker; no inline path prompts)
// ---------------------------------------------------------------------------

/// What the dialog returns on confirm: a file to open, or a folder+name to
/// save under.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum DialogMode {
    Open,
    Save,
}

/// Why the dialog was opened: loading replaces the document, SaveNew starts
/// a fresh bound document, SaveAs re-binds the current document.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum DialogPurpose {
    Load,
    SaveNew,
    SaveAs,
}

/// One row in the dialog file list.
pub struct DirEntry {
    pub name: String,
    pub is_dir: bool,
    pub size: u64,
    pub modified: String,
}

/// Modal file picker state. `ui.rs` renders it whenever
/// `App::file_dialog` is `Some`; `main.rs` forwards dialog keys/mouse to
/// the `App::dialog_*` delegates and commits/cancels via
/// [`App::dialog_confirm`] / [`App::dialog_cancel`].
pub struct FileDialog {
    pub mode: DialogMode,
    pub purpose: DialogPurpose,
    pub cwd: PathBuf,
    pub entries: Vec<DirEntry>,
    pub selected: Option<usize>,
    pub filename: String,
    /// `(label, icon, path)` sidebar shortcuts: existing dirs among
    /// Home, Documents, Downloads, Music, Pictures, Videos, plus Root.
    pub sidebar: Vec<(String, String, PathBuf)>,
}

/// `(folder, file)` nerd glyphs for the dialog rows, parsed from
/// `assets/nerd.txt` (first entries whose name contains "folder"/"file").
/// ASCII fallbacks (`"D"`/`"F"`) when the parse fails.
pub fn fs_glyphs() -> (String, String) {
    const NERD_SRC: &str = include_str!("../assets/nerd.txt");
    let mut folder: Option<String> = None;
    let mut file: Option<String> = None;
    for line in NERD_SRC.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let mut parts = line.split_whitespace();
        let (Some(glyph), Some(name)) = (parts.next(), parts.next()) else {
            continue;
        };
        let lname = name.to_ascii_lowercase();
        if folder.is_none() && lname.contains("folder") {
            folder = Some(glyph.to_string());
        }
        if file.is_none() && lname.contains("file") {
            file = Some(glyph.to_string());
        }
        if folder.is_some() && file.is_some() {
            break;
        }
    }
    (
        folder.unwrap_or_else(|| "D".to_string()),
        file.unwrap_or_else(|| "F".to_string()),
    )
}

/// Relative mtime label: `just now`, `5m ago`, `3h ago`, `2d ago`
/// (future timestamps → `just now`).
fn rel_time(mtime: SystemTime) -> String {
    let secs = SystemTime::now()
        .duration_since(mtime)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    if secs < 60 {
        "just now".to_string()
    } else if secs < 3600 {
        format!("{}m ago", secs / 60)
    } else if secs < 86400 {
        format!("{}h ago", secs / 3600)
    } else {
        format!("{}d ago", secs / 86400)
    }
}

fn default_sidebar() -> Vec<(String, String, PathBuf)> {
    let (folder_icon, _) = fs_glyphs();
    let home: Option<PathBuf> = std::env::var("HOME").map(PathBuf::from).ok();
    let mut out = Vec::new();
    if let Some(h) = &home {
        if h.is_dir() {
            out.push(("Home".to_string(), folder_icon.clone(), h.clone()));
        }
    }
    out.push((
        "Root".to_string(),
        folder_icon.clone(),
        PathBuf::from("/"),
    ));
    if let Some(h) = &home {
        for sub in ["Documents", "Downloads", "Music", "Pictures", "Videos"] {
            let p = h.join(sub);
            if p.is_dir() {
                out.push((sub.to_string(), folder_icon.clone(), p));
            }
        }
    }
    out
}

impl FileDialog {
    /// Build a dialog over `start`: used as-is when it is a directory,
    /// else its parent (callers pass the file's parent, `~/Documents`, or
    /// `~` — see [`App::open_load_dialog`] and friends).
    pub fn new(mode: DialogMode, purpose: DialogPurpose, start: PathBuf) -> Self {
        let cwd = if start.is_dir() {
            start
        } else {
            start
                .parent()
                .map(|p| p.to_path_buf())
                .unwrap_or(start)
        };
        let mut d = Self {
            mode,
            purpose,
            cwd,
            entries: Vec::new(),
            selected: None,
            filename: String::new(),
            sidebar: Vec::new(),
        };
        d.refresh();
        d
    }

    /// Re-read `cwd`: directories first (alpha, case-insensitive), then
    /// `*.omaframe.json` files (alpha, case-insensitive). Other files are
    /// hidden. An out-of-range selection resets to `None`; a missing or
    /// unreadable `cwd` yields an empty list (never panics).
    pub fn refresh(&mut self) {
        self.sidebar = default_sidebar();
        let mut dirs: Vec<DirEntry> = Vec::new();
        let mut files: Vec<DirEntry> = Vec::new();
        if let Ok(rd) = std::fs::read_dir(&self.cwd) {
            for ent in rd.flatten() {
                let name = ent.file_name().to_string_lossy().into_owned();
                let is_dir = ent.file_type().map(|t| t.is_dir()).unwrap_or(false);
                if is_dir {
                    dirs.push(DirEntry {
                        name,
                        is_dir: true,
                        size: 0,
                        modified: ent
                            .metadata()
                            .ok()
                            .and_then(|m| m.modified().ok())
                            .map(rel_time)
                            .unwrap_or_default(),
                    });
                } else if name.ends_with(".omaframe.json") {
                    let (size, modified) = ent
                        .metadata()
                        .ok()
                        .map(|m| {
                            (
                                m.len(),
                                m.modified().ok().map(rel_time).unwrap_or_default(),
                            )
                        })
                        .unwrap_or((0, String::new()));
                    files.push(DirEntry {
                        name,
                        is_dir: false,
                        size,
                        modified,
                    });
                }
            }
        }
        dirs.sort_by(|a, b| {
            a.name
                .to_lowercase()
                .cmp(&b.name.to_lowercase())
        });
        files.sort_by(|a, b| {
            a.name
                .to_lowercase()
                .cmp(&b.name.to_lowercase())
        });
        dirs.extend(files);
        self.entries = dirs;
        if self
            .selected
            .is_some_and(|s| s >= self.entries.len())
        {
            self.selected = None;
        }
    }

    /// Move the highlight by `dir` rows, wrapping around the list. Empty
    /// list → `None`; no selection yet → first (or last for `dir < 0`).
    pub fn move_selection(&mut self, dir: i32) {
        if self.entries.is_empty() {
            self.selected = None;
            return;
        }
        let n = self.entries.len();
        let next = match self.selected {
            None => {
                if dir < 0 {
                    n - 1
                } else {
                    0
                }
            }
            Some(s) => (s as i32 + dir).rem_euclid(n as i32) as usize,
        };
        self.selected = Some(next);
    }

    /// Descend into the selected directory (selection resets). No-op when
    /// nothing or a file is selected.
    pub fn enter_selected(&mut self) {
        let Some(s) = self.selected else { return };
        let Some(e) = self.entries.get(s) else { return };
        if !e.is_dir {
            return;
        }
        self.cwd = self.cwd.join(&e.name);
        self.refresh();
        self.selected = None;
    }

    /// Go to the parent directory (selection resets). No-op at the root.
    pub fn go_up(&mut self) {
        if let Some(p) = self.cwd.parent().map(|p| p.to_path_buf()) {
            self.cwd = p;
            self.refresh();
            self.selected = None;
        }
    }

    /// Jump to `path`: directories become `cwd`; a file path jumps to its
    /// parent and selects the file when it is listed.
    pub fn goto(&mut self, path: PathBuf) {
        if path.is_dir() {
            self.cwd = path;
            self.refresh();
            self.selected = None;
        } else if let Some(parent) = path.parent().map(|p| p.to_path_buf()) {
            if parent.is_dir() {
                let name = path
                    .file_name()
                    .map(|s| s.to_string_lossy().into_owned());
                self.cwd = parent;
                self.refresh();
                self.selected = name
                    .and_then(|n| {
                        self.entries
                            .iter()
                            .position(|e| !e.is_dir && e.name == n)
                    });
            }
        }
    }

    /// Type into the Save filename box (control chars ignored).
    pub fn type_char(&mut self, c: char) {
        if !c.is_control() {
            self.filename.push(c);
        }
    }

    /// Delete the last filename char.
    pub fn backspace(&mut self) {
        self.filename.pop();
    }

    /// Create `New Folder` / `New Folder 2` / …, then refresh and select it.
    pub fn make_folder(&mut self) {
        let mut n = 1u32;
        let name = loop {
            let cand = if n == 1 {
                "New Folder".to_string()
            } else {
                format!("New Folder {n}")
            };
            if !self.cwd.join(&cand).exists() {
                break cand;
            }
            n += 1;
        };
        let _ = std::fs::create_dir(self.cwd.join(&name));
        self.refresh();
        self.selected = self
            .entries
            .iter()
            .position(|e| e.is_dir && e.name == name);
    }

    /// The path to act on, or `None` when there is nothing valid to
    /// confirm: Open → the selected file (dirs / empty selection → `None`);
    /// Save → `cwd/filename` when the filename is non-blank.
    pub fn confirm_path(&self) -> Option<PathBuf> {
        match self.mode {
            DialogMode::Open => self
                .selected
                .and_then(|s| self.entries.get(s))
                .filter(|e| !e.is_dir)
                .map(|e| self.cwd.join(&e.name)),
            DialogMode::Save => {
                if self.filename.trim().is_empty() {
                    None
                } else {
                    Some(self.cwd.join(&self.filename))
                }
            }
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
    pub fg: PaintColor,
    pub bg: Option<PaintColor>,
    pub box_style: draw::BoxStyle,
    pub arrow: bool,
    pub cursor: (i32, i32),
    pub viewport: (i32, i32),
    pub palette_tab: usize,
    pub palette_scroll: usize,
    pub colors_scroll: usize,
    pub preview_dark: bool,
    pub file_dialog: Option<FileDialog>,
    pub status_msg: String,
    pub text_buffer: String,
    // --- session state (not in the brief's field list, but required) ---
    pub file_path: Option<PathBuf>,
    pub dirty: bool,
    pub should_quit: bool,
    anchor: Option<(i32, i32)>,
    drawing: bool,
    text_start: Option<(i32, i32)>,
    pan_anchor: Option<(u16, u16, (i32, i32))>,
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
            fg: PaintColor::Ansi(7),
            bg: None,
            box_style: draw::BoxStyle::Light,
            arrow: false,
            cursor: (0, 0),
            viewport: (0, 0),
            palette_tab,
            palette_scroll: 0,
            colors_scroll: 0,
            preview_dark,
            file_dialog: None,
            status_msg: "click-drag draws · wheel scrolls · middle-drag pans · right-click grabs · Ctrl-S saves"
                .to_string(),
            text_buffer: String::new(),
            file_path,
            dirty: false,
            should_quit: false,
            anchor: None,
            drawing: false,
            text_start: None,
            pan_anchor: None,
        }
    }

    pub fn set_status(&mut self, msg: impl Into<String>) {
        self.status_msg = msg.into();
    }

    /// `File: …` menu row: full path (`~`-shortened) with dirty star.
    pub fn full_path_label(&self) -> String {
        let base = match &self.file_path {
            Some(p) => {
                let s = p.display().to_string();
                if let Ok(home) = std::env::var("HOME") {
                    if let Some(rest) = s.strip_prefix(&home) {
                        format!("~{rest}")
                    } else {
                        s
                    }
                } else {
                    s
                }
            }
            None => "untitled".to_string(),
        };
        format!("{base}{}", if self.dirty { "*" } else { "" })
    }

    pub fn fg_label(&self) -> String {
        match self.fg {
            PaintColor::Ansi(i) => i.to_string(),
            rgb => rgb.to_hex(),
        }
    }

    /// Background pot label for the status bar (`None` → `"-"`).
    pub fn bg_label(&self) -> String {
        match self.bg {
            None => "-".to_string(),
            Some(PaintColor::Ansi(i)) => i.to_string(),
            Some(rgb) => rgb.to_hex(),
        }
    }

    // --- cursor / viewport ---

    /// Resolve the cursor off wide-char continuation guards. The canvas is
    /// infinite: no grid clamping, the cursor roams free.
    pub fn clamp_cursor(&mut self) {
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
        // Infinite canvas: the viewport roams free (may go negative).
        self.viewport = (nox, noy);
    }

    // --- middle-drag pan ---

    /// Begin a pan gesture: records the screen cell and viewport origin.
    pub fn start_pan(&mut self, col: u16, row: u16) {
        self.pan_anchor = Some((col, row, self.viewport));
    }

    /// Extend a pan gesture: drags the viewport by the screen delta.
    pub fn update_pan(&mut self, col: u16, row: u16) {
        if let Some((c0, r0, (ox, oy))) = self.pan_anchor {
            self.viewport = (
                ox - (col as i32 - c0 as i32),
                oy - (row as i32 - r0 as i32),
            );
        }
    }

    /// Release a pan gesture.
    pub fn end_pan(&mut self) {
        self.pan_anchor = None;
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

    /// Scroll the palette grid by `dir` rows. `cols`/`visible` describe the
    /// on-screen grid (row count = ceil(items / cols)); the scroll clamps so
    /// the last content row stays reachable without scrolling into the void.
    pub fn scroll_palette(&mut self, dir: i32, cols: usize, visible: usize) {
        let len = palette_chars(self.palette_tab).len();
        let total_rows = len.div_ceil(cols.max(1)).max(1);
        let max = total_rows.saturating_sub(visible.max(1));
        let next = self.palette_scroll as i32 + dir;
        self.palette_scroll = next.clamp(0, max as i32) as usize;
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

    /// Cycle the foreground pot through ANSI slots 0–15 (a truecolor pot
    /// resets into the ANSI cycle at 0).
    pub fn cycle_fg(&mut self) {
        let next = match self.fg {
            PaintColor::Ansi(i) => (i + 1) % 16,
            PaintColor::Rgb(..) => 0,
        };
        self.fg = PaintColor::Ansi(next);
        self.set_status(format!("fg: {}", self.fg_label()));
    }

    /// Cycle the background pot `None → Ansi(0..15) → None` (a truecolor
    /// pot steps to `None`, the end of the cycle).
    pub fn cycle_bg(&mut self) {
        self.bg = match self.bg {
            None => Some(PaintColor::Ansi(0)),
            Some(PaintColor::Ansi(i)) if i >= 15 => None,
            Some(PaintColor::Ansi(i)) => Some(PaintColor::Ansi(i + 1)),
            Some(PaintColor::Rgb(..)) => None,
        };
        self.set_status(format!("bg: {}", self.bg_label()));
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
            self.autosave();
            self.set_status("undo");
        } else {
            self.set_status("nothing to undo");
        }
    }

    pub fn redo(&mut self) {
        if self.history.redo(&mut self.doc) {
            self.dirty = true;
            self.autosave();
            self.set_status("redo");
        } else {
            self.set_status("nothing to redo");
        }
    }

    /// Explicit save (Ctrl-S): writes to the bound path, or opens a SaveAs
    /// dialog when pathless (returns false in that case).
    pub fn save(&mut self) -> bool {
        let Some(path) = self.file_path.clone() else {
            self.open_save_as_dialog();
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

    // --- colors panel (direct fg/bg pots) ---

    /// Set the foreground pot (left-click a swatch, or a truecolor pick).
    pub fn set_fg(&mut self, fg: PaintColor) {
        self.fg = fg;
        self.set_status(format!("fg: {}", self.fg_label()));
    }

    /// Set the background pot (right-click a swatch). `None` = transparent.
    pub fn set_bg(&mut self, bg: Option<PaintColor>) {
        self.bg = bg;
        self.set_status(format!("bg: {}", self.bg_label()));
    }

    /// Scroll the colors list by `dir` rows. `total_rows` is the full row
    /// count (ANSI slots + group headers + bonus palettes — the caller
    /// computes it); `visible` is the rows on screen.
    pub fn scroll_colors(&mut self, dir: i32, total_rows: usize, visible: usize) {
        let max = total_rows.saturating_sub(visible.max(1));
        let next = self.colors_scroll as i32 + dir;
        self.colors_scroll = next.clamp(0, max as i32) as usize;
    }

    // --- menu file ops + file dialog ---

    /// Expand `~` and relative paths against the current directory.
    pub fn expand_path(raw: &str) -> PathBuf {
        let s = raw.trim();
        if let Some(rest) = s.strip_prefix("~/").or_else(|| s.strip_prefix("~")) {
            if let Ok(home) = std::env::var("HOME") {
                return PathBuf::from(home).join(rest.trim_start_matches('/'));
            }
        }
        PathBuf::from(s)
    }

    fn doc_name_for(path: &std::path::Path) -> String {
        path.file_stem()
            .and_then(|s| s.to_str())
            .filter(|s| !s.is_empty())
            .unwrap_or("untitled")
            .to_string()
    }

    /// Dialog start dir: the bound file's parent, else `~/Documents` when it
    /// exists, else `~` (else the process cwd).
    fn dialog_start_dir(&self) -> PathBuf {
        if let Some(p) = &self.file_path {
            if let Some(parent) = p.parent() {
                if !parent.as_os_str().is_empty() {
                    return parent.to_path_buf();
                }
            }
        }
        if let Ok(home) = std::env::var("HOME") {
            let docs = PathBuf::from(&home).join("Documents");
            if docs.is_dir() {
                return docs;
            }
            return PathBuf::from(home);
        }
        std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
    }

    fn save_dialog_filename(&self) -> String {
        format!("{}.omaframe.json", self.doc.name)
    }

    /// Menu → Load: open the Load dialog over the start dir.
    pub fn open_load_dialog(&mut self) {
        let start = self.dialog_start_dir();
        self.file_dialog = Some(FileDialog::new(
            DialogMode::Open,
            DialogPurpose::Load,
            start,
        ));
        self.set_status("load: pick a .omaframe.json file");
    }

    /// Open the SaveNew dialog (fresh bound file at the confirmed path).
    pub fn open_save_new_dialog(&mut self) {
        let start = self.dialog_start_dir();
        let mut d = FileDialog::new(DialogMode::Save, DialogPurpose::SaveNew, start);
        d.filename = self.save_dialog_filename();
        self.file_dialog = Some(d);
        self.set_status("new file: pick a folder + name");
    }

    /// Open the SaveAs dialog (re-bind the current document on confirm).
    fn open_save_as_dialog(&mut self) {
        let start = self.dialog_start_dir();
        let mut d = FileDialog::new(DialogMode::Save, DialogPurpose::SaveAs, start);
        d.filename = self.save_dialog_filename();
        self.file_dialog = Some(d);
        self.set_status("save as: pick a folder + name");
    }

    /// Menu → New: keeps the file path slot (save first if dirty), starts a
    /// fresh 80×24 canvas.
    pub fn new_file_to(&mut self, path: PathBuf) -> bool {
        self.autosave();
        self.doc = Document::new(Self::doc_name_for(&path), 80, 24);
        self.preview_dark = self.doc.preview_dark;
        self.palette_tab = palette_tab_index(&self.doc.palette_tab);
        self.history = History::new();
        self.scratch.clear();
        self.cursor = (0, 0);
        self.viewport = (0, 0);
        self.palette_scroll = 0;
        self.file_path = Some(path.clone());
        match omaframe::model::save_file(&self.doc, &path) {
            Ok(()) => {
                self.dirty = false;
                self.set_status(format!("created {}", path.display()));
                true
            }
            Err(e) => {
                self.dirty = true;
                self.set_status(format!("save failed: {e}"));
                false
            }
        }
    }

    /// Menu → Load / dialog Load confirm: replaces the document on success,
    /// keeps the old one + status on failure.
    pub fn load_file_path(&mut self, path: PathBuf) -> bool {
        match omaframe::model::load_file(&path) {
            Ok(doc) => {
                self.preview_dark = doc.preview_dark;
                self.palette_tab = palette_tab_index(&doc.palette_tab);
                self.doc = doc;
                self.file_path = Some(path.clone());
                self.history = History::new();
                self.scratch.clear();
                self.cursor = (0, 0);
                self.viewport = (0, 0);
                self.palette_scroll = 0;
                self.dirty = false;
                self.set_status(format!("loaded {}", path.display()));
                true
            }
            Err(e) => {
                self.set_status(format!("load failed: {e}"));
                false
            }
        }
    }

    /// Save under a new path (dialog SaveAs confirm).
    pub fn save_as_path(&mut self, path: PathBuf) -> bool {
        match omaframe::model::save_file(&self.doc, &path) {
            Ok(()) => {
                self.file_path = Some(path.clone());
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

    /// Menu → Save: normal save, or a SaveAs dialog when pathless.
    pub fn menu_save(&mut self) {
        if self.file_path.is_some() {
            self.save();
        } else {
            self.open_save_as_dialog();
        }
    }

    /// Confirm the open dialog: Load → [`App::load_file_path`], SaveNew →
    /// [`App::new_file_to`], SaveAs → [`App::save_as_path`]. Closes the
    /// dialog + autosaves on success; on failure (or nothing confirmable)
    /// sets a status and stays open.
    pub fn dialog_confirm(&mut self) {
        let Some(dlg) = self.file_dialog.as_ref() else {
            return;
        };
        let purpose = dlg.purpose;
        let Some(path) = dlg.confirm_path() else {
            self.set_status(match purpose {
                DialogPurpose::Load => "load: select a file",
                _ => "save: enter a file name",
            });
            return;
        };
        let ok = match purpose {
            DialogPurpose::Load => self.load_file_path(path),
            DialogPurpose::SaveNew => self.new_file_to(path),
            DialogPurpose::SaveAs => self.save_as_path(path),
        };
        if ok {
            self.file_dialog = None;
            self.autosave();
        }
    }

    /// Dismiss the open dialog (no action).
    pub fn dialog_cancel(&mut self) {
        self.file_dialog = None;
        self.set_status("cancelled");
    }

    /// Dialog delegate: move the file highlight by `dir` rows.
    pub fn dialog_move_selection(&mut self, dir: i32) {
        if let Some(d) = self.file_dialog.as_mut() {
            d.move_selection(dir);
        }
    }

    /// Dialog delegate: type into the Save filename box.
    pub fn dialog_type(&mut self, c: char) {
        if let Some(d) = self.file_dialog.as_mut() {
            d.type_char(c);
        }
    }

    /// Dialog delegate: filename backspace.
    pub fn dialog_backspace(&mut self) {
        if let Some(d) = self.file_dialog.as_mut() {
            d.backspace();
        }
    }

    /// Dialog delegate: descend into the selected folder; when a FILE is
    /// selected in Open mode, confirm immediately.
    pub fn dialog_enter(&mut self) {
        let confirm_now = match self.file_dialog.as_mut() {
            Some(d) => {
                d.enter_selected();
                d.mode == DialogMode::Open && d.confirm_path().is_some()
            }
            None => return,
        };
        if confirm_now {
            self.dialog_confirm();
        }
    }

    /// Dialog delegate: go to the parent directory.
    pub fn dialog_up(&mut self) {
        if let Some(d) = self.file_dialog.as_mut() {
            d.go_up();
        }
    }

    /// Dialog delegate: jump to sidebar entry `i` (no-op when out of range).
    pub fn dialog_goto_sidebar(&mut self, i: usize) {
        let path = self
            .file_dialog
            .as_ref()
            .and_then(|d| d.sidebar.get(i))
            .map(|(_, _, p)| p.clone());
        if let Some(p) = path {
            if let Some(d) = self.file_dialog.as_mut() {
                d.goto(p);
            }
        }
    }

    /// Dialog delegate: create `New Folder…` and select it.
    pub fn dialog_make_folder(&mut self) {
        if let Some(d) = self.file_dialog.as_mut() {
            d.make_folder();
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

    /// Recolor a `draw_*` patch (placeholder `DRAW_FG` cells) to the fg/bg
    /// pots; transparent markers pass through as erasures.
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
            Tool::Pan => {
                // Pan is driven by screen coords (main.rs middle-drag or Pan
                // tool drags); nothing to anchor on the document grid.
                self.anchor = None;
                self.drawing = false;
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
                // Ovals never snap (tools-spec §5); the outline uses the
                // active palette char + pots, like the pencil.
                self.scratch =
                    draw::paint_cells(&draw::ellipse_cells(r), &self.ch, self.fg, self.bg);
            }
            Tool::Rect => {
                let c = if shift {
                    Self::constrain_square_anchor(a, (x, y))
                } else {
                    (x, y)
                };
                let r = Rect::from_points(a.0, a.1, c.0, c.1);
                // Plain rectangle outline in the palette char (Box stays the
                // smart auto-junction tool).
                self.scratch =
                    draw::paint_cells(&draw::rect_cells(r), &self.ch, self.fg, self.bg);
            }
            Tool::Select => {
                let r = Rect::from_points(a.0, a.1, x, y);
                self.history.set_selection(Some(r));
                self.set_status(format!("select {}x{}", r.w, r.h));
            }
            Tool::Text | Tool::Grab | Tool::Pan => {}
        }
    }

    /// Release: commit one undo entry (or nothing for no-ops).
    pub fn end_stroke(&mut self) {
        if !self.drawing {
            return;
        }
        match self.tool {
            Tool::Pencil | Tool::Eraser | Tool::Box | Tool::RoundedRect | Tool::Line
            | Tool::Oval | Tool::Rect => {
                if self.scratch.is_empty() {
                    self.anchor = None;
                    self.drawing = false;
                    return;
                }
                let patch = std::mem::take(&mut self.scratch);
                if self.history.commit(&mut self.doc, &patch) {
                    self.dirty = true;
                    self.autosave();
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
            Tool::Text | Tool::Grab | Tool::Pan => {
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

    /// Eyedropper (decisions.md Q4): copy ch+fg+bg into the pencil, verbatim.
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
                let bg = match self.bg {
                    None => "-".to_string(),
                    Some(PaintColor::Ansi(i)) => i.to_string(),
                    Some(rgb) => rgb.to_hex(),
                };
                let fg = match self.fg {
                    PaintColor::Ansi(i) => i.to_string(),
                    rgb => rgb.to_hex(),
                };
                self.set_status(format!("grabbed '{}' fg {fg} bg {bg}", self.ch));
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
            self.autosave();
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
                self.autosave();
                self.set_status("selection cut");
            }
            self.history.clear_selection();
            return;
        }
        let mut patch = Layer::new();
        patch.set(self.cursor.0, self.cursor.1, Cell::erased());
        if self.history.commit(&mut self.doc, &patch) {
            self.dirty = true;
            self.autosave();
            self.set_status("cleared 1 cell");
        }
    }

    /// Palette click: load the char into the pencil (pots unchanged).
    pub fn pick_palette_char(&mut self, s: &str) {
        self.ch = s.to_string();
        if self.tool != Tool::Pencil {
            self.tool = Tool::Pencil;
        }
        self.set_status(format!(
            "ch '{s}' fg {} tab {}",
            self.fg_label(),
            PALETTE_TABS[self.palette_tab]
        ));
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
    use omaframe::chars;

    fn test_doc() -> Document {
        Document::new("t", 80, 24)
    }

    /// Unique temp dir per test (pid-tagged; removed at the end).
    fn tempdir(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("omaframe-{}-{}", tag, std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
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
    fn cursor_roams_free_and_off_guards() {
        // Infinite canvas: no grid clamping, the cursor keeps any coords.
        let mut app = App::new(test_doc(), None);
        app.cursor = (999, -5);
        app.clamp_cursor();
        assert_eq!(app.cursor, (999, -5));
        app.cursor = (-3, 99);
        app.clamp_cursor();
        assert_eq!(app.cursor, (-3, 99));

        // Wide-char guard resolves to its anchor.
        let mut patch = Layer::new();
        patch.set(5, 5, Cell::new("漢", PaintColor::Ansi(1), None));
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
        // Letters/numbers stay seeded here; symbols/outlines/blocks come
        // from the sibling chars module (wiring, not counts).
        assert_eq!(palette_chars(0).len(), 52);
        assert_eq!(palette_chars(1).len(), 10);
        assert_eq!(palette_chars(2), chars::symbols_tab());
        assert_eq!(palette_chars(3), chars::outlines_tab());
        assert_eq!(palette_chars(4), chars::blocks_tab());
        assert!(!palette_chars(2).is_empty());
        assert!(!palette_chars(3).is_empty());
        assert!(!palette_chars(4).is_empty());
        assert!(!palette_chars(5).is_empty()); // nerds from assets/nerd.txt
    }

    #[test]
    fn truecolor_pots_cycle_set_and_label() {
        let mut app = App::new(test_doc(), None);
        assert_eq!(app.fg, PaintColor::Ansi(7));
        assert_eq!(app.bg, None);
        assert_eq!(app.bg_label(), "-");
        // fg cycles Ansi 0-15 with wraparound.
        app.cycle_fg();
        assert_eq!(app.fg, PaintColor::Ansi(8));
        for _ in 0..7 {
            app.cycle_fg();
        }
        assert_eq!(app.fg, PaintColor::Ansi(15));
        app.cycle_fg();
        assert_eq!(app.fg, PaintColor::Ansi(0));
        // Cycling from a truecolor pot resets into the ANSI cycle.
        app.set_fg(PaintColor::Rgb(200, 100, 50));
        assert_eq!(app.fg, PaintColor::Rgb(200, 100, 50));
        assert_eq!(app.fg_label(), "#c86432");
        app.cycle_fg();
        assert_eq!(app.fg, PaintColor::Ansi(0));
        // bg cycles None → Ansi(0..15) → None.
        app.cycle_bg();
        assert_eq!(app.bg, Some(PaintColor::Ansi(0)));
        for _ in 0..15 {
            app.cycle_bg();
        }
        assert_eq!(app.bg, Some(PaintColor::Ansi(15)));
        assert_ne!(app.bg_label(), "-");
        app.cycle_bg();
        assert_eq!(app.bg, None);
        assert_eq!(app.bg_label(), "-");
        // Setters take truecolor verbatim.
        app.set_fg(PaintColor::Rgb(1, 2, 3));
        app.set_bg(Some(PaintColor::Rgb(4, 5, 6)));
        assert_eq!(app.fg, PaintColor::Rgb(1, 2, 3));
        assert_eq!(app.bg, Some(PaintColor::Rgb(4, 5, 6)));
        assert_eq!(app.bg_label(), "#040506");
    }

    #[test]
    fn tint_maps_placeholders_to_truecolor_pots() {
        let mut app = App::new(test_doc(), None);
        app.set_fg(PaintColor::Rgb(10, 20, 30));
        app.set_bg(Some(PaintColor::Ansi(2)));
        let mut raw = Layer::new();
        raw.set(0, 0, Cell::new("─", draw::DRAW_FG, None));
        raw.set(1, 0, Cell::erased());
        let tinted = app.tint(raw);
        assert_eq!(
            tinted.get(0, 0),
            Some(Cell::new(
                "─",
                PaintColor::Rgb(10, 20, 30),
                Some(PaintColor::Ansi(2))
            ))
        );
        // Erased markers pass through (compose empty, raw key kept).
        assert_eq!(tinted.get(1, 0), None);
        assert!(tinted.keys().any(|k| k == (1, 0)));
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
    fn rect_and_oval_use_palette_char() {
        let mut app = App::new(test_doc(), None);
        app.ch = "#".to_string();
        app.fg = PaintColor::Ansi(2);
        // Rect: plain outline in the palette char, committed as one entry.
        app.set_tool(Tool::Rect);
        app.start_stroke(0, 0, false);
        app.update_stroke(3, 2, false);
        app.end_stroke();
        assert_eq!(app.history.undo_len(), 1);
        let cell = app.doc.cell(0, 0).expect("rect corner");
        assert_eq!(cell.ch, "#");
        assert_eq!(cell.fg, PaintColor::Ansi(2));
        assert!(app.doc.cell(1, 1).is_none(), "rect interior empty");
        // Oval: same char discipline (no fixed ─/│).
        app.set_tool(Tool::Oval);
        app.start_stroke(10, 10, false);
        app.update_stroke(14, 13, false);
        app.end_stroke();
        // Every painted cell (rect + oval) uses the palette char.
        let mut painted = 0;
        for ((_, _), c) in app.doc.active_layer().entries() {
            if c.is_transparent() {
                continue;
            }
            painted += 1;
            assert_eq!(c.ch, "#", "oval paints the palette char, no fixed ─/│");
        }
        assert!(painted > 10, "rect + oval left marks");
    }

    #[test]
    fn fs_glyphs_come_from_nerd_txt() {
        // First "folder"/"file" name hits in assets/nerd.txt.
        let (folder, file) = fs_glyphs();
        assert_eq!(folder, "\u{f07b}");
        assert_eq!(file, "\u{f15b}");
    }

    #[test]
    fn dialog_navigation_make_folder_and_confirm_paths() {
        let root = tempdir("dlg");
        std::fs::create_dir_all(root.join("sub").join("nested")).unwrap();
        std::fs::write(root.join("sub").join("a.omaframe.json"), "{}").unwrap();
        std::fs::write(root.join("b.omaframe.json"), "{}").unwrap();
        std::fs::write(root.join("ignore.txt"), "x").unwrap();

        // Open dialog over the root: dirs first, then *.omaframe.json files.
        let mut d = FileDialog::new(DialogMode::Open, DialogPurpose::Load, root.clone());
        assert_eq!(d.cwd, root);
        let names: Vec<(&str, bool)> = d
            .entries
            .iter()
            .map(|e| (e.name.as_str(), e.is_dir))
            .collect();
        assert_eq!(names, vec![("sub", true), ("b.omaframe.json", false)]);
        assert!(!d.sidebar.is_empty());
        assert!(d.sidebar.iter().any(|(l, _, _)| l == "Root"));

        // Selection starts empty; moving selects the first row, enter descends.
        assert_eq!(d.selected, None);
        d.move_selection(1);
        assert_eq!(d.selected, Some(0));
        d.enter_selected();
        assert_eq!(d.cwd, root.join("sub"));
        assert_eq!(d.selected, None);
        // Back up, then jump straight into a file's parent via goto.
        d.go_up();
        assert_eq!(d.cwd, root);
        d.goto(root.join("sub").join("a.omaframe.json"));
        assert_eq!(d.cwd, root.join("sub"));
        assert_eq!(d.selected, Some(1)); // [nested(dir), a.omaframe.json]
        d.goto(root.clone());

        // make_folder creates New Folder / New Folder 2… and selects each.
        d.make_folder();
        assert!(root.join("New Folder").is_dir());
        assert_eq!(
            d.selected
                .and_then(|s| d.entries.get(s))
                .map(|e| e.name.clone()),
            Some("New Folder".to_string())
        );
        d.make_folder();
        assert!(root.join("New Folder 2").is_dir());
        assert_eq!(
            d.selected
                .and_then(|s| d.entries.get(s))
                .map(|e| e.name.clone()),
            Some("New Folder 2".to_string())
        );

        // Open confirm: file → full path; dir / nothing → None.
        d.goto(root.clone());
        let file_idx = d
            .entries
            .iter()
            .position(|e| !e.is_dir && e.name == "b.omaframe.json")
            .unwrap();
        d.selected = Some(file_idx);
        assert_eq!(d.confirm_path(), Some(root.join("b.omaframe.json")));
        d.selected = Some(0); // a dir ("New Folder")
        assert!(d.entries[0].is_dir);
        assert_eq!(d.confirm_path(), None);
        d.selected = None;
        assert_eq!(d.confirm_path(), None);

        // Save confirm: blank filename → None, else cwd/filename.
        let mut s = FileDialog::new(DialogMode::Save, DialogPurpose::SaveAs, root.clone());
        assert_eq!(s.confirm_path(), None);
        for c in "n.omaframe.json".chars() {
            s.type_char(c);
        }
        assert_eq!(s.filename, "n.omaframe.json");
        assert_eq!(s.confirm_path(), Some(root.join("n.omaframe.json")));
        s.backspace();
        assert_eq!(s.filename, "n.omaframe.jso");
        s.filename.clear();
        for c in "   ".chars() {
            s.type_char(c);
        }
        assert_eq!(s.confirm_path(), None, "blank filename confirms nothing");

        // Tilde expansion still works for typed paths.
        let home = std::env::var("HOME").unwrap();
        assert_eq!(
            App::expand_path("~/x/y.json"),
            std::path::PathBuf::from(home).join("x/y.json")
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn dialog_save_new_and_load_round_trip() {
        let root = tempdir("dlg-rt");
        // SaveNew flow through dialog_confirm: fresh doc bound + saved.
        let mut app = App::new(test_doc(), None);
        app.open_save_new_dialog();
        assert!(app.file_dialog.is_some());
        assert_eq!(
            app.file_dialog.as_ref().unwrap().purpose,
            DialogPurpose::SaveNew
        );
        app.file_dialog.as_mut().unwrap().goto(root.clone());
        app.file_dialog.as_mut().unwrap().filename.clear();
        for c in "rt.omaframe.json".chars() {
            app.dialog_type(c);
        }
        app.dialog_confirm();
        assert!(app.file_dialog.is_none(), "dialog closes on success");
        let path = root.join("rt.omaframe.json");
        assert_eq!(app.file_path, Some(path.clone()));
        assert!(path.exists(), "new_file_to saves immediately");

        // Paint + commit (autosaves through the new binding), then load the
        // file back in a fresh app via the Open dialog + Enter.
        app.start_stroke(2, 3, false);
        app.update_stroke(4, 3, false);
        app.end_stroke();
        let on_disk = omaframe::model::load_file(&path).unwrap();
        assert!(on_disk.cell(2, 3).is_some(), "autosaved stroke on disk");

        let mut app2 = App::new(test_doc(), None);
        app2.open_load_dialog();
        app2.file_dialog.as_mut().unwrap().goto(root.clone());
        {
            let dlg = app2.file_dialog.as_ref().unwrap();
            let idx = dlg
                .entries
                .iter()
                .position(|e| !e.is_dir && e.name == "rt.omaframe.json")
                .unwrap();
            app2.file_dialog.as_mut().unwrap().selected = Some(idx);
        }
        app2.dialog_enter(); // file in Open mode confirms immediately
        assert!(app2.file_dialog.is_none());
        assert_eq!(app2.file_path, Some(path.clone()));
        assert!(app2.doc.cell(2, 3).is_some(), "loaded painted cell");

        // Failure keeps the dialog open: blank SaveAs name confirms nothing.
        app2.menu_save(); // path bound → plain save, no dialog
        assert!(app2.file_dialog.is_none());
        let mut app3 = App::new(test_doc(), None);
        app3.open_save_new_dialog();
        app3.file_dialog.as_mut().unwrap().goto(root.clone());
        app3.file_dialog.as_mut().unwrap().filename.clear();
        app3.dialog_confirm();
        assert!(
            app3.file_dialog.is_some(),
            "failed/empty confirm stays open"
        );
        assert!(app3.status_msg.starts_with("save:"));
        app3.dialog_cancel();
        assert!(app3.file_dialog.is_none());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn autosave_writes_file_on_stroke_commit() {
        let root = tempdir("autosave");
        let path = root.join("a.omaframe.json");
        let mut app = App::new(test_doc(), Some(path.clone()));
        assert!(!path.exists());
        app.start_stroke(1, 1, false);
        app.update_stroke(2, 1, false);
        app.end_stroke();
        assert!(path.exists(), "stroke commit autosaves to the bound path");
        assert!(!app.dirty);
        let doc = omaframe::model::load_file(&path).unwrap();
        assert!(doc.cell(1, 1).is_some());
        assert!(doc.cell(2, 1).is_some());
        // Undo/redo also persist.
        app.undo();
        let doc = omaframe::model::load_file(&path).unwrap();
        assert_eq!(doc.cell(1, 1), None);
        app.redo();
        let doc = omaframe::model::load_file(&path).unwrap();
        assert!(doc.cell(2, 1).is_some());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn no_inline_prompt_state_dialogs_instead() {
        // The inline path-prompt system is gone: file picking is modal.
        // (Compile-time proof: no `prompt`/`prompt_buf` fields, no
        // `PromptKind`, no start/confirm_prompt methods remain.)
        let mut app = App::new(test_doc(), None);
        assert!(app.file_dialog.is_none(), "no dialog open by default");
        app.open_load_dialog();
        assert!(app.file_dialog.is_some());
        app.dialog_cancel();
        assert!(app.file_dialog.is_none());
        // Pathless explicit save opens a SaveAs dialog and returns false.
        assert!(!app.save());
        assert!(app.file_dialog.is_some());
        assert_eq!(
            app.file_dialog.as_ref().unwrap().purpose,
            DialogPurpose::SaveAs
        );
    }

    #[test]
    fn menu_file_ops_and_color_pots() {
        let mut app = App::new(test_doc(), None);
        // New-file flow binds the path and saves immediately (macOS-style).
        let dir = std::env::temp_dir().join("omaframe-newfile-test");
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("n.omaframe.json");
        let _ = std::fs::remove_file(&path);
        app.dirty = true;
        assert!(app.new_file_to(path.clone()));
        assert!(!app.dirty);
        assert_eq!(app.history.undo_len(), 0);
        assert!(path.exists(), "created up front");
        let _ = std::fs::remove_dir_all(&dir);
        // Truecolor pots accept any PaintColor verbatim.
        app.set_fg(PaintColor::Ansi(3));
        assert_eq!(app.fg, PaintColor::Ansi(3));
        app.set_fg(PaintColor::Rgb(9, 9, 9));
        assert_eq!(app.fg, PaintColor::Rgb(9, 9, 9));
        app.set_bg(None);
        assert_eq!(app.bg, None);
        assert_eq!(app.bg_label(), "-");
        app.set_bg(Some(PaintColor::Ansi(5)));
        assert_eq!(app.bg, Some(PaintColor::Ansi(5)));
        // Pan tool shortcut + full path label.
        assert_eq!(Tool::from_shortcut('_'), Some(Tool::Pan));
        assert!(app.full_path_label().ends_with("n.omaframe.json"));
        // Colors scroll clamps against the caller-provided total.
        // (17 color rows: transparent + 16 slots, headers add more.)
        app.scroll_colors(99, 17, 5);
        assert_eq!(app.colors_scroll, 12);
        app.scroll_colors(-99, 17, 5);
        assert_eq!(app.colors_scroll, 0);
        app.scroll_colors(3, 10, 4);
        assert_eq!(app.colors_scroll, 3);
        app.scroll_colors(99, 10, 4);
        assert_eq!(app.colors_scroll, 6);
        app.scroll_colors(1, 3, 10);
        assert_eq!(app.colors_scroll, 0);
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
    fn grab_copies_cell_into_pencil_verbatim() {
        let mut app = App::new(test_doc(), None);
        let mut patch = Layer::new();
        patch.set(
            4,
            4,
            Cell::new("Z", PaintColor::Ansi(9), Some(PaintColor::Ansi(2))),
        );
        patch.set(
            5,
            5,
            Cell::new(
                "Q",
                PaintColor::Rgb(11, 22, 33),
                Some(PaintColor::Rgb(44, 55, 66)),
            ),
        );
        assert!(app.history.commit(&mut app.doc, &patch));
        assert!(app.grab_at(4, 4));
        assert_eq!(app.ch, "Z");
        assert_eq!(app.fg, PaintColor::Ansi(9));
        assert_eq!(app.bg, Some(PaintColor::Ansi(2)));
        assert_eq!(app.tool, Tool::Pencil);
        // Truecolor cells copy verbatim too.
        assert!(app.grab_at(5, 5));
        assert_eq!(app.ch, "Q");
        assert_eq!(app.fg, PaintColor::Rgb(11, 22, 33));
        assert_eq!(app.bg, Some(PaintColor::Rgb(44, 55, 66)));
        assert!(!app.grab_at(70, 20)); // empty: changes nothing
        assert_eq!(app.ch, "Q");
    }
}
