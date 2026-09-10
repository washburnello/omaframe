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
use omaframe::model::{Cell, Document, History, Layer, PaintColor, Rect, Widget};
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

// ---------------------------------------------------------------------------
// Glyph palette (categorized FA + Octicons, searchable)
// ---------------------------------------------------------------------------

/// One palette glyph: display char, searchable name, descriptive category.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Glyph {
    pub ch: String,
    pub name: String,
    pub category: String,
}

/// Glyph category stepper order (`All` first, then alphabetical).
pub const GLYPH_CATEGORIES: [&str; 21] = [
    "All",
    "Arrows",
    "Brands",
    "Commerce",
    "Communication",
    "Culture & Faith",
    "Development",
    "Devices",
    "Editing",
    "Files & Folders",
    "Food & Drink",
    "Home",
    "Interface",
    "Media",
    "Medical",
    "Misc",
    "Nature",
    "Octicons",
    "People",
    "Travel",
    "Weather & Time",
];

/// Full glyph list parsed once (`assets/glyphs.txt` is ~2k lines; parsing
/// per render would be wasteful). Format per line:
/// `<glyph>  <name>  <U+CODE>  <Category…>` (category runs to EOL).
pub fn all_glyphs() -> &'static Vec<Glyph> {
    static CACHE: std::sync::OnceLock<Vec<Glyph>> = std::sync::OnceLock::new();
    CACHE.get_or_init(|| {
        const SRC: &str = include_str!("../assets/glyphs.txt");
        let mut out = Vec::new();
        for line in SRC.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let mut parts = line.split_whitespace();
            let (Some(ch), Some(name), Some(code)) =
                (parts.next(), parts.next(), parts.next())
            else {
                continue;
            };
            if !code.starts_with('U') {
                continue;
            }
            // Category runs to end of line (`Files & Folders` has spaces).
            let category: String = parts.collect::<Vec<_>>().join(" ");
            if category.is_empty() {
                continue;
            }
            out.push(Glyph {
                ch: ch.to_string(),
                name: name.to_string(),
                category: category.to_string(),
            });
        }
        out
    })
}

/// Palette contents per tab. Letters/numbers/widgets are seeded here;
/// symbols/outlines/blocks come from the sibling `omaframe::chars` module;
/// glyphs (tab 5) come from [`all_glyphs`] (single chars; widgets tab holds
/// multi-char stamp names). NOTE: tab 5 ignores category/search filters —
/// UI code must use [`palette_visible`] instead.
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
// Selection handles + select-tool drags (Phase 3)
// ---------------------------------------------------------------------------

/// Selection handle positions: corners + edge midpoints.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Handle {
    NW,
    N,
    NE,
    E,
    SE,
    S,
    SW,
    W,
}

impl Handle {
    pub fn is_corner(self) -> bool {
        matches!(
            self,
            Handle::NW | Handle::NE | Handle::SE | Handle::SW
        )
    }

    /// Cell of this handle within `r` (`None` for empty rects). Degenerate
    /// rects merge handles: 1-wide rects collapse E/W into the corners and
    /// N/S onto the column; 1-tall likewise. Callers should dedupe.
    pub fn pos(self, r: &Rect) -> Option<(i32, i32)> {
        if r.w == 0 || r.h == 0 {
            return None;
        }
        let (x0, y0) = (r.x, r.y);
        let (x1, y1) = (r.x + r.w as i32 - 1, r.y + r.h as i32 - 1);
        let mx = r.x + r.w as i32 / 2;
        let my = r.y + r.h as i32 / 2;
        Some(match self {
            Handle::NW => (x0, y0),
            Handle::N => (mx, y0),
            Handle::NE => (x1, y0),
            Handle::E => (x1, my),
            Handle::SE => (x1, y1),
            Handle::S => (mx, y1),
            Handle::SW => (x0, y1),
            Handle::W => (x0, my),
        })
    }

    /// All handle positions for `r`, deduped (corners first).
    pub fn all(r: &Rect) -> Vec<(Handle, (i32, i32))> {
        let mut out = Vec::new();
        let mut seen = std::collections::HashSet::new();
        for h in [
            Handle::NW,
            Handle::NE,
            Handle::SE,
            Handle::SW,
            Handle::N,
            Handle::E,
            Handle::S,
            Handle::W,
        ] {
            if let Some(p) = h.pos(r) {
                if seen.insert(p) {
                    out.push((h, p));
                }
            }
        }
        out
    }

    /// Handle occupying doc cell `(x, y)` in `r`, if any.
    pub fn at(r: &Rect, x: i32, y: i32) -> Option<Handle> {
        Self::all(r)
            .into_iter()
            .find(|(_, p)| *p == (x, y))
            .map(|(h, _)| h)
    }

    /// Opposite corner (for rubber-adjust drags started on a handle).
    pub fn opposite_corner(self, r: &Rect) -> (i32, i32) {
        let (x0, y0) = (r.x, r.y);
        let (x1, y1) = (r.x + r.w as i32 - 1, r.y + r.h as i32 - 1);
        match self {
            Handle::NW => (x1, y1),
            Handle::N => (x1, y1),
            Handle::NE => (x0, y1),
            Handle::E => (x0, y1),
            Handle::SE => (x0, y0),
            Handle::S => (x0, y0),
            Handle::SW => (x1, y0),
            Handle::W => (x1, y0),
        }
    }
}

/// In-progress select-tool drag (Phase 3). Rubber-band uses the legacy
/// anchor/drawing flow with `select_drag == None`.
#[derive(Clone, Debug)]
pub enum SelectDrag {
    /// Block move: `origin` rect on the active layer, grab offset from the
    /// press point to the origin top-left, `moved` once the cursor leaves
    /// the press cell. `widget` is the moved entity, if any.
    Move {
        origin: Rect,
        grab_dx: i32,
        grab_dy: i32,
        moved: bool,
        widget: Option<usize>,
    },
    /// Widget corner resize: fixed `start` rect, dragged corner.
    ResizeWidget {
        idx: usize,
        corner: Handle,
        start: Rect,
    },
    /// Line-tip reshape: `tip` follows the cursor, redrawn from `anchor`.
    Tip {
        tip: (i32, i32),
        anchor: (i32, i32),
    },
    /// Widget stamp placement of `kind`.
    Stamp {
        kind: String,
    },
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

/// Why the dialog was opened: loading replaces the document, SaveAs
/// re-binds the current document.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum DialogPurpose {
    Load,
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
    /// `*.oframe` files (alpha, case-insensitive). Other files are
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
                } else if name.ends_with(".oframe") {
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
    /// Save → `cwd/filename` with `.oframe` appended when the user typed
    /// no (or another) extension.
    pub fn confirm_path(&self) -> Option<PathBuf> {
        use omaframe::model::FILE_EXTENSION;
        match self.mode {
            DialogMode::Open => self
                .selected
                .and_then(|s| self.entries.get(s))
                .filter(|e| !e.is_dir)
                .map(|e| self.cwd.join(&e.name)),
            DialogMode::Save => {
                let name = self.filename.trim();
                if name.is_empty() {
                    None
                } else {
                    let mut p = PathBuf::from(name);
                    if p.extension().and_then(|e| e.to_str())
                        != Some(FILE_EXTENSION)
                    {
                        p.set_extension(FILE_EXTENSION);
                    }
                    Some(self.cwd.join(p))
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
    pub glyph_category: usize,
    pub palette_search: String,
    pub search_focused: bool,
    pub colors_scroll: usize,
    pub preview_dark: bool,
    pub file_dialog: Option<FileDialog>,
    pub status_msg: String,
    pub text_buffer: String,
    // --- session state (not in the brief's field list, but required) ---
    pub file_path: Option<PathBuf>,
    pub dirty: bool,
    pub should_quit: bool,
    pub last_dir: Option<PathBuf>,
    pending_overwrite: Option<PathBuf>,
    /// Armed New-file discard confirm (first New press with dirty work).
    pending_new: bool,
    anchor: Option<(i32, i32)>,
    drawing: bool,
    text_start: Option<(i32, i32)>,
    pan_anchor: Option<(u16, u16, (i32, i32))>,
    /// Armed widget kind from the Widgets tab (next canvas drag stamps it;
    /// Shift keeps it armed for repeats).
    pub pending_widget: Option<String>,
    /// Widget entity backing the live selection, if any.
    pub selected_widget: Option<usize>,
    /// In-progress select/stamp drag (rubber-band uses anchor + None).
    select_drag: Option<SelectDrag>,
    /// Live selection captured at press time (move-begin detection +
    /// opposite-corner rubber adjust).
    sel_at_press: Option<Rect>,
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
            fg: PaintColor::Theme("foreground".to_string()),
            bg: None,
            box_style: draw::BoxStyle::Light,
            arrow: false,
            cursor: (0, 0),
            viewport: (0, 0),
            palette_tab,
            palette_scroll: 0,
            glyph_category: 0,
            palette_search: String::new(),
            search_focused: false,
            colors_scroll: 0,
            preview_dark,
            file_dialog: None,
            status_msg: "click-drag draws · wheel scrolls · middle-drag pans · right-click grabs · Ctrl-S saves"
                .to_string(),
            text_buffer: String::new(),
            file_path,
            dirty: false,
            should_quit: false,
            last_dir: Self::read_lastdir(),
            pending_overwrite: None,
            pending_new: false,
            anchor: None,
            drawing: false,
            text_start: None,
            pan_anchor: None,
            pending_widget: None,
            selected_widget: None,
            select_drag: None,
            sel_at_press: None,
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
        match &self.fg {
            PaintColor::Ansi(i) => i.to_string(),
            PaintColor::Theme(name) => name.clone(),
            rgb => rgb.to_hex(),
        }
    }

    /// Background pot label for the status bar (`None` → `"-"`).
    pub fn bg_label(&self) -> String {
        match &self.bg {
            None => "-".to_string(),
            Some(PaintColor::Ansi(i)) => i.to_string(),
            Some(PaintColor::Theme(name)) => name.clone(),
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
        self.clear_search();
        self.doc.palette_tab = PALETTE_TAB_IDS[self.palette_tab].to_string();
        self.set_status(format!("palette: {}", PALETTE_TABS[self.palette_tab]));
    }

    pub fn set_palette_tab(&mut self, idx: usize) {
        if idx < PALETTE_TABS.len() {
            self.palette_tab = idx;
            self.palette_scroll = 0;
            self.clear_search();
            self.doc.palette_tab = PALETTE_TAB_IDS[idx].to_string();
        }
    }

    // --- glyph categories + palette search ---

    /// Glyphs passing the current category + search filters (single source
    /// for render, hit-testing, and scroll math on tab 5).
    pub fn visible_glyphs(&self) -> Vec<Glyph> {
        let query = self.palette_search.to_lowercase();
        all_glyphs()
            .iter()
            .filter(|g| {
                if self.glyph_category > 0 {
                    let want = GLYPH_CATEGORIES
                        .get(self.glyph_category)
                        .copied()
                        .unwrap_or("All");
                    if g.category != want {
                        return false;
                    }
                }
                if !query.is_empty() {
                    let q = query.as_str();
                    if !g.name.to_lowercase().contains(q)
                        && !g.ch.to_lowercase().contains(q)
                    {
                        return false;
                    }
                }
                true
            })
            .cloned()
            .collect()
    }

    /// What the palette grid shows for the active tab: glyphs go through
    /// category + search filters; every other tab filters its display
    /// strings by the search query (case-insensitive substring).
    pub fn palette_visible(&self) -> Vec<String> {
        if self.palette_tab == 5 {
            return self.visible_glyphs().into_iter().map(|g| g.ch).collect();
        }
        let items = palette_chars(self.palette_tab);
        let query = self.palette_search.to_lowercase();
        if query.is_empty() {
            return items;
        }
        items
            .into_iter()
            .filter(|s| s.to_lowercase().contains(query.as_str()))
            .collect()
    }

    /// Unfiltered item count for the active tab (the `/m` in the search
    /// row's `n/m` count).
    pub fn palette_total(&self) -> usize {
        if self.palette_tab == 5 {
            all_glyphs().len()
        } else {
            palette_chars(self.palette_tab).len()
        }
    }

    pub fn step_glyph_category(&mut self, dir: i32) {
        let n = GLYPH_CATEGORIES.len() as i32;
        self.glyph_category =
            (self.glyph_category as i32 + dir).rem_euclid(n) as usize;
        self.palette_scroll = 0;
    }

    pub fn focus_search(&mut self) {
        self.search_focused = true;
    }

    pub fn unfocus_search(&mut self) {
        self.search_focused = false;
    }

    pub fn clear_search(&mut self) {
        self.palette_search.clear();
        self.search_focused = false;
        self.palette_scroll = 0;
    }

    pub fn push_search(&mut self, c: char) {
        if !c.is_control() {
            self.palette_search.push(c);
            self.palette_scroll = 0;
        }
    }

    pub fn pop_search(&mut self) {
        self.palette_search.pop();
        self.palette_scroll = 0;
    }

    /// Scroll the palette grid by `dir` rows. `cols`/`visible` describe the
    /// on-screen grid (row count = ceil(items / cols)); the scroll clamps so
    /// the last content row stays reachable without scrolling into the void.
    pub fn scroll_palette(&mut self, dir: i32, cols: usize, visible: usize) {
        let len = self.palette_visible().len();
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

    /// Cycle the foreground pot through ANSI slots 0–15 (a non-slot pot
    /// resets into the ANSI cycle at 0).
    pub fn cycle_fg(&mut self) {
        let next = match self.fg {
            PaintColor::Ansi(i) => (i + 1) % 16,
            _ => 0,
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
            _ => None,
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
        // Phase 3 gestures + pending New confirm never survive a tool switch.
        self.pending_new = false;
        // Phase 3 gestures never survive a tool switch.
        self.select_drag = None;
        self.sel_at_press = None;
        self.selected_widget = None;
        self.pending_widget = None;
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

    /// Quit guard: clean docs quit immediately; dirty docs with a path
    /// save first (staying put when the save fails); dirty pathless docs
    /// open Save As instead of quitting so no work is silently lost.
    pub fn request_quit(&mut self) {
        if !self.dirty {
            self.should_quit = true;
            return;
        }
        if self.file_path.is_some() {
            if self.save() {
                self.should_quit = true;
            }
            return;
        }
        self.open_save_as_dialog();
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

    /// Own config dir (`~/.config/omaframe`, `XDG_CONFIG_HOME`-aware).
    fn config_dir() -> Option<PathBuf> {
        std::env::var("XDG_CONFIG_HOME")
            .ok()
            .map(PathBuf::from)
            .or_else(|| {
                std::env::var("HOME")
                    .ok()
                    .map(|h| PathBuf::from(h).join(".config"))
            })
            .map(|d| d.join("omaframe"))
    }

    fn lastdir_file() -> Option<PathBuf> {
        Self::config_dir().map(|d| d.join("lastdir"))
    }

    fn read_lastdir() -> Option<PathBuf> {
        let p = Self::lastdir_file()?;
        let s = std::fs::read_to_string(p).ok()?;
        let dir = PathBuf::from(s.trim());
        dir.is_dir().then_some(dir)
    }

    fn write_lastdir(dir: &std::path::Path) {
        if let Some(f) = Self::lastdir_file() {
            if let Some(parent) = f.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            let _ = std::fs::write(f, dir.to_string_lossy().as_bytes());
        }
    }

    /// Dialog start dir: the bound file's parent, else the last dialog
    /// directory (persisted), else `~/Documents` when it exists, else `~`
    /// (else the process cwd).
    fn dialog_start_dir(&self) -> PathBuf {
        if let Some(p) = &self.file_path {
            if let Some(parent) = p.parent() {
                if !parent.as_os_str().is_empty() {
                    return parent.to_path_buf();
                }
            }
        }
        if let Some(d) = &self.last_dir {
            if d.is_dir() {
                return d.clone();
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
        format!("{}.oframe", self.doc.name)
    }

    /// Menu → Load: open the Load dialog over the start dir. Dirty work is
    /// saved first when bound (or Save As opens when pathless) so nothing
    /// is silently discarded.
    pub fn open_load_dialog(&mut self) {
        self.pending_new = false;
        if self.dirty {
            if self.file_path.is_some() {
                if !self.save() {
                    return;
                }
            } else {
                self.open_save_as_dialog();
                self.set_status("save first — then Load");
                return;
            }
        }
        let start = self.dialog_start_dir();
        self.pending_overwrite = None;
        self.file_dialog = Some(FileDialog::new(
            DialogMode::Open,
            DialogPurpose::Load,
            start,
        ));
        self.set_status("load: pick a .oframe file");
    }

    /// Menu → New: two-step when dirty (first press arms, second press
    /// discards into a blank file), immediate blank reinit when clean.
    pub fn new_file_request(&mut self) {
        if self.dirty && !self.pending_new {
            self.pending_new = true;
            self.set_status("unsaved changes — New again to discard, Esc cancels");
            return;
        }
        self.pending_new = false;
        self.new_blank();
    }

    /// Fresh blank 80×24 canvas: unbound (save later via Save/SaveAs),
    /// clean, cursor home, no selection.
    pub fn new_blank(&mut self) {
        self.doc = Document::new("untitled", 80, 24);
        self.preview_dark = self.doc.preview_dark;
        self.palette_tab = palette_tab_index(&self.doc.palette_tab);
        self.history = History::new();
        self.scratch.clear();
        self.cursor = (0, 0);
        self.viewport = (0, 0);
        self.palette_scroll = 0;
        self.file_path = None;
        self.dirty = false;
        self.selected_widget = None;
        self.select_drag = None;
        self.sel_at_press = None;
        self.anchor = None;
        self.drawing = false;
        self.set_status("new blank canvas");
    }

    /// Open the SaveAs dialog (re-bind the current document on confirm).
    pub fn open_save_as_dialog(&mut self) {
        self.pending_new = false;
        let start = self.dialog_start_dir();
        let mut d = FileDialog::new(DialogMode::Save, DialogPurpose::SaveAs, start);
        d.filename = self.save_dialog_filename();
        self.file_dialog = Some(d);
        self.set_status("save as: pick a folder + name");
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

    /// Confirm the open dialog: Load → [`App::load_file_path`], SaveAs →
    /// [`App::save_as_path`]. SaveAs onto an existing file needs two
    /// consecutive confirms (overwrite guard); anything else that changes
    /// the target clears the pending state. Closes the dialog on success
    /// and remembers the directory.
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
        if purpose == DialogPurpose::SaveAs
            && path.exists()
            && self.pending_overwrite.as_ref() != Some(&path)
        {
            self.pending_overwrite = Some(path);
            self.set_status("exists — Enter again to overwrite");
            return;
        }
        self.pending_overwrite = None;
        let ok = match purpose {
            DialogPurpose::Load => self.load_file_path(path),
            DialogPurpose::SaveAs => self.save_as_path(path),
        };
        if ok {
            if let Some(dlg) = self.file_dialog.as_ref() {
                self.last_dir = Some(dlg.cwd.clone());
                Self::write_lastdir(&dlg.cwd);
            }
            self.file_dialog = None;
        }
    }

    /// Dismiss the open dialog (no action).
    pub fn dialog_cancel(&mut self) {
        self.pending_overwrite = None;
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

    /// Square constraint around a fixed corner: expand the shorter axis.
    fn square_from_corner(
        start: &Rect,
        corner: Handle,
        cur: (i32, i32),
    ) -> (i32, i32) {
        let (fx, fy) = corner.opposite_corner(start);
        let (nx, ny) = Self::constrain_square_anchor((fx, fy), cur);
        (nx, ny)
    }

    /// Normalize a widget resize rect: at least 1×1; single-row kinds
    /// (template height 1) stay 1 tall so handles cannot stretch text rows.
    fn normalize_widget_rect(idx: usize, doc: &Document, r: Rect) -> Rect {
        let single_row = doc
            .widgets
            .get(idx)
            .and_then(|w| omaframe::widgets::spec(&w.kind))
            .is_some_and(|s| s.default_h <= 1);
        let (w, mut h) = (r.w.max(1), r.h.max(1));
        if single_row {
            h = 1;
        }
        Rect::new(r.x, r.y, w, h)
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
                tinted.set(x, y, Cell::new(c.ch, self.fg.clone(), self.bg.clone()));
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

    // --- Phase 3: widgets + selection handles ---

    /// Topmost widget entity containing `(x, y)`, if any.
    pub fn widget_at(&self, x: i32, y: i32) -> Option<usize> {
        self.doc
            .widgets
            .iter()
            .rposition(|w| {
                x >= w.rect.x
                    && y >= w.rect.y
                    && x < w.rect.x + w.rect.w as i32
                    && y < w.rect.y + w.rect.h as i32
            })
    }

    /// The selected widget, when the live selection is exactly its rect.
    fn selected_widget_for(&self, sel: &Rect) -> Option<usize> {
        self.selected_widget.filter(|idx| {
            self.doc
                .widgets
                .get(*idx)
                .is_some_and(|w| w.rect == *sel)
        })
    }

    /// Arm a widget kind from the Widgets tab: the next canvas drag stamps
    /// it (click = default size). Shift keeps it armed for repeats.
    pub fn arm_widget(&mut self, kind: &str) {
        if omaframe::widgets::spec(kind).is_none() {
            return;
        }
        self.pending_widget = Some(kind.to_string());
        self.set_status(format!(
            "widget '{kind}': drag on canvas to stamp (Shift keeps it armed)"
        ));
    }

    /// Begin a widget stamp drag at a document cell. `keep_armed` (Shift)
    /// leaves the kind armed for another stamp.
    pub fn start_stamp(&mut self, x: i32, y: i32, keep_armed: bool) {
        let Some(kind) = self.pending_widget.clone() else {
            return;
        };
        if !keep_armed {
            self.pending_widget = None;
        }
        let (x, y) = self.resolve_guard(x, y);
        self.cursor = (x, y);
        self.clamp_cursor();
        self.anchor = Some((x, y));
        self.drawing = true;
        self.scratch.clear();
        self.select_drag = Some(SelectDrag::Stamp { kind });
        self.history.clear_selection();
        self.selected_widget = None;
        self.update_stamp(x, y);
    }

    /// Rebuild the stamp preview for `kind` over the anchor→cursor rect.
    /// Clicks (1×1) fall back to the catalog default size.
    fn update_stamp(&mut self, x: i32, y: i32) {
        let Some(SelectDrag::Stamp { kind }) = self.select_drag.clone() else {
            return;
        };
        let Some(a) = self.anchor else { return };
        self.cursor = (x, y);
        let mut r = Rect::from_points(a.0, a.1, x, y);
        if r.w <= 1 && r.h <= 1 {
            if let Some(spec) = omaframe::widgets::spec(&kind) {
                r = Rect::new(a.0, a.1, spec.default_w, spec.default_h);
            }
        }
        self.scratch = self.bake_widget_layer(&kind, &r);
    }

    /// Bake `kind` into a scratch layer over `r` with the current pots.
    fn bake_widget_layer(&self, kind: &str, r: &Rect) -> Layer {
        let spec = match omaframe::widgets::spec(kind) {
            Some(s) => s,
            None => return Layer::new(),
        };
        let style = spec.styles.first().cloned().unwrap_or_default();
        let baked = omaframe::widgets::bake(
            kind,
            r.w,
            r.h,
            &spec.label,
            &style,
            self.fg.clone(),
            self.bg.clone(),
        )
        .unwrap_or_default();
        let mut layer = Layer::new();
        for ((dx, dy), cell) in baked {
            layer.set(r.x + dx, r.y + dy, cell);
        }
        layer
    }

    /// Preview layer for resizing widget `idx` to `r` (own colors, never
    /// the pots — resize must not recolor).
    fn bake_widget_preview(&self, idx: usize, r: &Rect) -> Layer {
        let (kind, label, style, fg, bg) = match self.doc.widgets.get(idx) {
            Some(w) => {
                let (fg, bg) = self.widget_colors(idx);
                (w.kind.clone(), w.label.clone(), w.style.clone(), fg, bg)
            }
            None => return Layer::new(),
        };
        let baked =
            omaframe::widgets::bake(&kind, r.w, r.h, &label, &style, fg, bg)
                .unwrap_or_default();
        let mut layer = Layer::new();
        for ((dx, dy), cell) in baked {
            layer.set(r.x + dx, r.y + dy, cell);
        }
        layer
    }

    /// Commit a finished stamp: one undo entry for cells + widget entity.
    fn commit_stamp(&mut self, kind: String, r: Rect) {
        let widgets_before = Some(self.doc.widgets.clone());
        let patch = self.bake_widget_layer(&kind, &r);
        let spec = omaframe::widgets::spec(&kind);
        let (label, style) = match spec {
            Some(s) => (
                s.label.clone(),
                s.styles.first().cloned().unwrap_or_default(),
            ),
            None => (String::new(), String::new()),
        };
        let rect = Rect::new(r.x, r.y, r.w.max(1), r.h.max(1));
        self.doc.widgets.push(Widget {
            kind: kind.clone(),
            rect,
            label,
            style,
        });
        let idx = self.doc.widgets.len() - 1;
        let layer_idx = self.doc.active;
        if self
            .history
            .commit_full(&mut self.doc, layer_idx, &patch, None, widgets_before)
        {
            self.dirty = true;
        }
        self.history.set_selection(Some(rect));
        self.selected_widget = Some(idx);
        self.set_status(format!("stamped {kind} {}x{}", rect.w, rect.h));
    }

    /// Non-transparent active-layer cells of the widget's current bake
    /// (used to erase exactly the widget on move/resize, never neighbours).
    fn widget_baked_cells(&self, idx: usize) -> Vec<((i32, i32), Cell)> {
        let Some(w) = self.doc.widgets.get(idx) else {
            return Vec::new();
        };
        let layer = self.doc.active_layer();
        let mut out = Vec::new();
        for y in w.rect.y..w.rect.y + w.rect.h as i32 {
            for x in w.rect.x..w.rect.x + w.rect.w as i32 {
                if let Some(c) = layer.get(x, y) {
                    out.push(((x, y), c));
                }
            }
        }
        out
    }

    /// Colors for re-baking widget `idx`: first baked cell's pots, else the
    /// current pots.
    fn widget_colors(&self, idx: usize) -> (PaintColor, Option<PaintColor>) {
        if let Some((_, c)) = self.widget_baked_cells(idx).into_iter().next() {
            (c.fg, c.bg)
        } else {
            (self.fg.clone(), self.bg.clone())
        }
    }

    /// Begin a left-drag gesture at a document cell.
    pub fn start_stroke(&mut self, x: i32, y: i32, _shift: bool) {
        self.pending_new = false;
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
                // Phase 3 dispatch order: handle → tip → inside-selection
                // move → widget select → rubber-band.
                self.sel_at_press = self.history.selection();
                self.selected_widget = self
                    .sel_at_press
                    .as_ref()
                    .and_then(|r| self.selected_widget_for(r));
                if let Some(sel) = self.sel_at_press {
                    // Corner handle on a widget: resize drag. Other
                    // handles: rubber-adjust from the opposite corner.
                    if let Some(h) = Handle::at(&sel, x, y) {
                        if h.is_corner() {
                            if let Some(idx) = self.selected_widget {
                                self.anchor = Some((x, y));
                                self.drawing = true;
                                self.scratch.clear();
                                let start = self.doc.widgets[idx].rect;
                                self.select_drag = Some(SelectDrag::ResizeWidget {
                                    idx,
                                    corner: h,
                                    start,
                                });
                                self.set_status("resize: drag a corner");
                                return;
                            }
                        }
                        let opp = h.opposite_corner(&sel);
                        self.anchor = Some(opp);
                        self.drawing = true;
                        self.scratch.clear();
                        self.selected_widget = None;
                        return;
                    }
                    // Inside the live selection: pending move (confirmed
                    // once the cursor leaves the press cell).
                    if sel.contains(x, y) {
                        let widget = self.selected_widget;
                        self.anchor = Some((x, y));
                        self.drawing = true;
                        self.scratch.clear();
                        self.select_drag = Some(SelectDrag::Move {
                            origin: sel,
                            grab_dx: x - sel.x,
                            grab_dy: y - sel.y,
                            moved: false,
                            widget,
                        });
                        return;
                    }
                }
                // Line endpoint: tip-reshape drag from its neighbour.
                let layer_idx = self.doc.active;
                if let Some((tip, anchor)) =
                    draw::line_endpoint(&self.doc, layer_idx, x, y)
                {
                    self.anchor = Some((x, y));
                    self.drawing = true;
                    self.scratch.clear();
                    self.select_drag = Some(SelectDrag::Tip { tip, anchor });
                    self.set_status("line tip: drag to reshape");
                    return;
                }
                // Widget body: select it (a follow-up drag moves it via the
                // inside-selection path above).
                if let Some(idx) = self.widget_at(x, y) {
                    let rect = self.doc.widgets[idx].rect;
                    self.history.set_selection(Some(rect));
                    self.selected_widget = Some(idx);
                    self.sel_at_press = Some(rect);
                    self.anchor = Some((x, y));
                    self.drawing = true;
                    self.scratch.clear();
                    self.set_status(format!(
                        "widget '{}' selected: drag to move, corners resize",
                        self.doc.widgets[idx].kind
                    ));
                    return;
                }
                // Rubber-band (legacy flow).
                self.selected_widget = None;
                self.sel_at_press = None;
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
        // Widget stamps bypass the active tool (armed from the Widgets
        // tab under any tool).
        if matches!(self.select_drag, Some(SelectDrag::Stamp { .. })) {
            self.update_stamp(x, y);
            return;
        }
        match self.tool {
            Tool::Pencil => {
                let dab = draw::paint_cells(&[(x, y)], &self.ch, self.fg.clone(), self.bg.clone());
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
                    patch.set(c.0, c.1, Cell::new(head, self.fg.clone(), self.bg.clone()));
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
                    draw::paint_cells(&draw::ellipse_cells(r), &self.ch, self.fg.clone(), self.bg.clone());
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
                    draw::paint_cells(&draw::rect_cells(r), &self.ch, self.fg.clone(), self.bg.clone());
            }
            Tool::Select => {
                // Active Phase 3 drag: update its preview.
                if self.select_drag.is_some() {
                    self.update_select_drag(x, y, shift);
                    return;
                }
                // Fresh press inside a selection (widget click or
                // inside-selection press): leaving the press cell begins
                // a move.
                if let Some(origin) = self.sel_at_press {
                    if let Some(a) = self.anchor {
                        if (x, y) != a && origin.contains(a.0, a.1) {
                            let widget = self.selected_widget;
                            self.select_drag = Some(SelectDrag::Move {
                                origin,
                                grab_dx: a.0 - origin.x,
                                grab_dy: a.1 - origin.y,
                                moved: false,
                                widget,
                            });
                            self.update_select_drag(x, y, shift);
                            return;
                        }
                    }
                }
                let r = Rect::from_points(a.0, a.1, x, y);
                self.history.set_selection(Some(r));
                self.set_status(format!("select {}x{}", r.w, r.h));
            }
            Tool::Text | Tool::Grab | Tool::Pan => {}
        }
    }

    /// Extend the live Phase 3 drag (move / resize / tip / stamp).
    fn update_select_drag(&mut self, x: i32, y: i32, shift: bool) {
        let (x, y) = self.resolve_guard(x, y);
        self.cursor = (x, y);
        let drag = match self.select_drag.clone() {
            Some(d) => d,
            None => return,
        };
        match drag {
            SelectDrag::Move {
                origin,
                grab_dx,
                grab_dy,
                widget,
                ..
            } => {
                let dx = x - (origin.x + grab_dx);
                let dy = y - (origin.y + grab_dy);
                let moved = dx != 0 || dy != 0;
                self.select_drag = Some(SelectDrag::Move {
                    origin,
                    grab_dx,
                    grab_dy,
                    moved,
                    widget,
                });
                if !moved {
                    self.scratch.clear();
                    return;
                }
                // Ghost preview: translated active-layer cells.
                let layer = self.doc.active_layer();
                let mut preview = Layer::new();
                for ((cx, cy), cell) in layer.entries() {
                    if cell.is_transparent() {
                        continue;
                    }
                    if origin.contains(cx, cy) {
                        preview.set(cx + dx, cy + dy, cell);
                    }
                }
                self.scratch = preview;
                self.set_status(format!("move {dx:+},{dy:+} · release to drop"));
            }
            SelectDrag::ResizeWidget { idx, corner, start } => {
                let (fx, fy) = corner.opposite_corner(&start);
                let (mut nx, mut ny) = (x, y);
                if shift {
                    // Square constraint around the fixed corner.
                    let (sx, sy) = Self::square_from_corner(&start, corner, (x, y));
                    nx = sx;
                    ny = sy;
                }
                let r = Rect::from_points(fx, fy, nx, ny);
                let r = Self::normalize_widget_rect(idx, &self.doc, r);
                self.cursor = (x, y);
                self.scratch = self.bake_widget_preview(idx, &r);
                self.set_status(format!("resize {}x{}", r.w, r.h));
            }
            SelectDrag::Tip { tip, anchor } => {
                let mut patch = self.tint(draw::draw_line(
                    anchor.0,
                    anchor.1,
                    x,
                    y,
                    false,
                ));
                let layer_idx = self.doc.active;
                draw::snap_patch(&self.doc, layer_idx, &mut patch);
                self.scratch = patch;
                let _ = tip;
                self.set_status("line tip: release to commit");
            }
            SelectDrag::Stamp { .. } => {
                self.update_stamp(x, y);
            }
        }
    }

    /// Release a Phase 3 drag: exactly one undo entry per gesture.
    fn end_select_drag(&mut self) {
        let drag = match self.select_drag.take() {
            Some(d) => d,
            None => return,
        };
        self.scratch.clear();
        match drag {
            SelectDrag::Move {
                origin,
                grab_dx,
                grab_dy,
                moved,
                widget,
            } => {
                if !moved {
                    // Click without drag: keep the selection.
                    self.set_status("select: kept");
                    return;
                }
                let dx = self.cursor.0 - (origin.x + grab_dx);
                let dy = self.cursor.1 - (origin.y + grab_dy);
                self.commit_move(origin, dx, dy, widget);
            }
            SelectDrag::ResizeWidget { idx, corner, start } => {
                let (fx, fy) = corner.opposite_corner(&start);
                let r = Rect::from_points(fx, fy, self.cursor.0, self.cursor.1);
                let r = Self::normalize_widget_rect(idx, &self.doc, r);
                self.commit_widget_resize(idx, r);
            }
            SelectDrag::Tip { tip, anchor } => {
                let (cx, cy) = (self.cursor.0, self.cursor.1);
                if (cx, cy) == tip {
                    self.set_status("line tip: unchanged");
                    return;
                }
                let mut patch = self.tint(draw::draw_line(
                    anchor.0, anchor.1, cx, cy, false,
                ));
                // Erase the stale tip unless the new line covers it.
                if patch.get(tip.0, tip.1).is_none() {
                    patch.set(tip.0, tip.1, Cell::erased());
                }
                let layer_idx = self.doc.active;
                draw::snap_patch(&self.doc, layer_idx, &mut patch);
                if self.history.commit(&mut self.doc, &patch) {
                    self.dirty = true;
                }
                self.history.clear_selection();
                self.selected_widget = None;
                self.set_status("line reshaped");
            }
            SelectDrag::Stamp { kind } => {
                let Some(a) = self.anchor else { return };
                let (cx, cy) = (self.cursor.0, self.cursor.1);
                let mut r = Rect::from_points(a.0, a.1, cx, cy);
                if r.w <= 1 && r.h <= 1 {
                    if let Some(spec) = omaframe::widgets::spec(&kind) {
                        r = Rect::new(a.0, a.1, spec.default_w, spec.default_h);
                    }
                }
                self.commit_stamp(kind, r);
            }
        }
    }

    /// Commit a block move: erase the origin cells, paint them translated.
    /// Snapped after the fact so moved boxes reconnect to neighbours.
    /// Two passes (erase all, then paint all): a translated target can
    /// overlap another origin cell, and paint must win regardless of
    /// `HashMap` iteration order.
    fn commit_move(&mut self, origin: Rect, dx: i32, dy: i32, widget: Option<usize>) {
        let widgets_before = widget.map(|_| self.doc.widgets.clone());
        let layer = self.doc.active_layer();
        let mut moving: Vec<((i32, i32), Cell)> = Vec::new();
        for ((cx, cy), cell) in layer.entries() {
            if cell.is_transparent() || !origin.contains(cx, cy) {
                continue;
            }
            moving.push(((cx, cy), cell));
        }
        let mut patch = Layer::new();
        for ((cx, cy), _) in &moving {
            patch.set(*cx, *cy, Cell::erased());
        }
        for ((cx, cy), cell) in &moving {
            patch.set(cx + dx, cy + dy, cell.clone());
        }
        // Widget entities ride along with their cells.
        if let Some(idx) = widget {
            if let Some(w) = self.doc.widgets.get_mut(idx) {
                w.rect.x += dx;
                w.rect.y += dy;
            }
        }
        let new_sel = Rect::new(origin.x + dx, origin.y + dy, origin.w, origin.h);
        let layer_idx = self.doc.active;
        if self.history.commit_full(
            &mut self.doc,
            layer_idx,
            &patch,
            None,
            widgets_before,
        ) {
            self.dirty = true;
        }
        self.history.set_selection(Some(new_sel));
        self.set_status(format!("moved {dx:+},{dy:+}"));
    }

    /// Commit a widget corner resize: erase the old bake exactly, bake the
    /// new rect with the widget's own colors.
    fn commit_widget_resize(&mut self, idx: usize, r: Rect) {
        let (kind, label, style, old_rect) = match self.doc.widgets.get(idx) {
            Some(w) => (
                w.kind.clone(),
                w.label.clone(),
                w.style.clone(),
                w.rect,
            ),
            None => return,
        };
        let (fg, bg) = self.widget_colors(idx);
        let widgets_before = Some(self.doc.widgets.clone());
        // Erase only cells matching the old bake (never neighbours).
        let old_bake = omaframe::widgets::bake(
            &kind,
            old_rect.w,
            old_rect.h,
            &label,
            &style,
            fg.clone(),
            bg.clone(),
        )
        .unwrap_or_default();
        let mut old_set = std::collections::HashSet::new();
        for ((dx, dy), _) in &old_bake {
            old_set.insert((old_rect.x + dx, old_rect.y + dy));
        }
        let mut patch = Layer::new();
        {
            let layer = self.doc.active_layer();
            for (x, y) in old_set {
                if let Some(c) = layer.get(x, y) {
                    // Only erase what the widget itself painted: compare
                    // against the old bake cell.
                    let matches = old_bake.iter().any(|((dx, dy), cell)| {
                        old_rect.x + dx == x && old_rect.y + dy == y && *cell == c
                    });
                    if matches {
                        patch.set(x, y, Cell::erased());
                    }
                }
            }
        }
        let new_bake = omaframe::widgets::bake(
            &kind, r.w, r.h, &label, &style, fg, bg,
        )
        .unwrap_or_default();
        for ((dx, dy), cell) in new_bake {
            patch.set(r.x + dx, r.y + dy, cell);
        }
        if let Some(w) = self.doc.widgets.get_mut(idx) {
            w.rect = Rect::new(r.x, r.y, r.w.max(1), r.h.max(1));
        }
        let layer_idx = self.doc.active;
        if self.history.commit_full(
            &mut self.doc,
            layer_idx,
            &patch,
            None,
            widgets_before,
        ) {
            self.dirty = true;
        }
        let rect = self.doc.widgets[idx].rect;
        self.history.set_selection(Some(rect));
        self.selected_widget = Some(idx);
        self.set_status(format!("resized {}x{}", rect.w, rect.h));
    }

    /// Release: commit one undo entry (or nothing for no-ops).
    pub fn end_stroke(&mut self) {
        if !self.drawing {
            return;
        }
        // Phase 3 select/stamp drags commit through their own paths.
        if self.select_drag.is_some() {
            self.end_select_drag();
            self.anchor = None;
            self.drawing = false;
            self.sel_at_press = None;
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
                }
                self.anchor = None;
                self.drawing = false;
            }
            Tool::Select => {
                // Click without drag clears — unless the press began
                // inside a selection or on a widget (then it keeps it);
                // rubber-band keeps the rect and commits nothing.
                if self.sel_at_press.is_some() {
                    // kept
                } else if let Some(a) = self.anchor {
                    if a == self.cursor {
                        self.history.clear_selection();
                        self.selected_widget = None;
                        self.set_status("select: cleared");
                    }
                }
                self.anchor = None;
                self.drawing = false;
                self.sel_at_press = None;
            }
            Tool::Text | Tool::Grab | Tool::Pan => {
                self.anchor = None;
                self.drawing = false;
            }
        }
    }

    /// Esc: cancel a shape gesture, commit pending text (decisions.md),
    /// else clear drag state / disarm widget / clear the selection.
    pub fn cancel_stroke(&mut self) {
        if self.text_start.is_some() {
            self.commit_text();
            return;
        }
        if self.select_drag.is_some() {
            self.select_drag = None;
            self.scratch.clear();
            self.anchor = None;
            self.drawing = false;
            self.sel_at_press = None;
            self.set_status("cancelled");
            return;
        }
        if self.drawing {
            self.scratch.clear();
            self.anchor = None;
            self.drawing = false;
            self.set_status("cancelled");
            return;
        }
        if self.pending_widget.take().is_some() {
            self.set_status("widget disarmed");
            return;
        }
        if self.pending_new {
            self.pending_new = false;
            self.set_status("cancelled");
            return;
        }
        if self.history.selection().is_some() {
            self.history.clear_selection();
            self.selected_widget = None;
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
                let bg = match &self.bg {
                    None => "-".to_string(),
                    Some(PaintColor::Ansi(i)) => i.to_string(),
                    Some(PaintColor::Theme(name)) => name.clone(),
                    Some(rgb) => rgb.to_hex(),
                };
                let fg = match &self.fg {
                    PaintColor::Ansi(i) => i.to_string(),
                    PaintColor::Theme(name) => name.clone(),
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
        let dab = draw::paint_cells(&[(self.cursor.0, self.cursor.1)], &s, self.fg.clone(), self.bg.clone());
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
        assert!(!palette_chars(5).is_empty()); // full glyph list
    }

    #[test]
    fn glyph_data_is_big_clean_and_categorized() {
        use std::collections::HashSet;
        let all = all_glyphs();
        // Well beyond the old 104-nerd list.
        assert!(all.len() > 1000, "glyph count: {}", all.len());
        // Every entry single-cell, non-blank, categorized in the stepper.
        let cats: HashSet<&str> = GLYPH_CATEGORIES.iter().copied().collect();
        let mut codes = HashSet::new();
        for g in all.iter() {
            assert_eq!(unicode_width::UnicodeWidthStr::width(g.ch.as_str()), 1, "wide: {}", g.name);
            assert!(!g.name.is_empty() && !g.ch.is_empty());
            assert!(cats.contains(g.category.as_str()), "unknown cat: {}", g.category);
            assert!(codes.insert(g.ch.clone()), "dup char: {}", g.name);
        }
        // Category stepper covers the file: every category non-empty.
        for cat in GLYPH_CATEGORIES.iter().skip(1) {
            assert!(
                all.iter().any(|g| &g.category == cat),
                "empty category: {cat}"
            );
        }
    }

    #[test]
    fn glyph_filter_category_and_search() {
        let mut app = App::new(test_doc(), None);
        app.set_palette_tab(5);
        let total = app.palette_total();
        assert_eq!(total, all_glyphs().len());
        assert_eq!(app.palette_visible().len(), total);
        // Category narrows.
        let arrows = GLYPH_CATEGORIES.iter().position(|c| *c == "Arrows").unwrap();
        app.glyph_category = arrows;
        let shown = app.palette_visible();
        assert!(!shown.is_empty() && shown.len() < total);
        // Search narrows within the category; case-insensitive.
        app.palette_search = "ARROW".to_string();
        let keys = app.palette_visible();
        assert!(!keys.is_empty() && keys.len() <= shown.len());
        // Clearing restores everything; stepper wraps both directions.
        app.clear_search();
        assert!(!app.search_focused);
        assert_eq!(app.palette_visible().len(), shown.len());
        app.step_glyph_category(1);
        app.step_glyph_category(-1);
        assert_eq!(app.glyph_category, arrows);
        app.glyph_category = 0;
        app.step_glyph_category(-1);
        assert_eq!(app.glyph_category, GLYPH_CATEGORIES.len() - 1);
        // Non-glyph tabs filter their own display strings.
        app.set_palette_tab(0);
        app.palette_search = "a".to_string();
        let letters = app.palette_visible();
        assert!(letters.contains(&"A".to_string()));
        assert!(letters.contains(&"a".to_string()));
        assert!(!letters.contains(&"B".to_string()));
        // No-match state.
        app.palette_search = "zzz_no_such_glyph".to_string();
        assert!(app.palette_visible().is_empty());
        // Typing helpers reset scroll.
        app.palette_scroll = 99;
        app.push_search('x');
        assert_eq!(app.palette_scroll, 0);
        app.pop_search();
        app.focus_search();
        assert!(app.search_focused);
        app.unfocus_search();
        assert!(!app.search_focused);
    }

    #[test]
    fn truecolor_pots_cycle_set_and_label() {
        let mut app = App::new(test_doc(), None);
        // Default pencil follows the live theme foreground variable.
        assert_eq!(app.fg, PaintColor::Theme("foreground".to_string()));
        assert_eq!(app.fg_label(), "foreground");
        assert_eq!(app.bg, None);
        assert_eq!(app.bg_label(), "-");
        // fg cycles Ansi 0-15 with wraparound (from any non-slot pot too).
        app.cycle_fg();
        assert_eq!(app.fg, PaintColor::Ansi(0));
        for _ in 0..15 {
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
        std::fs::write(root.join("sub").join("a.oframe"), "{}").unwrap();
        std::fs::write(root.join("b.oframe"), "{}").unwrap();
        std::fs::write(root.join("ignore.txt"), "x").unwrap();

        // Open dialog over the root: dirs first, then *.oframe files.
        let mut d = FileDialog::new(DialogMode::Open, DialogPurpose::Load, root.clone());
        assert_eq!(d.cwd, root);
        let names: Vec<(&str, bool)> = d
            .entries
            .iter()
            .map(|e| (e.name.as_str(), e.is_dir))
            .collect();
        assert_eq!(names, vec![("sub", true), ("b.oframe", false)]);
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
        d.goto(root.join("sub").join("a.oframe"));
        assert_eq!(d.cwd, root.join("sub"));
        assert_eq!(d.selected, Some(1)); // [nested(dir), a.oframe]
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
            .position(|e| !e.is_dir && e.name == "b.oframe")
            .unwrap();
        d.selected = Some(file_idx);
        assert_eq!(d.confirm_path(), Some(root.join("b.oframe")));
        d.selected = Some(0); // a dir ("New Folder")
        assert!(d.entries[0].is_dir);
        assert_eq!(d.confirm_path(), None);
        d.selected = None;
        assert_eq!(d.confirm_path(), None);

        // Save confirm: blank filename → None, else cwd/filename.
        let mut s = FileDialog::new(DialogMode::Save, DialogPurpose::SaveAs, root.clone());
        assert_eq!(s.confirm_path(), None);
        for c in "n.oframe".chars() {
            s.type_char(c);
        }
        assert_eq!(s.filename, "n.oframe");
        assert_eq!(s.confirm_path(), Some(root.join("n.oframe")));
        s.backspace();
        assert_eq!(s.filename, "n.ofram");
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
    fn new_file_request_confirms_then_blanks() {
        // Clean doc: immediate blank reinit, no dialog, no questions.
        let mut app = App::new(test_doc(), None);
        app.new_file_request();
        assert!(!app.pending_new);
        assert_eq!(app.file_path, None);
        assert!(!app.dirty);
        assert_eq!(app.doc.name, "untitled");
        assert_eq!(app.cursor, (0, 0));
        // Dirty doc: first press arms, keeps everything.
        app.start_stroke(2, 2, false);
        app.end_stroke();
        assert!(app.dirty);
        app.new_file_request();
        assert!(app.pending_new, "armed");
        assert!(app.dirty, "work kept while armed");
        assert!(app.doc.cell(2, 2).is_some());
        assert!(app.status_msg.contains("New again"));
        // Drawing instead cancels the arm.
        app.start_stroke(3, 3, false);
        app.end_stroke();
        assert!(!app.pending_new);
        // Re-arm, then confirm: blank reinit, unbound, clean.
        app.new_file_request();
        app.new_file_request();
        assert!(!app.pending_new);
        assert_eq!(app.file_path, None);
        assert!(!app.dirty);
        assert_eq!(app.doc.cell(2, 2), None, "blanked");
        assert_eq!(app.doc.cell(3, 3), None, "blanked");
        assert_eq!(app.history.selection(), None);
        // Esc cancels the arm without losing work.
        app.start_stroke(4, 4, false);
        app.end_stroke();
        app.new_file_request();
        assert!(app.pending_new);
        app.cancel_stroke();
        assert!(!app.pending_new);
        assert!(app.dirty, "work kept after Esc");
        assert!(app.doc.cell(4, 4).is_some());
    }

    #[test]
    fn save_as_flow_and_load_round_trip() {
        let root = tempdir("dlg-rt");
        // SaveAs flow through dialog_confirm binds + saves.
        let mut app = App::new(test_doc(), None);
        app.start_stroke(2, 3, false);
        app.update_stroke(4, 3, false);
        app.end_stroke();
        app.open_save_as_dialog();
        assert!(app.file_dialog.is_some());
        assert_eq!(
            app.file_dialog.as_ref().unwrap().purpose,
            DialogPurpose::SaveAs
        );
        app.file_dialog.as_mut().unwrap().goto(root.clone());
        app.file_dialog.as_mut().unwrap().filename.clear();
        for c in "rt.oframe".chars() {
            app.dialog_type(c);
        }
        app.dialog_confirm();
        assert!(app.file_dialog.is_none(), "dialog closes on success");
        let path = root.join("rt.oframe");
        assert_eq!(app.file_path, Some(path.clone()));
        assert!(path.exists(), "save_as writes immediately");

        // Fresh cells + commit mark dirty WITHOUT writing (explicit save
        // model), then load the file back in a fresh app via dialog.
        // (Fresh cells: the pre-save stroke already occupies (2,3)-(4,3).)
        app.start_stroke(5, 5, false);
        app.update_stroke(6, 5, false);
        app.end_stroke();
        assert!(app.dirty, "stroke marks dirty");
        let on_disk = omaframe::model::load_file(&path).unwrap();
        assert!(on_disk.cell(5, 5).is_none(), "no autosave: disk unchanged");
        assert!(app.save(), "explicit save writes");
        let on_disk = omaframe::model::load_file(&path).unwrap();
        assert!(on_disk.cell(5, 5).is_some(), "saved stroke on disk");

        let mut app2 = App::new(test_doc(), None);
        app2.open_load_dialog();
        app2.file_dialog.as_mut().unwrap().goto(root.clone());
        {
            let dlg = app2.file_dialog.as_ref().unwrap();
            let idx = dlg
                .entries
                .iter()
                .position(|e| !e.is_dir && e.name == "rt.oframe")
                .unwrap();
            app2.file_dialog.as_mut().unwrap().selected = Some(idx);
        }
        app2.dialog_enter(); // file in Open mode confirms immediately
        assert!(app2.file_dialog.is_none());
        assert_eq!(app2.file_path, Some(path.clone()));
        assert!(app2.doc.cell(2, 3).is_some(), "loaded pre-save cell");
        assert!(app2.doc.cell(5, 5).is_some(), "loaded post-save cell");

        // Failure keeps the dialog open: blank SaveAs name confirms nothing.
        app2.menu_save(); // path bound → plain save, no dialog
        assert!(app2.file_dialog.is_none());
        let mut app3 = App::new(test_doc(), None);
        app3.open_save_as_dialog();
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
    fn save_appends_oframe() {
        use omaframe::model::FILE_EXTENSION;
        assert_eq!(FILE_EXTENSION, "oframe");
        // Bare names gain the extension; correct/other extensions pass.
        let root = tempdir("ext");
        let mut d = FileDialog::new(
            DialogMode::Save,
            DialogPurpose::SaveAs,
            root.clone(),
        );
        d.filename = "foo".to_string();
        assert_eq!(d.confirm_path(), Some(root.join("foo.oframe")));
        d.filename = "foo.oframe".to_string();
        assert_eq!(d.confirm_path(), Some(root.join("foo.oframe")));
        d.filename = "foo.txt".to_string();
        assert_eq!(d.confirm_path(), Some(root.join("foo.oframe")));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn overwrite_needs_two_confirms_and_quit_guards_dirty() {
        let root = tempdir("overwrite");
        let path = root.join("exists.oframe");
        std::fs::write(&path, "{}").unwrap();
        // SaveAs onto an existing file: first confirm warns, dialog stays.
        let mut app = App::new(test_doc(), None);
        app.open_save_as_dialog();
        app.file_dialog.as_mut().unwrap().goto(root.clone());
        app.file_dialog.as_mut().unwrap().filename.clear();
        for c in "exists.oframe".chars() {
            app.dialog_type(c);
        }
        app.dialog_confirm();
        assert!(app.file_dialog.is_some(), "first confirm warns only");
        assert!(app.status_msg.contains("overwrite"));
        app.dialog_confirm();
        assert!(app.file_dialog.is_none(), "second confirm writes");
        assert_eq!(app.file_path, Some(path.clone()));
        // Quit guard: clean quits, dirty+path saves+quits, dirty pathless
        // opens Save instead.
        let mut clean = App::new(test_doc(), Some(path.clone()));
        clean.request_quit();
        assert!(clean.should_quit);
        let mut dirty = App::new(test_doc(), Some(path.clone()));
        dirty.start_stroke(1, 1, false);
        dirty.end_stroke();
        assert!(dirty.dirty);
        dirty.request_quit();
        assert!(dirty.should_quit, "bound dirty saves then quits");
        assert!(!dirty.dirty);
        let mut homeless = App::new(test_doc(), None);
        homeless.start_stroke(1, 1, false);
        homeless.end_stroke();
        homeless.request_quit();
        assert!(!homeless.should_quit, "pathless does not quit");
        assert!(homeless.file_dialog.is_some(), "Save dialog opens");
        assert_eq!(
            homeless.file_dialog.as_ref().unwrap().purpose,
            DialogPurpose::SaveAs
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn loader_rejects_non_oframe_files() {
        let root = tempdir("ext-guard");
        let txt = root.join("notes.txt");
        std::fs::write(&txt, "{}").unwrap();
        assert!(omaframe::model::load_file(&txt).is_err());
        // Loader is case-insensitive on the suffix.
        let upper = root.join("doc.OFRAME");
        std::fs::write(&upper, r#"{"version":1,"name":"x","grid":{"w":5,"h":5},"layers":[{"name":"l","visible":true,"cells":[]}]}"#).unwrap();
        assert!(omaframe::model::load_file(&upper).is_ok());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn strokes_mark_dirty_without_autosave() {
        let root = tempdir("autosave");
        let path = root.join("a.oframe");
        let mut app = App::new(test_doc(), Some(path.clone()));
        assert!(!path.exists());
        app.start_stroke(1, 1, false);
        app.update_stroke(2, 1, false);
        app.end_stroke();
        assert!(app.dirty, "stroke marks dirty");
        assert!(!path.exists(), "explicit model: no autosave on commit");
        // Undo/redo also stay in memory only.
        app.undo();
        assert!(app.dirty);
        assert!(!path.exists());
        app.redo();
        assert!(!path.exists());
        // Explicit save writes; quit guard then quits cleanly.
        assert!(app.save());
        assert!(!app.dirty);
        let doc = omaframe::model::load_file(&path).unwrap();
        assert!(doc.cell(1, 1).is_some());
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
        // New-file flow reinits blank + unbound (save later via Save As).
        app.start_stroke(1, 1, false);
        app.end_stroke();
        app.new_file_request();
        app.new_file_request();
        assert!(!app.dirty);
        assert_eq!(app.history.undo_len(), 0);
        assert_eq!(app.file_path, None);
        assert_eq!(app.doc.name, "untitled");
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
        // Pan tool shortcut + unbound new-file label.
        assert_eq!(Tool::from_shortcut('_'), Some(Tool::Pan));
        assert_eq!(app.full_path_label(), "untitled");
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

    // --- Phase 3: handles, stamps, moves, tips, resizes ---

    #[test]
    fn handle_geometry_corners_and_degenerate() {
        let r = Rect::new(2, 3, 5, 4);
        assert_eq!(Handle::at(&r, 2, 3), Some(Handle::NW));
        assert_eq!(Handle::at(&r, 6, 3), Some(Handle::NE));
        assert_eq!(Handle::at(&r, 6, 6), Some(Handle::SE));
        assert_eq!(Handle::at(&r, 2, 6), Some(Handle::SW));
        assert_eq!(Handle::at(&r, 4, 3), Some(Handle::N));
        assert_eq!(Handle::at(&r, 6, 5), Some(Handle::E));
        assert_eq!(Handle::at(&r, 4, 6), Some(Handle::S));
        assert_eq!(Handle::at(&r, 2, 5), Some(Handle::W));
        assert_eq!(Handle::at(&r, 4, 4), None, "interior is not a handle");
        assert_eq!(Handle::at(&r, 0, 0), None, "exterior is not a handle");
        // Corners win ties in all(); every position maps.
        assert_eq!(Handle::all(&r).len(), 8);
        // 1-wide column: edge handles collapse into corners; dedupe keeps
        // the first listed (SE wins the bottom cell over SW).
        let col = Rect::new(0, 0, 1, 5);
        let pts = Handle::all(&col);
        assert!(pts.len() < 8);
        assert_eq!(Handle::at(&col, 0, 0), Some(Handle::NW));
        assert_eq!(Handle::at(&col, 0, 4), Some(Handle::SE));
        // Empty rect: nothing.
        assert!(Handle::all(&Rect::new(0, 0, 0, 3)).is_empty());
        assert!(Handle::NW.is_corner());
        assert!(!Handle::N.is_corner());
        // Opposite corners anchor rubber-adjust drags.
        assert_eq!(Handle::NW.opposite_corner(&r), (6, 6));
        assert_eq!(Handle::SE.opposite_corner(&r), (2, 3));
    }

    #[test]
    fn stamp_places_widget_in_one_undo_entry() {
        let mut app = App::new(test_doc(), None);
        app.set_tool(Tool::Select);
        app.arm_widget("button");
        assert_eq!(app.pending_widget, Some("button".to_string()));
        // Drag-out rect 0,0 → 11,0 (12 wide).
        app.start_stamp(0, 0, false);
        assert!(app.pending_widget.is_none(), "disarmed after stamp");
        app.update_stroke(11, 0, false);
        let before = app.history.undo_len();
        app.end_stroke();
        assert_eq!(app.history.undo_len(), before + 1, "single entry");
        assert_eq!(app.doc.widgets.len(), 1);
        let w = &app.doc.widgets[0];
        assert_eq!(w.kind, "button");
        assert_eq!((w.rect.x, w.rect.y, w.rect.w), (0, 0, 12));
        // Baked cells exist on the active layer.
        assert!(app.doc.cell(0, 0).is_some());
        assert_eq!(app.doc.cell(0, 0).unwrap().ch, "[");
        // Selection follows the widget; widget is selected.
        assert_eq!(app.history.selection(), Some(w.rect));
        assert_eq!(app.selected_widget, Some(0));
        // Click (no drag) falls back to the default size.
        let mut app2 = App::new(test_doc(), None);
        app2.arm_widget("checkbox");
        app2.start_stamp(5, 5, false);
        app2.end_stroke();
        assert_eq!(app2.doc.widgets.len(), 1);
        assert_eq!(
            (app2.doc.widgets[0].rect.w, app2.doc.widgets[0].rect.h),
            (9, 1)
        );
        // Shift keeps the kind armed for repeats.
        let mut app3 = App::new(test_doc(), None);
        app3.arm_widget("close");
        app3.start_stamp(0, 0, true);
        app3.end_stroke();
        assert_eq!(app3.pending_widget, Some("close".to_string()));
        // Stamps work under any active tool (Pencil here, not Select).
        let mut app4 = App::new(test_doc(), None);
        app4.set_tool(Tool::Pencil);
        app4.arm_widget("close");
        app4.start_stamp(0, 0, false);
        app4.update_stroke(2, 0, false);
        app4.end_stroke();
        assert_eq!(app4.doc.widgets.len(), 1);
        assert_eq!(app4.doc.widgets[0].kind, "close");
        // Undo removes cells AND the entity together.
        app.undo();
        assert!(app.doc.widgets.is_empty());
        assert_eq!(app.doc.cell(0, 0), None);
        app.redo();
        assert_eq!(app.doc.widgets.len(), 1);
        assert!(app.doc.cell(0, 0).is_some());
    }

    #[test]
    fn select_moves_plain_blocks_in_one_entry() {
        let mut app = App::new(test_doc(), None);
        // Paint two cells, rubber-band them, drag the body +1,+1.
        app.set_tool(Tool::Pencil);
        app.ch = "x".to_string();
        app.start_stroke(2, 2, false);
        app.end_stroke();
        app.start_stroke(3, 3, false);
        app.end_stroke();
        app.set_tool(Tool::Select);
        app.start_stroke(1, 1, false);
        app.update_stroke(4, 4, false);
        app.end_stroke();
        let sel = app.history.selection().expect("rubber selection");
        assert_eq!((sel.x, sel.y, sel.w, sel.h), (1, 1, 4, 4));
        let undo_before = app.history.undo_len();
        // Press inside (not on a handle — (3,2) is interior, not a handle
        // of the 1,1+4x4 rect whose handles sit on the border).
        app.start_stroke(3, 2, false);
        app.update_stroke(4, 3, false);
        app.end_stroke();
        assert_eq!(app.history.undo_len(), undo_before + 1, "single entry");
        // Cells relocated +1,+1; origin erased.
        assert!(app.doc.cell(3, 3).is_some());
        assert!(app.doc.cell(4, 4).is_some());
        assert_eq!(app.doc.cell(2, 2), None);
        // Selection followed the move.
        assert_eq!(
            app.history.selection(),
            Some(Rect::new(2, 2, 4, 4))
        );
        app.undo();
        assert!(app.doc.cell(2, 2).is_some(), "undo restores origin");
        assert_eq!(app.doc.cell(4, 4), None, "undo removes the move");
        assert!(app.doc.cell(3, 3).is_some(), "(3,3) was occupied pre-move");
    }

    #[test]
    fn select_click_without_drag_keeps_selection() {
        let mut app = App::new(test_doc(), None);
        app.set_tool(Tool::Select);
        app.start_stroke(0, 0, false);
        app.update_stroke(5, 5, false);
        app.end_stroke();
        assert!(app.history.selection().is_some());
        let undo_before = app.history.undo_len();
        // Click inside without moving: no new entry, selection kept.
        app.start_stroke(2, 2, false);
        app.end_stroke();
        assert_eq!(app.history.undo_len(), undo_before);
        assert!(app.history.selection().is_some());
        // Click outside clears (legacy rubber behavior).
        app.start_stroke(50, 50, false);
        app.end_stroke();
        assert_eq!(app.history.selection(), None);
    }

    #[test]
    fn tip_drag_reshapes_line_endpoints() {
        let mut app = App::new(test_doc(), None);
        // Horizontal line (0,0)→(4,0) on the active layer.
        app.set_tool(Tool::Line);
        app.start_stroke(0, 0, false);
        app.update_stroke(4, 0, false);
        app.end_stroke();
        assert_eq!(app.doc.cell(4, 0).unwrap().ch, "─");
        // Grab the (4,0) tip and drag it down to (4,3).
        app.set_tool(Tool::Select);
        app.start_stroke(4, 0, false);
        app.update_stroke(4, 3, false);
        let undo_before = app.history.undo_len();
        app.end_stroke();
        assert_eq!(app.history.undo_len(), undo_before + 1, "single entry");
        // Old tip erased, new leg present with a corner at the anchor.
        assert_eq!(app.doc.cell(4, 0), None, "stale tip erased");
        assert!(app.doc.cell(4, 3).is_some(), "new tip painted");
        assert!(app.doc.cell(3, 0).is_some(), "old run kept");
        app.undo();
        assert_eq!(app.doc.cell(4, 0).unwrap().ch, "─", "undo restores");
        assert_eq!(app.doc.cell(4, 3), None);
    }

    #[test]
    fn widget_resize_rebakes_and_moves_track_rect() {
        let mut app = App::new(test_doc(), None);
        app.set_tool(Tool::Select);
        app.arm_widget("button");
        app.start_stamp(0, 0, false);
        app.update_stroke(7, 0, false);
        app.end_stroke();
        assert_eq!(app.selected_widget, Some(0));
        // Corner drag SE: 8-wide → 12-wide.
        let r = app.doc.widgets[0].rect;
        app.start_stroke(r.x + r.w as i32 - 1, r.y + r.h as i32 - 1, false);
        app.update_stroke(11, 0, false);
        let undo_before = app.history.undo_len();
        app.end_stroke();
        assert_eq!(app.history.undo_len(), undo_before + 1, "single entry");
        assert_eq!(app.doc.widgets[0].rect.w, 12);
        // Body drag moves cells + entity together.
        let r = app.doc.widgets[0].rect;
        app.start_stroke(r.x + 2, r.y, false);
        app.update_stroke(r.x + 4, r.y + 2, false);
        app.end_stroke();
        assert_eq!(app.doc.widgets[0].rect.x, r.x + 2);
        assert_eq!(app.doc.widgets[0].rect.y, r.y + 2);
        assert_eq!(app.history.selection(), Some(app.doc.widgets[0].rect));
        app.undo();
        assert_eq!(app.doc.widgets[0].rect, r, "undo restores rect");
    }

    #[test]
    fn widget_click_selects_and_tool_switch_clears_phase3() {
        let mut app = App::new(test_doc(), None);
        app.set_tool(Tool::Select);
        app.arm_widget("close");
        app.start_stamp(10, 10, false);
        app.end_stroke();
        // Click the widget body: selects it.
        app.history.clear_selection();
        app.selected_widget = None;
        app.start_stroke(11, 10, false);
        app.end_stroke();
        assert_eq!(app.selected_widget, Some(0));
        assert_eq!(app.history.selection(), Some(app.doc.widgets[0].rect));
        // Switching tools clears gesture state.
        app.set_tool(Tool::Pencil);
        assert_eq!(app.history.selection(), None);
        assert_eq!(app.selected_widget, None);
        assert_eq!(app.pending_widget, None);
        // Esc disarms a pending widget.
        app.arm_widget("close");
        app.cancel_stroke();
        assert_eq!(app.pending_widget, None);
    }
}
