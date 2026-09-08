//! Document model for omaframe: grid + sparse layers + diff-stack undo.
//!
//! Port of `client/layer.ts` (`Layer` map + `apply` + `LayerView` top-wins)
//! and `client/store/canvas.ts` (`commitScratch` / undo / redo / selection
//! snapshots), extended from `Cell = char` to `Cell { ch, fg, bg }` with
//! ANSI-16 colors per `plan.md` §3.2 and `docs/model-api.md`.
//!
//! Key semantics (normative for the tools/TUI/export waves):
//!
//! - A composed cell is **transparent** iff its key is absent, its `ch` is
//!   empty (erased), a space, or NUL. Space and erased show lower layers
//!   through; the eraser writes [`Cell::erased`].
//! - Compose is **top-wins**: `layers[0]` is bottom, `layers[last]` is top,
//!   invisible layers are skipped, and the `scratch` argument renders above
//!   everything (no blend modes in v1).
//! - [`Layer::set`] stores exactly what it is given, **including**
//!   transparent (deletion-marker) cells — this is how eraser/text-backspace
//!   scratch layers express deletions, exactly like TS `Layer.set` storing
//!   `""`. Committed layers stay sparse because [`Layer::apply`] turns
//!   transparent patch values into key removals and [`load_json`] drops
//!   transparent file entries. Use [`Layer::compact`] to canonicalize any
//!   layer. (This differs from model-api §2's "transparent => remove key";
//!   see DIVERGENCE note below.)
//! - [`Layer::apply`] folds a patch into a layer and returns the inverse
//!   diff, or `None` when nothing changed (=> push nothing on the undo
//!   stack). A repaint with an identical cell and an erasure over empty
//!   space are both no-ops.
//! - Wide chars (width 2 via `unicode-width`): stored once at the anchor
//!   cell; `(x+1, y)` is a continuation guard. Painting a cell that is
//!   currently a continuation clears the wide anchor (no orphans); painting
//!   a wide char clears the covered `(x+1, y)` cell (last-writer-wins).
//!   Export skips guards; the cursor jumps over them via
//!   [`Document::cursor_step_x`].
//!
//! DIVERGENCES from `docs/model-api.md` (docs intentionally untouched):
//!
//! 1. `Cell.ch` is a `String`, not `Option<char>`, so one grapheme with a
//!    combining mark (schema `maxLength: 2`, e.g. `"e\u{301}"`) survives a
//!    load/save round-trip. `""`/`" "`/`"\0"` mean transparent; [`Cell`]
//!    is `Clone` but not `Copy`.
//! 2. `Layer::set` stores transparent markers (see above); sparse form for
//!    committed layers is enforced by `apply`/`load_json`, not by `set`.
//!    `keys`/`entries`/`len` therefore count raw entries (markers included,
//!    as in TS `map.size`); `get`/`has`/`compose` filter markers out.
//! 3. `Document` additionally carries `name` and `palette_tab` (both in the
//!    schema, needed for a faithful round-trip). The file's `active` layer
//!    is runtime-only: the schema has no such field, so saves omit it and
//!    loads reset it to `0`.
//! 4. `Widget` keeps schema-faithful `kind`/`label`/`style` strings instead
//!    of strict `WidgetKind`/`WidgetStyle`/`WidgetState` enums (the schema's
//!    `style` values such as `"checked"` are state, not a border style, so
//!    enums would lose data). Widgets are stored and round-tripped but NOT
//!    rendered by the model: export uses the baked cells already present in
//!    `layers`. Parametric bake-on-resize belongs to the widget-stamp tool.
//! 5. `History` additionally owns the live `selection` (mirrors TS
//!    `CanvasStore._selection`, which has no model-api home) so undo/redo
//!    can stash/restore it. `commit` returns `bool` (true when an entry was
//!    pushed) and any commit clears redo even when the diff is empty (both
//!    match TS `commitScratch`).
//! 6. `export_txt` skips fully-empty trailing rows (model-api §9); the skill
//!    doc's "emit all `grid.h` lines" differs there. The demo is unaffected
//!    (no trailing empty rows). `render_to_text` is a one-line alias.
//! 7. Loader tolerance (never silently lose data, never invent content):
//!    unknown keys ignored; grids beyond the 500×200 soft cap accepted;
//!    out-of-grid and negative coordinates preserved (render clips);
//!    `""`/space/NUL/zero-width entries dropped (they compose identically
//!    to absent). Loud errors (`Err`, never silent clamp) for: wrong
//!    `version`, out-of-range colors, multi-char `ch`, empty layers,
//!    zero grid dimensions, zero-size widgets.

use std::collections::HashMap;
use std::fmt;
use std::path::Path;

use serde::{Deserialize, Serialize};
use unicode_width::UnicodeWidthStr;

/// Undo depth cap (`MAX_UNDO`, tools-spec §0). TS grows unbounded in
/// practice; Rust truncates to 50 on every push to the undo stack.
pub const MAX_UNDO: usize = 50;

/// Soft grid caps (plan §3.2). Files beyond these load fine; the viewport
/// clips at render time.
pub const MAX_GRID_W: u32 = 500;
/// Soft grid caps (plan §3.2). Files beyond these load fine; the viewport
/// clips at render time.
pub const MAX_GRID_H: u32 = 200;

/// Current `.omaframe.json` format version. Anything else fails to load.
pub const FILE_VERSION: u32 = 1;

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

/// All model failures. Colors/shapes report the offending values so agents
/// see their mistakes (CLI maps any `Err` to a non-zero exit).
#[derive(Debug)]
pub enum ModelError {
    /// `version` field was not [`FILE_VERSION`].
    BadVersion(u32),
    /// `fg` not in `0..=15` or `bg` not in `-1..=15`.
    BadColor { fg: i32, bg: i32 },
    /// `ch` holds more than one spacing character.
    BadChar(String),
    /// Zero grid dimension, empty layer list, or similar document problem.
    BadGrid(String),
    /// Zero-size widget or similar geometry problem.
    BadGeometry(String),
    /// Malformed JSON / wrong JSON types.
    Json(serde_json::Error),
    /// File I/O failure in [`load_file`] / [`save_file`].
    Io(std::io::Error),
}

impl fmt::Display for ModelError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::BadVersion(v) => write!(f, "unsupported .omaframe.json version {v} (want 1)"),
            Self::BadColor { fg, bg } => {
                write!(f, "bad color fg={fg} bg={bg} (want fg 0-15, bg -1-15)")
            }
            Self::BadChar(ch) => write!(f, "bad ch {ch:?} (want a single character)"),
            Self::BadGrid(msg) => write!(f, "bad grid: {msg}"),
            Self::BadGeometry(msg) => write!(f, "bad geometry: {msg}"),
            Self::Json(e) => write!(f, "bad JSON: {e}"),
            Self::Io(e) => write!(f, "i/o error: {e}"),
        }
    }
}

impl std::error::Error for ModelError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Json(e) => Some(e),
            Self::Io(e) => Some(e),
            _ => None,
        }
    }
}

impl From<serde_json::Error> for ModelError {
    fn from(e: serde_json::Error) -> Self {
        Self::Json(e)
    }
}

impl From<std::io::Error> for ModelError {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e)
    }
}

// ---------------------------------------------------------------------------
// Width helpers
// ---------------------------------------------------------------------------

/// Display width of a string in terminal cells (`unicode-width`).
/// Returns 0 for empty/zero-width strings, 1 for normal glyphs (including
/// box drawing and most symbols), 2 for CJK and some Nerd glyphs.
pub fn width_of(s: &str) -> usize {
    s.width()
}

/// True when `s` occupies a continuation guard cell as well as its anchor
/// (display width >= 2).
pub fn is_wide(s: &str) -> bool {
    width_of(s) >= 2
}

/// True for the transparent spellings: empty (erased), space, NUL.
pub fn is_transparent_ch(s: &str) -> bool {
    s.is_empty() || s == " " || s == "\0"
}

// ---------------------------------------------------------------------------
// Cell
// ---------------------------------------------------------------------------

/// One painted cell.
///
/// `ch` holds a single grapheme (usually one `char`; a base char plus one
/// combining mark also round-trips). Empty / `" "` / `"\0"` all mean
/// transparent (see [`Cell::is_transparent`]). `fg` is an ANSI slot `0..=15`;
/// `bg` is `0..=15` or `-1` = transparent.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Cell {
    pub ch: String,
    pub fg: i8,
    pub bg: i8,
}

impl Cell {
    /// Build a cell. Accepts `char`, `&str`, or `String`. Like the TS
    /// `Layer.set`, this constructor does not validate; boundary checks
    /// live in [`Cell::validate`] (called by [`load_json`] and
    /// [`save_json`]).
    pub fn new(ch: impl Into<String>, fg: i8, bg: i8) -> Self {
        Self {
            ch: ch.into(),
            fg,
            bg,
        }
    }

    /// The erased cell: transparent, no colors.
    pub fn erased() -> Self {
        Self {
            ch: String::new(),
            fg: 0,
            bg: -1,
        }
    }

    /// Transparent iff `ch` is empty, a space, or NUL (model-api §3: key
    /// absent, `None`, `' '`, and `'\0'` are all transparent; space-filled
    /// and erased tails therefore export identically).
    pub fn is_transparent(&self) -> bool {
        is_transparent_ch(&self.ch)
    }

    /// Display width in terminal cells (0, 1, or 2 for well-formed cells).
    pub fn width(&self) -> usize {
        width_of(&self.ch)
    }

    /// Boundary check: colors in range, `ch` at most one spacing character
    /// (a lone base char, a wide char, or base + one combining mark).
    /// Transparent and zero-width cells are valid (they canonicalize away).
    pub fn validate(&self) -> Result<(), ModelError> {
        if !(0..=15).contains(&self.fg) || !(-1..=15).contains(&self.bg) {
            return Err(ModelError::BadColor {
                fg: self.fg as i32,
                bg: self.bg as i32,
            });
        }
        let nchars = self.ch.chars().count();
        if nchars > 2 || (nchars == 2 && width_of(&self.ch) > 1) {
            return Err(ModelError::BadChar(self.ch.clone()));
        }
        Ok(())
    }
}

impl Default for Cell {
    /// Defaults to [`Cell::erased`].
    fn default() -> Self {
        Self::erased()
    }
}

// ---------------------------------------------------------------------------
// Layer
// ---------------------------------------------------------------------------

/// Sparse layer: maps `(x, y)` grid coords to cells.
///
/// `set` stores values verbatim, including transparent deletion markers
/// (TS `Layer.set` semantics — the eraser builds scratch this way). Reads
/// (`get`/`has`) and compose treat markers as transparent; [`Layer::apply`]
/// turns them into removals; [`load_json`] never stores them; [`save_json`]
/// never writes them. [`Layer::compact`] drops them on demand.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Layer {
    cells: HashMap<(i32, i32), Cell>,
}

impl Layer {
    /// Empty layer.
    pub fn new() -> Self {
        Self::default()
    }

    /// Cell at `(x, y)`, or `None` when absent **or transparent**.
    pub fn get(&self, x: i32, y: i32) -> Option<Cell> {
        self.cells.get(&(x, y)).and_then(|c| {
            if c.is_transparent() {
                None
            } else {
                Some(c.clone())
            }
        })
    }

    /// Stored cell verbatim, **including** transparent deletion markers
    /// ([`get`](Self::get) filters those out). Used by the TUI to render
    /// eraser-ghost previews over scratch layers; never used for compose.
    pub fn get_raw(&self, x: i32, y: i32) -> Option<&Cell> {
        self.cells.get(&(x, y))
    }

    /// Store a cell verbatim, including transparent deletion markers used
    /// by scratch/patch layers. (Divergence from model-api §2, see module
    /// docs.)
    pub fn set(&mut self, x: i32, y: i32, c: Cell) {
        self.cells.insert((x, y), c);
    }

    /// True when a non-transparent cell is stored at `(x, y)`.
    pub fn has(&self, x: i32, y: i32) -> bool {
        self.get(x, y).is_some()
    }

    /// Raw stored keys, **including** deletion-marker keys in scratch/patch
    /// layers (mirrors TS `map.keys()`; committed layers canonicalized by
    /// `apply`/`load_json` hold no markers).
    pub fn keys(&self) -> impl Iterator<Item = (i32, i32)> + '_ {
        self.cells.keys().copied()
    }

    /// Raw stored entries, **including** deletion markers (needed by the TUI
    /// to render eraser ghosts and by `apply` to fold deletions).
    pub fn entries(&self) -> impl Iterator<Item = ((i32, i32), Cell)> + '_ {
        self.cells.iter().map(|(k, v)| (*k, v.clone()))
    }

    /// Drop all entries (markers included).
    pub fn clear(&mut self) {
        self.cells.clear();
    }

    /// Number of raw stored entries, markers included (mirrors TS
    /// `map.size`, which is what `setScratchLayer` uses to detect a gesture
    /// start).
    pub fn len(&self) -> usize {
        self.cells.len()
    }

    /// True when no entries at all are stored.
    pub fn is_empty(&self) -> bool {
        self.cells.is_empty()
    }

    /// Merge another layer's raw entries into this one (scratch builders,
    /// snap buffers). Markers overwrite content and vice versa.
    pub fn set_from(&mut self, other: &Layer) {
        for (k, v) in &other.cells {
            self.cells.insert(*k, v.clone());
        }
    }

    /// Drop transparent deletion markers, restoring canonical sparse form.
    pub fn compact(&mut self) {
        self.cells.retain(|_, c| !c.is_transparent());
    }

    /// Paint one non-transparent cell, maintaining the wide-char invariant:
    /// a paint landing on a continuation guard clears the wide anchor to
    /// its left (no orphans), and a wide paint clears the cell it covers to
    /// its right (last-writer-wins). Transparent paints just remove the key
    /// and never touch neighbours.
    fn paint_one(&mut self, x: i32, y: i32, c: &Cell) {
        if c.is_transparent() {
            self.cells.remove(&(x, y));
            return;
        }
        if let Some(left) = self.cells.get(&(x - 1, y)) {
            if !left.is_transparent() && left.width() >= 2 {
                self.cells.remove(&(x - 1, y));
            }
        }
        self.cells.insert((x, y), c.clone());
        if c.width() >= 2 {
            self.cells.remove(&(x + 1, y));
        }
    }

    /// Commit semantics: fold `patch` into `self`, returning the inverse
    /// diff, or `None` when nothing changed (=> push nothing on the undo
    /// stack). Transparent patch values delete keys. A key whose composed
    /// value is unchanged (identical repaint, erasure over empty space)
    /// contributes nothing to the diff. Wide-char neighbour clearing is
    /// part of the fold and is reflected in the diff, so undo restores the
    /// exact prior state. Patch entries are applied in `(y, x)` order for
    /// determinism.
    pub fn apply(&mut self, patch: &Layer) -> Option<Layer> {
        if patch.cells.is_empty() {
            return None;
        }
        // Snapshot every key the fold could touch: patch keys plus the
        // one-step neighbours a paint can clear.
        let mut affected: Vec<(i32, i32)> = Vec::with_capacity(patch.cells.len() * 2);
        for ((x, y), c) in &patch.cells {
            affected.push((*x, *y));
            if !c.is_transparent() {
                affected.push((*x - 1, *y));
                if c.width() >= 2 {
                    affected.push((*x + 1, *y));
                }
            }
        }
        affected.sort_unstable();
        affected.dedup();
        let before: Vec<Option<Cell>> = affected.iter().map(|(x, y)| self.get(*x, *y)).collect();

        let mut order: Vec<(i32, i32)> = patch.cells.keys().copied().collect();
        order.sort_unstable_by_key(|(x, y)| (*y, *x));
        for (x, y) in order {
            // `set` stored the patch verbatim, so the value is present.
            let c = patch.cells.get(&(x, y)).expect("patch key vanished");
            self.paint_one(x, y, c);
        }

        let mut undo = Layer::new();
        for ((x, y), old) in affected.into_iter().zip(before) {
            if self.get(x, y) != old {
                undo.cells.insert(
                    (x, y),
                    old.unwrap_or_else(Cell::erased),
                );
            }
        }
        if undo.cells.is_empty() {
            None
        } else {
            Some(undo)
        }
    }
}

// ---------------------------------------------------------------------------
// Rect
// ---------------------------------------------------------------------------

/// Cell bounding box: origin `(x, y)` plus size. Used for selections,
/// widget bounds, and [`export_selection`].
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub w: u32,
    pub h: u32,
}

impl Rect {
    /// Build from origin plus size.
    pub fn new(x: i32, y: i32, w: u32, h: u32) -> Self {
        Self { x, y, w, h }
    }

    /// Build from two corners (e.g. a rubber-band anchor and cursor),
    /// normalized so direction does not matter; both corners are inclusive,
    /// so `from_points(2, 2, 2, 2)` is a 1×1 rect.
    pub fn from_points(x0: i32, y0: i32, x1: i32, y1: i32) -> Self {
        Self {
            x: x0.min(x1),
            y: y0.min(y1),
            w: (x0 - x1).unsigned_abs() + 1,
            h: (y0 - y1).unsigned_abs() + 1,
        }
    }

    /// True when `(x, y)` lies inside (origin inclusive, far edge exclusive
    /// of `x + w` / `y + h`).
    pub fn contains(&self, x: i32, y: i32) -> bool {
        x >= self.x
            && y >= self.y
            && (x as i64) < self.x as i64 + self.w as i64
            && (y as i64) < self.y as i64 + self.h as i64
    }

    /// True when either dimension is zero.
    pub fn is_empty(&self) -> bool {
        self.w == 0 || self.h == 0
    }
}

// ---------------------------------------------------------------------------
// Widget
// ---------------------------------------------------------------------------

/// Parametric widget stamp (plan §4.4). Schema-faithful record: `kind`,
/// `label`, and `style` are opaque strings (the schema's `style` values
/// include state words like `"checked"`, so strict enums would lose data).
/// Widgets round-trip through load/save but are NOT rendered by the model;
/// export uses the baked cells in `layers`.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct Widget {
    /// Widget kind, e.g. `"button"`, `"panel"`, `"progress"`.
    pub kind: String,
    /// Cell bounding box.
    #[serde(flatten)]
    pub rect: Rect,
    /// Display label (default `""`).
    #[serde(default)]
    pub label: String,
    /// Style variant, e.g. `"rounded"`, `"checked"` (default `""`).
    #[serde(default)]
    pub style: String,
}

// ---------------------------------------------------------------------------
// Document
// ---------------------------------------------------------------------------

/// One named layer in the ordered stack (`layers[0]` is bottom).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NamedLayer {
    pub name: String,
    pub visible: bool,
    pub layer: Layer,
}

/// Ordered layer stack plus grid, widgets, and preview hints.
///
/// Invariants (maintained by [`Document::new`] and [`load_json`]):
/// `layers` is non-empty and `active < layers.len()`. Mutating the public
/// fields directly can break these; prefer [`Document::set_active`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Document {
    /// Wireframe name (schema `name`).
    pub name: String,
    /// Grid size `(w, h)`; new files default to 80×24.
    pub grid: (u32, u32),
    /// Layer stack, index 0 = bottom. Defaults: `["wireframe", "labels"]`.
    pub layers: Vec<NamedLayer>,
    /// Index of the layer tools paint into. Runtime-only: not serialized
    /// (the schema has no such field); loads reset it to `0`.
    pub active: usize,
    /// Parametric widget stamps (stored, not rendered by the model).
    pub widgets: Vec<Widget>,
    /// Dark/light canvas-background hint (chrome only, never cells).
    pub preview_dark: bool,
    /// Last-active palette tab hint (default `"outlines"`).
    pub palette_tab: String,
}

impl Document {
    /// New `w`×`h` document with empty `wireframe` + `labels` layers,
    /// dark preview, and the `outlines` palette tab. Zero dimensions clamp
    /// to 1.
    pub fn new(name: impl Into<String>, w: u32, h: u32) -> Self {
        Self {
            name: name.into(),
            grid: (w.max(1), h.max(1)),
            layers: vec![
                NamedLayer {
                    name: "wireframe".to_string(),
                    visible: true,
                    layer: Layer::new(),
                },
                NamedLayer {
                    name: "labels".to_string(),
                    visible: true,
                    layer: Layer::new(),
                },
            ],
            active: 0,
            widgets: Vec::new(),
            preview_dark: true,
            palette_tab: "outlines".to_string(),
        }
    }

    /// The layer tools paint into (panics if the `layers`/`active`
    /// invariant was broken by direct field mutation).
    pub fn active_layer(&self) -> &Layer {
        &self.layers[self.active].layer
    }

    /// Mutable access to the layer tools paint into (same invariant as
    /// [`Document::active_layer`]).
    pub fn active_layer_mut(&mut self) -> &mut Layer {
        &mut self.layers[self.active].layer
    }

    /// Select the active layer. Returns false and keeps the old layer when
    /// `idx` is out of range.
    pub fn set_active(&mut self, idx: usize) -> bool {
        if idx < self.layers.len() {
            self.active = idx;
            true
        } else {
            false
        }
    }

    /// Committed top-wins cell: topmost visible non-transparent layer cell.
    fn committed(&self, x: i32, y: i32) -> Option<Cell> {
        for nl in self.layers.iter().rev() {
            if !nl.visible {
                continue;
            }
            if let Some(c) = nl.layer.get(x, y) {
                return Some(c);
            }
        }
        None
    }

    /// Topmost visible non-transparent cell at `(x, y)`, with `scratch`
    /// consulted above the top layer (port of TS `LayerView.get` over
    /// `[committed, scratch]`).
    pub fn compose(&self, scratch: &Layer, x: i32, y: i32) -> Option<Cell> {
        if let Some(c) = scratch.get(x, y) {
            return Some(c);
        }
        self.committed(x, y)
    }

    /// Committed top-wins cell without scratch (what exporters and the
    /// eyedropper read when no gesture is in flight).
    pub fn cell(&self, x: i32, y: i32) -> Option<Cell> {
        self.committed(x, y)
    }

    /// Full-row render helper for exporters/preview: `compose` for
    /// `x in 0..grid_w`.
    pub fn compose_row(&self, scratch: &Layer, y: i32) -> Vec<Option<Cell>> {
        let w = self.grid.0.min(i32::MAX as u32) as i32;
        (0..w).map(|x| self.compose(scratch, x, y)).collect()
    }

    /// True when `(x, y)` is the continuation guard of a wide char anchored
    /// at `(x-1, y)` in the committed view. The renderer skips guards, the
    /// cursor jumps over them, and export emits nothing for them.
    pub fn is_continuation(&self, x: i32, y: i32) -> bool {
        matches!(self.committed(x - 1, y), Some(c) if c.width() >= 2)
    }

    /// Move the cursor one cell horizontally, skipping continuation guards:
    /// `dir > 0` steps right (landing on a guard jumps one further),
    /// `dir < 0` steps left (landing on a guard jumps back to its anchor),
    /// `dir == 0` stays. The result is clamped to `>= 0`; the TUI clamps
    /// the upper end to its viewport.
    pub fn cursor_step_x(&self, x: i32, y: i32, dir: i32) -> i32 {
        let mut nx = if dir > 0 {
            x + 1
        } else if dir < 0 {
            x - 1
        } else {
            x
        };
        if dir != 0 && self.is_continuation(nx, y) {
            nx += dir.signum();
        }
        nx.max(0)
    }
}

// ---------------------------------------------------------------------------
// History (diff-stack undo)
// ---------------------------------------------------------------------------

/// One undo entry: the inverse patch for a single layer plus the selection
/// captured before the gesture (port of the parallel `_undoLayers` /
/// `_undoSelections` stacks in `canvas.ts`, kept in one struct so the two
/// can never desync).
#[derive(Clone, Debug)]
pub struct HistoryEntry {
    /// Which layer the diff applies to (per-layer aware).
    pub layer_idx: usize,
    /// Inverse patch: `apply` it to undo.
    pub diff: Layer,
    /// Selection captured before the gesture (restored on undo).
    pub selection: Option<Rect>,
}

/// Diff-stack undo/redo plus the live selection and the pending
/// gesture-start selection (port of `CanvasStore` undo/redo/selection).
#[derive(Clone, Debug, Default)]
pub struct History {
    undo: Vec<HistoryEntry>,
    redo: Vec<HistoryEntry>,
    pending_selection: Option<Rect>,
    selection: Option<Rect>,
}

impl History {
    /// Empty history with no selection.
    pub fn new() -> Self {
        Self::default()
    }

    /// Commit `patch` to the document's active layer, capturing the pending
    /// gesture-start selection. Returns true when a non-empty diff produced
    /// an undo entry. Always clears the redo stack, even for no-op commits
    /// (TS `commitScratch` semantics).
    pub fn commit(&mut self, doc: &mut Document, patch: &Layer) -> bool {
        let layer_idx = doc.active;
        let selection = self.pending_selection.take();
        self.commit_layer(doc, layer_idx, patch, selection)
    }

    /// Commit `patch` to one layer with an explicit selection snapshot.
    /// Cross-layer gestures commit one entry per affected layer, ordered
    /// bottom-up (model-api §5). Returns true when an entry was pushed;
    /// false (and no state touched) when `layer_idx` is out of range.
    pub fn commit_layer(
        &mut self,
        doc: &mut Document,
        layer_idx: usize,
        patch: &Layer,
        selection: Option<Rect>,
    ) -> bool {
        if layer_idx >= doc.layers.len() {
            return false;
        }
        let pushed = match doc.layers[layer_idx].layer.apply(patch) {
            Some(diff) => {
                self.undo.push(HistoryEntry {
                    layer_idx,
                    diff,
                    selection,
                });
                if self.undo.len() > MAX_UNDO {
                    self.undo.drain(..self.undo.len() - MAX_UNDO);
                }
                true
            }
            None => false,
        };
        self.redo.clear();
        pushed
    }

    /// Undo the newest entry. Returns false when the stack is empty (or the
    /// entry's layer is gone, in which case the entry is kept).
    pub fn undo(&mut self, doc: &mut Document) -> bool {
        let Some(entry) = self.undo.pop() else {
            return false;
        };
        if entry.layer_idx >= doc.layers.len() {
            self.undo.push(entry);
            return false;
        }
        let redo_diff = doc.layers[entry.layer_idx]
            .layer
            .apply(&entry.diff)
            .unwrap_or_default();
        self.redo.push(HistoryEntry {
            layer_idx: entry.layer_idx,
            diff: redo_diff,
            selection: self.selection,
        });
        self.selection = entry.selection;
        true
    }

    /// Redo the newest undone entry. Returns false when the stack is empty
    /// (or the entry's layer is gone, in which case the entry is kept).
    pub fn redo(&mut self, doc: &mut Document) -> bool {
        let Some(entry) = self.redo.pop() else {
            return false;
        };
        if entry.layer_idx >= doc.layers.len() {
            self.redo.push(entry);
            return false;
        }
        let undo_diff = doc.layers[entry.layer_idx]
            .layer
            .apply(&entry.diff)
            .unwrap_or_default();
        self.undo.push(HistoryEntry {
            layer_idx: entry.layer_idx,
            diff: undo_diff,
            selection: self.selection,
        });
        if self.undo.len() > MAX_UNDO {
            self.undo.drain(..self.undo.len() - MAX_UNDO);
        }
        self.selection = entry.selection;
        true
    }

    /// Capture the live selection as a gesture begins (the TUI calls this
    /// when scratch goes empty → non-empty, `canvas.ts:191-199`). The next
    /// [`History::commit`] stores it in the new entry.
    pub fn set_pending_selection(&mut self, sel: Option<Rect>) {
        self.pending_selection = sel;
    }

    /// Live selection (what the select tool highlights).
    pub fn selection(&self) -> Option<Rect> {
        self.selection
    }

    /// Set the live selection.
    pub fn set_selection(&mut self, sel: Option<Rect>) {
        self.selection = sel;
    }

    /// Clear the live selection (tool switch, `select.ts:401-413`).
    pub fn clear_selection(&mut self) {
        self.selection = None;
    }

    /// Number of undo entries.
    pub fn undo_len(&self) -> usize {
        self.undo.len()
    }

    /// Number of redo entries.
    pub fn redo_len(&self) -> usize {
        self.redo.len()
    }

    /// True when [`History::undo`] would do something.
    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    /// True when [`History::redo`] would do something.
    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }
}

// ---------------------------------------------------------------------------
// File format (.omaframe.json read/write)
// ---------------------------------------------------------------------------

/// Wire cell: schema `{x, y, ch, fg, bg}`. Colors stay `i32` through parse
/// so out-of-range values become [`ModelError::BadColor`] (never a bare
/// JSON type error, never a silent clamp).
#[derive(Debug, Serialize, Deserialize)]
struct FileCell {
    x: i32,
    y: i32,
    ch: String,
    fg: i32,
    bg: i32,
}

#[derive(Debug, Serialize, Deserialize)]
struct FileLayer {
    name: String,
    visible: bool,
    cells: Vec<FileCell>,
}

#[derive(Debug, Serialize, Deserialize)]
struct FileWidget {
    kind: String,
    x: i32,
    y: i32,
    w: u32,
    h: u32,
    #[serde(default)]
    label: String,
    #[serde(default)]
    style: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct FileGrid {
    w: u32,
    h: u32,
}

fn default_preview_dark() -> bool {
    true
}

fn default_palette_tab() -> String {
    "outlines".to_string()
}

#[derive(Debug, Serialize, Deserialize)]
struct FilePreview {
    #[serde(default = "default_preview_dark")]
    dark: bool,
}

impl Default for FilePreview {
    fn default() -> Self {
        Self { dark: true }
    }
}

/// Shadow of the on-disk schema: field order matches `schema.json`
/// (`version, name, grid, layers, widgets, preview, paletteTab`) so saves
/// are deterministic. Unknown keys are ignored on load (forward-compat).
#[derive(Debug, Serialize, Deserialize)]
struct FileDoc {
    version: u32,
    name: String,
    grid: FileGrid,
    layers: Vec<FileLayer>,
    #[serde(default)]
    widgets: Vec<FileWidget>,
    #[serde(default)]
    preview: FilePreview,
    #[serde(default = "default_palette_tab", rename = "paletteTab")]
    palette_tab: String,
}

fn check_cell_shape(ch: &str) -> Result<(), ModelError> {
    let nchars = ch.chars().count();
    if nchars > 2 || (nchars == 2 && width_of(ch) > 1) {
        return Err(ModelError::BadChar(ch.to_string()));
    }
    Ok(())
}

fn file_cell_to_validated(fc: &FileCell) -> Result<Option<Cell>, ModelError> {
    if !(0..=15).contains(&fc.fg) || !(-1..=15).contains(&fc.bg) {
        return Err(ModelError::BadColor {
            fg: fc.fg,
            bg: fc.bg,
        });
    }
    check_cell_shape(&fc.ch)?;
    // Canonicalize: transparent and zero-width entries compose identically
    // to absent, so the sparse layer simply omits them.
    if is_transparent_ch(&fc.ch) || width_of(&fc.ch) == 0 {
        return Ok(None);
    }
    Ok(Some(Cell {
        ch: fc.ch.clone(),
        fg: fc.fg as i8,
        bg: fc.bg as i8,
    }))
}

fn file_doc_to_document(fd: FileDoc) -> Result<Document, ModelError> {
    if fd.version != FILE_VERSION {
        return Err(ModelError::BadVersion(fd.version));
    }
    if fd.grid.w == 0 || fd.grid.h == 0 {
        return Err(ModelError::BadGrid(format!(
            "grid {}x{} (want >= 1x1)",
            fd.grid.w, fd.grid.h
        )));
    }
    if fd.layers.is_empty() {
        return Err(ModelError::BadGrid("at least one layer is required".to_string()));
    }
    let mut layers = Vec::with_capacity(fd.layers.len());
    for fl in &fd.layers {
        // File order is significant: duplicate (x, y) = last wins, and
        // overlapping wide anchors resolve the same way.
        let mut layer = Layer::new();
        for fc in &fl.cells {
            if let Some(cell) = file_cell_to_validated(fc)? {
                layer.paint_one(fc.x, fc.y, &cell);
            }
        }
        layers.push(NamedLayer {
            name: fl.name.clone(),
            visible: fl.visible,
            layer,
        });
    }
    let mut widgets = Vec::with_capacity(fd.widgets.len());
    for fw in &fd.widgets {
        if fw.w == 0 || fw.h == 0 {
            return Err(ModelError::BadGeometry(format!(
                "widget {:?} is {}x{} (want >= 1x1)",
                fw.kind, fw.w, fw.h
            )));
        }
        widgets.push(Widget {
            kind: fw.kind.clone(),
            rect: Rect::new(fw.x, fw.y, fw.w, fw.h),
            label: fw.label.clone(),
            style: fw.style.clone(),
        });
    }
    Ok(Document {
        name: fd.name,
        grid: (fd.grid.w, fd.grid.h),
        layers,
        active: 0,
        widgets,
        preview_dark: fd.preview.dark,
        palette_tab: fd.palette_tab,
    })
}

fn document_to_file_doc(doc: &Document) -> Result<FileDoc, ModelError> {
    if doc.layers.is_empty() {
        return Err(ModelError::BadGrid("at least one layer is required".to_string()));
    }
    if doc.grid.0 == 0 || doc.grid.1 == 0 {
        return Err(ModelError::BadGrid(format!(
            "grid {}x{} (want >= 1x1)",
            doc.grid.0, doc.grid.1
        )));
    }
    let mut layers = Vec::with_capacity(doc.layers.len());
    for nl in &doc.layers {
        let mut cells: Vec<((i32, i32), &Cell)> = nl
            .layer
            .cells
            .iter()
            .filter(|(_, c)| !c.is_transparent() && c.width() != 0)
            .map(|(k, c)| (*k, c))
            .collect();
        cells.sort_unstable_by_key(|((x, y), _)| (*y, *x));
        let mut out = Vec::with_capacity(cells.len());
        for ((x, y), c) in cells {
            c.validate()?;
            out.push(FileCell {
                x,
                y,
                ch: c.ch.clone(),
                fg: c.fg as i32,
                bg: c.bg as i32,
            });
        }
        layers.push(FileLayer {
            name: nl.name.clone(),
            visible: nl.visible,
            cells: out,
        });
    }
    let widgets = doc
        .widgets
        .iter()
        .map(|w| FileWidget {
            kind: w.kind.clone(),
            x: w.rect.x,
            y: w.rect.y,
            w: w.rect.w,
            h: w.rect.h,
            label: w.label.clone(),
            style: w.style.clone(),
        })
        .collect();
    Ok(FileDoc {
        version: FILE_VERSION,
        name: doc.name.clone(),
        grid: FileGrid {
            w: doc.grid.0,
            h: doc.grid.1,
        },
        layers,
        widgets,
        preview: FilePreview {
            dark: doc.preview_dark,
        },
        palette_tab: doc.palette_tab.clone(),
    })
}

/// Parse `.omaframe.json` text into a [`Document`] (schema:
/// `skills/omaframe/schema.json`). See the module docs for tolerance
/// (ignored unknown keys, soft grid cap, preserved out-of-grid coords)
/// versus loud errors (version, colors, shapes).
pub fn load_json(s: &str) -> Result<Document, ModelError> {
    let fd: FileDoc = serde_json::from_str(s)?;
    file_doc_to_document(fd)
}

/// Serialize a [`Document`] to canonical pretty `.omaframe.json` text
/// (deterministic: schema field order, cells sorted by `(y, x)`).
pub fn save_json(doc: &Document) -> Result<String, ModelError> {
    let fd = document_to_file_doc(doc)?;
    Ok(serde_json::to_string_pretty(&fd)?)
}

/// Load a `.omaframe.json` file from disk.
pub fn load_file(path: impl AsRef<Path>) -> Result<Document, ModelError> {
    load_json(&std::fs::read_to_string(path)?)
}

/// Save a [`Document`] to disk as `.omaframe.json`.
pub fn save_file(doc: &Document, path: impl AsRef<Path>) -> Result<(), ModelError> {
    std::fs::write(path, save_json(doc)?)?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Export (character view of compose)
// ---------------------------------------------------------------------------

fn export_rows(doc: &Document, x0: i32, y0: i32, w: u32, h: u32) -> Vec<String> {
    // Unbounded: the canvas is infinite, `compose`/`cell` return `None`
    // outside content (rendered/exported as spaces). i64 math keeps extreme
    // coords panic-free.
    let x_end = (x0 as i64 + w as i64).min(i32::MAX as i64);
    let y_end = (y0 as i64 + h as i64).min(i32::MAX as i64);
    let mut lines = Vec::new();
    for y in y0..y_end as i32 {
        let mut line = String::new();
        for x in x0..x_end as i32 {
            // Continuation guards emit nothing: the wide anchor already
            // occupies both terminal columns.
            if doc.is_continuation(x, y) {
                continue;
            }
            match doc.cell(x, y) {
                Some(c) => line.push_str(&c.ch),
                None => line.push(' '),
            }
        }
        // Export trims trailing transparent cells per row, so space-filled
        // and erased tails export identically (asciiflow issue #211).
        line = line.trim_end_matches(' ').to_string();
        lines.push(line);
    }
    // Skip fully-empty trailing rows.
    while lines.last().is_some_and(|l| l.is_empty()) {
        lines.pop();
    }
    lines
}

fn join_lines(lines: Vec<String>) -> String {
    if lines.is_empty() {
        String::new()
    } else {
        lines.join("\n") + "\n"
    }
}

/// Bounding box of non-transparent cells in visible layers, if any.
///
/// The canvas is infinite; [`Document::grid`] is only the new-file hint and
/// the minimum export frame — never a boundary. Export expands the frame to
/// fit content but never shrinks it, so files whose content sits inside the
/// grid render byte-identically to before.
pub fn content_bounds(doc: &Document) -> Option<Rect> {
    let mut min_x = i32::MAX;
    let mut min_y = i32::MAX;
    let mut max_x = i32::MIN;
    let mut max_y = i32::MIN;
    for nl in &doc.layers {
        if !nl.visible {
            continue;
        }
        for ((x, y), c) in nl.layer.entries() {
            if c.is_transparent() {
                continue;
            }
            min_x = min_x.min(x);
            min_y = min_y.min(y);
            max_x = max_x.max(x);
            max_y = max_y.max(y);
        }
    }
    if min_x == i32::MAX {
        return None;
    }
    // i64 math keeps pathological coords panic-free; Rect is u32-sized.
    let w = (max_x as i64 - min_x as i64 + 1).clamp(1, u32::MAX as i64) as u32;
    let h = (max_y as i64 - min_y as i64 + 1).clamp(1, u32::MAX as i64) as u32;
    Some(Rect::new(min_x, min_y, w, h))
}

/// Export frame: grid minimum expanded to fit content (infinite canvas).
/// `(x0, y0, w, h)` with `w/h >= 1` whenever the grid is non-empty.
fn export_frame(doc: &Document) -> (i32, i32, u32, u32) {
    let gw = doc.grid.0.min(i32::MAX as u32) as i64;
    let gh = doc.grid.1.min(i32::MAX as u32) as i64;
    let (mut x0, mut y0) = (0i64, 0i64);
    let (mut x1, mut y1) = (gw, gh);
    if let Some(b) = content_bounds(doc) {
        x0 = x0.min(b.x as i64);
        y0 = y0.min(b.y as i64);
        x1 = x1.max(b.x as i64 + b.w as i64);
        y1 = y1.max(b.y as i64 + b.h as i64);
    }
    let w = (x1 - x0).clamp(0, u32::MAX as i64) as u32;
    let h = (y1 - y0).clamp(0, u32::MAX as i64) as u32;
    (
        x0.clamp(i32::MIN as i64, i32::MAX as i64) as i32,
        y0.clamp(i32::MIN as i64, i32::MAX as i64) as i32,
        w,
        h,
    )
}

/// Plain-text export (`.txt`): characters only, ANSI stripped. Per row:
/// `compose` chars (`None` → `' '`), trailing spaces trimmed, fully-empty
/// trailing rows skipped. Pure of all TUI state, so snapshot tests never
/// boot Ratatui.
pub fn export_txt(doc: &Document) -> String {
    let (x0, y0, w, h) = export_frame(doc);
    join_lines(export_rows(doc, x0, y0, w, h))
}

/// Plain-text export of a selection rect (unbounded: the infinite canvas has
/// no grid to clip against; same row rules as [`export_txt`]). Used by
/// copy-selection and the CLI `--selection x,y,w,h` flag.
pub fn export_selection(doc: &Document, rect: &Rect) -> String {
    join_lines(export_rows(doc, rect.x, rect.y, rect.w, rect.h))
}

/// Plain-text export of the full grid (alias of [`export_txt`)).
pub fn render_to_text(doc: &Document) -> String {
    export_txt(doc)
}

fn fg_sgr(fg: i8) -> i32 {
    if fg < 8 {
        30 + fg as i32
    } else {
        90 + (fg as i32 - 8)
    }
}

fn bg_sgr(bg: i8) -> Option<i32> {
    if bg < 0 {
        None
    } else if bg < 8 {
        Some(40 + bg as i32)
    } else {
        Some(100 + (bg as i32 - 8))
    }
}

/// `.md` export: `# name` title plus a fenced `text` block.
pub fn export_md(doc: &Document) -> String {
    format!("# {}\n```text\n{}```\n", doc.name, export_txt(doc))
}

/// `.txt+ansi` export: like [`export_txt`] but non-transparent cells carry
/// SGR color escapes from their fg/bg slots (`30–37`/`90–97`,
/// `40–47`/`100–107`; `bg = -1` emits no background code). Lines are reset
/// with `\x1b[0m` after their last colored cell.
pub fn export_ansi(doc: &Document) -> String {
    let (fx, fy, fw, fh) = export_frame(doc);
    let x_end = (fx as i64 + fw as i64).min(i32::MAX as i64);
    let y_end = (fy as i64 + fh as i64).min(i32::MAX as i64);
    let mut lines = Vec::new();
    for y in fy..y_end as i32 {
        // (text, color) segments so trailing spaces can be trimmed before
        // any escape is emitted.
        let mut segs: Vec<(String, Option<(i8, i8)>)> = Vec::new();
        for x in fx..x_end as i32 {
            if doc.is_continuation(x, y) {
                continue;
            }
            match doc.cell(x, y) {
                Some(c) => segs.push((c.ch.clone(), Some((c.fg, c.bg)))),
                None => segs.push((" ".to_string(), None)),
            }
        }
        while segs.last().is_some_and(|(s, col)| col.is_none() && s == " ") {
            segs.pop();
        }
        while segs.last().is_some_and(|(s, _)| s.is_empty()) {
            segs.pop();
        }
        if segs.is_empty() {
            lines.push(String::new());
            continue;
        }
        let mut line = String::new();
        let mut cur: Option<(i8, i8)> = None;
        for (s, col) in &segs {
            if *col != cur {
                if col.is_none() {
                    line.push_str("\x1b[0m");
                } else {
                    let (fg, bg) = col.unwrap();
                    match bg_sgr(bg) {
                        Some(bg_code) => {
                            line.push_str(&format!("\x1b[{};{}m", fg_sgr(fg), bg_code));
                        }
                        None => {
                            line.push_str(&format!("\x1b[{}m", fg_sgr(fg)));
                        }
                    }
                }
                cur = *col;
            }
            line.push_str(s);
        }
        if cur.is_some() {
            line.push_str("\x1b[0m");
        }
        lines.push(line);
    }
    while lines.last().is_some_and(|l| l.is_empty()) {
        lines.pop();
    }
    join_lines(lines)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn cell(ch: &str, fg: i8, bg: i8) -> Cell {
        Cell::new(ch, fg, bg)
    }

    fn paint(doc: &mut Document, layer_idx: usize, x: i32, y: i32, ch: &str, fg: i8) -> bool {
        let mut patch = Layer::new();
        patch.set(x, y, cell(ch, fg, -1));
        let mut hist = History::new();
        hist.commit_layer(doc, layer_idx, &patch, None)
    }

    #[test]
    fn compose_top_wins() {
        let mut doc = Document::new("t", 10, 5);
        paint(&mut doc, 0, 3, 1, "a", 1);
        paint(&mut doc, 1, 3, 1, "b", 2);
        assert_eq!(doc.cell(3, 1), Some(cell("b", 2, -1)));
        // Lower layer still holds its own value underneath.
        assert_eq!(doc.layers[0].layer.get(3, 1), Some(cell("a", 1, -1)));
        // Invisible top layer shows through.
        doc.layers[1].visible = false;
        assert_eq!(doc.cell(3, 1), Some(cell("a", 1, -1)));
        // Scratch beats everything.
        let mut scratch = Layer::new();
        scratch.set(3, 1, cell("s", 3, -1));
        assert_eq!(doc.compose(&scratch, 3, 1), Some(cell("s", 3, -1)));
        // Empty spot composes to None.
        assert_eq!(doc.cell(0, 0), None);
    }

    #[test]
    fn transparency_space_and_null_show_through() {
        let mut doc = Document::new("t", 10, 5);
        paint(&mut doc, 0, 2, 2, "a", 1);
        // A space marker stored on the top layer is transparent: the bottom
        // layer shows through (markers can reach committed layers via
        // direct `set`, e.g. tool scratch folded by hand).
        doc.layers[1].layer.set(2, 2, cell(" ", 5, -1));
        assert_eq!(doc.cell(2, 2), Some(cell("a", 1, -1)));
        doc.layers[1].layer.set(2, 2, Cell::erased());
        assert_eq!(doc.cell(2, 2), Some(cell("a", 1, -1)));
        // Committing a space over the top layer's own content IS a change
        // for that layer (its cell goes away), even though the composed
        // view still shows the bottom layer.
        paint(&mut doc, 1, 2, 2, "b", 5);
        assert_eq!(doc.cell(2, 2), Some(cell("b", 5, -1)));
        let mut patch = Layer::new();
        patch.set(2, 2, cell(" ", 5, -1));
        let mut hist = History::new();
        assert!(hist.commit_layer(&mut doc, 1, &patch, None));
        assert_eq!(doc.cell(2, 2), Some(cell("a", 1, -1)));
        // Committing transparency over an already-empty spot of that layer
        // is a no-op: nothing observable changed, no undo entry.
        let mut patch2 = Layer::new();
        patch2.set(2, 2, Cell::erased());
        assert!(!hist.commit_layer(&mut doc, 1, &patch2, None));
        // Erasing the bottom layer as well leaves nothing.
        let mut patch3 = Layer::new();
        patch3.set(2, 2, Cell::erased());
        assert!(hist.commit_layer(&mut doc, 0, &patch3, None));
        assert_eq!(doc.cell(2, 2), None);
        // Cell-level transparency spellings.
        assert!(cell("", 0, -1).is_transparent());
        assert!(cell(" ", 0, -1).is_transparent());
        assert!(cell("\0", 0, -1).is_transparent());
        assert!(!cell("a", 0, -1).is_transparent());
    }

    #[test]
    fn undo_redo_round_trip() {
        let mut doc = Document::new("t", 10, 5);
        let mut hist = History::new();
        assert!(!hist.can_undo());
        assert!(!hist.can_redo());
        let mut patch = Layer::new();
        patch.set(1, 1, cell("x", 4, -1));
        assert!(hist.commit(&mut doc, &patch));
        assert_eq!(doc.cell(1, 1), Some(cell("x", 4, -1)));
        assert!(hist.can_undo());
        assert!(hist.undo(&mut doc));
        assert_eq!(doc.cell(1, 1), None);
        assert!(!hist.can_undo());
        assert!(hist.can_redo());
        assert!(hist.redo(&mut doc));
        assert_eq!(doc.cell(1, 1), Some(cell("x", 4, -1)));
        // Undo on an empty stack is a no-op returning false.
        let mut fresh = History::new();
        assert!(!fresh.undo(&mut doc));
        assert!(!fresh.redo(&mut doc));
    }

    #[test]
    fn empty_diff_is_a_no_op_but_clears_redo() {
        let mut doc = Document::new("t", 10, 5);
        let mut hist = History::new();
        let mut patch = Layer::new();
        patch.set(1, 1, cell("x", 4, -1));
        assert!(hist.commit(&mut doc, &patch));
        assert!(hist.undo(&mut doc));
        assert!(hist.can_redo());
        // Re-committing the identical cell is a no-op: no new entry ...
        let mut same = Layer::new();
        same.set(1, 1, cell("x", 4, -1));
        // (cell is currently absent after the undo, so this paints again)
        assert!(hist.commit(&mut doc, &same));
        // ... and an actually-identical repaint pushes nothing ...
        let mut same2 = Layer::new();
        same2.set(1, 1, cell("x", 4, -1));
        assert!(!hist.commit(&mut doc, &same2));
        // ... nor does erasing empty space ...
        let mut erase_empty = Layer::new();
        erase_empty.set(7, 7, Cell::erased());
        assert!(!hist.commit(&mut doc, &erase_empty));
        // ... nor does an empty patch.
        assert!(!hist.commit(&mut doc, &Layer::new()));
        assert_eq!(hist.undo_len(), 1);
        // No-op commits still clear redo (TS commitScratch semantics).
        let mut doc2 = Document::new("t", 10, 5);
        let mut h2 = History::new();
        let mut p = Layer::new();
        p.set(0, 0, cell("a", 1, -1));
        assert!(h2.commit(&mut doc2, &p));
        assert!(h2.undo(&mut doc2));
        assert!(h2.can_redo());
        assert!(!h2.commit(&mut doc2, &Layer::new()));
        assert!(!h2.can_redo());
    }

    #[test]
    fn new_commit_clears_redo() {
        let mut doc = Document::new("t", 10, 5);
        let mut hist = History::new();
        let mut p1 = Layer::new();
        p1.set(0, 0, cell("a", 1, -1));
        assert!(hist.commit(&mut doc, &p1));
        assert!(hist.undo(&mut doc));
        let mut p2 = Layer::new();
        p2.set(1, 0, cell("b", 2, -1));
        assert!(hist.commit(&mut doc, &p2));
        assert!(!hist.can_redo());
        assert!(!hist.redo(&mut doc));
    }

    #[test]
    fn undo_is_capped_at_fifty() {
        let mut doc = Document::new("t", 100, 5);
        let mut hist = History::new();
        for i in 0..55 {
            let mut p = Layer::new();
            p.set(i, 0, cell("x", 1, -1));
            assert!(hist.commit(&mut doc, &p));
        }
        assert_eq!(hist.undo_len(), MAX_UNDO);
        assert_eq!(MAX_UNDO, 50);
        for _ in 0..50 {
            assert!(hist.undo(&mut doc));
        }
        assert!(!hist.can_undo());
        // The five oldest commits were dropped, so their cells survive.
        for x in 0..5 {
            assert_eq!(doc.cell(x, 0), Some(cell("x", 1, -1)));
        }
        for x in 5..55 {
            assert_eq!(doc.cell(x, 0), None);
        }
    }

    #[test]
    fn selection_rides_along_undo_redo() {
        let mut doc = Document::new("t", 10, 10);
        let mut hist = History::new();
        let before = Rect::new(0, 0, 3, 3);
        let after = Rect::new(5, 5, 2, 2);
        let mut p = Layer::new();
        p.set(1, 1, cell("x", 1, -1));
        assert!(hist.commit_layer(&mut doc, 0, &p, Some(before)));
        hist.set_selection(Some(after));
        assert!(hist.undo(&mut doc));
        assert_eq!(hist.selection(), Some(before));
        assert!(hist.redo(&mut doc));
        assert_eq!(hist.selection(), Some(after));
        // commit() consumes the pending gesture-start selection.
        hist.set_pending_selection(Some(before));
        let mut p2 = Layer::new();
        p2.set(2, 2, cell("y", 1, -1));
        assert!(hist.commit(&mut doc, &p2));
        assert!(hist.undo(&mut doc));
        assert_eq!(hist.selection(), Some(before));
    }

    #[test]
    fn json_round_trip_preserves_everything() {
        let mut doc = Document::new("roundtrip", 20, 8);
        paint(&mut doc, 0, 2, 1, "─", 4);
        paint(&mut doc, 1, 4, 3, "O", 7);
        paint(&mut doc, 1, 5, 3, "K", 7);
        // Multi-char ch must fail validation (and therefore saving).
        doc.layers[0].layer.set(0, 0, cell("ab", 1, -1));
        assert!(save_json(&doc).is_err());
        doc.layers[0].layer.cells.remove(&(0, 0));
        doc.widgets.push(Widget {
            kind: "button".to_string(),
            rect: Rect::new(5, 2, 6, 1),
            label: "OK".to_string(),
            style: "default".to_string(),
        });
        doc.preview_dark = false;
        doc.palette_tab = "blocks".to_string();
        let saved = save_json(&doc).expect("save");
        let back = load_json(&saved).expect("load");
        assert_eq!(doc, back);
        assert_eq!(export_txt(&doc), export_txt(&back));
        // Saving twice is stable.
        assert_eq!(saved, save_json(&back).expect("resave"));
    }

    #[test]
    fn json_rejects_bad_values_loudly() {
        // Bad version.
        let err = load_json(r#"{"version":2,"name":"x","grid":{"w":5,"h":5},"layers":[]}"#)
            .expect_err("version 2 must fail");
        assert!(matches!(err, ModelError::BadVersion(2)), "got {err:?}");
        // Bad colors.
        let bad_fg = r#"{"version":1,"name":"x","grid":{"w":5,"h":5},"layers":[{"name":"l","visible":true,"cells":[{"x":0,"y":0,"ch":"a","fg":16,"bg":-1}]}]}"#;
        assert!(matches!(
            load_json(bad_fg).expect_err("fg 16 must fail"),
            ModelError::BadColor { fg: 16, .. }
        ));
        let bad_bg = r#"{"version":1,"name":"x","grid":{"w":5,"h":5},"layers":[{"name":"l","visible":true,"cells":[{"x":0,"y":0,"ch":"a","fg":1,"bg":-2}]}]}"#;
        assert!(matches!(
            load_json(bad_bg).expect_err("bg -2 must fail"),
            ModelError::BadColor { bg: -2, .. }
        ));
        // Multi-char ch.
        let bad_ch = r#"{"version":1,"name":"x","grid":{"w":5,"h":5},"layers":[{"name":"l","visible":true,"cells":[{"x":0,"y":0,"ch":"ab","fg":1,"bg":-1}]}]}"#;
        assert!(matches!(
            load_json(bad_ch).expect_err("multi-char must fail"),
            ModelError::BadChar(_)
        ));
        // Empty layers.
        let no_layers = r#"{"version":1,"name":"x","grid":{"w":5,"h":5},"layers":[]}"#;
        assert!(matches!(
            load_json(no_layers).expect_err("no layers must fail"),
            ModelError::BadGrid(_)
        ));
        // Unknown top-level keys are ignored (forward-compat).
        let extra = r#"{"version":1,"name":"x","grid":{"w":5,"h":5},"layers":[{"name":"l","visible":true,"cells":[]}],"futureThing":{"a":1}}"#;
        assert!(load_json(extra).is_ok());
        // Space entries load fine and compose transparent.
        let spacey = r#"{"version":1,"name":"x","grid":{"w":5,"h":5},"layers":[{"name":"l","visible":true,"cells":[{"x":1,"y":1,"ch":" ","fg":1,"bg":-1},{"x":2,"y":1,"ch":"b","fg":1,"bg":-1}]}]}"#;
        let doc = load_json(spacey).expect("space entries load");
        assert_eq!(doc.cell(1, 1), None);
        assert_eq!(doc.cell(2, 1), Some(cell("b", 1, -1)));
    }

    #[test]
    fn wide_char_width_and_guard_rules() {
        assert_eq!(width_of("漢"), 2);
        assert_eq!(width_of("─"), 1);
        assert_eq!(width_of("A"), 1);
        assert_eq!(width_of(""), 0);
        assert!(is_wide("漢"));
        assert!(!is_wide("─"));
        assert_eq!(cell("漢", 1, -1).width(), 2);

        // Paint wide, then paint on its continuation: anchor cleared.
        let mut doc = Document::new("t", 10, 5);
        let mut hist = History::new();
        let mut p = Layer::new();
        p.set(3, 1, cell("漢", 1, -1));
        assert!(hist.commit(&mut doc, &p));
        assert!(doc.is_continuation(4, 1));
        assert!(!doc.is_continuation(3, 1));
        assert_eq!(doc.cell(3, 1), Some(cell("漢", 1, -1)));
        let mut p2 = Layer::new();
        p2.set(4, 1, cell("x", 2, -1));
        assert!(hist.commit(&mut doc, &p2));
        assert_eq!(doc.cell(4, 1), Some(cell("x", 2, -1)));
        assert_eq!(doc.cell(3, 1), None);
        assert!(!doc.is_continuation(4, 1));
        // Undo restores the wide anchor exactly.
        assert!(hist.undo(&mut doc));
        assert_eq!(doc.cell(3, 1), Some(cell("漢", 1, -1)));
        assert!(doc.is_continuation(4, 1));

        // Cursor jumps over guards in both directions.
        assert_eq!(doc.cursor_step_x(2, 1, 1), 3); // onto anchor: stop
        assert_eq!(doc.cursor_step_x(3, 1, 1), 5); // onto guard: skip
        assert_eq!(doc.cursor_step_x(5, 1, -1), 3); // onto guard: back to anchor
        assert_eq!(doc.cursor_step_x(3, 1, -1), 2);
        assert_eq!(doc.cursor_step_x(0, 1, -1), 0); // clamped at 0
        assert_eq!(doc.cursor_step_x(2, 1, 0), 2);

        // Export skips the guard (no padding space).
        let txt = export_txt(&doc);
        assert!(txt.contains("漢"), "got {txt:?}");
        let row: String = txt.lines().nth(1).unwrap().to_string();
        assert_eq!(row, "   漢");
    }

    #[test]
    fn golden_demo_export_is_byte_identical() {
        let src = include_str!("../testdata/demo.omaframe.json");
        let expected = include_str!("../testdata/demo.txt");
        let doc = load_json(src).expect("demo loads");
        assert_eq!(doc.grid, (40, 12));
        assert_eq!(doc.layers.len(), 2);
        assert_eq!(export_txt(&doc), expected);
        assert_eq!(render_to_text(&doc), expected);
        // Save -> reload keeps the export stable.
        let back = load_json(&save_json(&doc).expect("save")).expect("reload");
        assert_eq!(export_txt(&back), expected);
    }

    #[test]
    fn selection_export_clips_and_trims() {
        let doc = load_json(include_str!("../testdata/demo.omaframe.json")).expect("demo");
        // The "Settings" title span on the top border.
        let title = export_selection(&doc, &Rect::new(6, 0, 8, 1));
        assert_eq!(title, "Settings\n");
        // A rect covering the buttons row keeps leading spaces, trims the tail.
        let buttons = export_selection(&doc, &Rect::new(0, 2, 40, 1));
        assert_eq!(buttons, "  │  [ OK ]   [ Cancel ]             │\n");
        // Fully clipped rects export empty.
        assert_eq!(export_selection(&doc, &Rect::new(100, 100, 5, 5)), "");
        assert_eq!(export_selection(&doc, &Rect::new(0, 0, 0, 5)), "");
    }

    #[test]
    fn content_bounds_and_expanded_export_frame() {
        let mut doc = Document::new("t", 80, 24);
        assert_eq!(content_bounds(&doc), None);
        // Content inside the grid: frame is the grid (golden-compatible).
        doc.active_layer_mut().set(2, 3, Cell::new("x", 7, -1));
        assert_eq!(
            content_bounds(&doc),
            Some(Rect::new(2, 3, 1, 1))
        );
        // Content beyond (and before) the grid expands the export frame.
        doc.active_layer_mut().set(100, 50, Cell::new("y", 7, -1));
        doc.active_layer_mut().set(-4, -2, Cell::new("z", 7, -1));
        let b = content_bounds(&doc).expect("bounds");
        assert_eq!((b.x, b.y), (-4, -2));
        let txt = export_txt(&doc);
        let lines: Vec<&str> = txt.lines().collect();
        assert_eq!(lines.len(), 53, "rows -2..=50");
        assert!(lines[0].starts_with("z"), "negative-origin row exports");
        assert!(lines[52].ends_with('y'), "far cell exports past grid");
        // Hidden layers don't count.
        doc.layers[0].visible = false;
        assert_eq!(content_bounds(&doc), None);
    }

    #[test]
    fn markdown_export_wraps_txt() {
        let doc = load_json(include_str!("../testdata/demo.omaframe.json")).expect("demo");
        let md = export_md(&doc);
        assert!(md.starts_with("# settings-panel\n```text\n"));
        assert!(md.ends_with("```\n"));
        assert!(md.contains("Settings"));
    }

    #[test]
    fn ansi_export_strips_back_to_txt() {
        let doc = load_json(include_str!("../testdata/demo.omaframe.json")).expect("demo");
        let ansi = export_ansi(&doc);
        assert!(ansi.contains("\x1b["), "expected SGR codes");
        // Strip SGR sequences; what remains must equal plain export.
        let mut stripped = String::new();
        let mut chars = ansi.chars();
        while let Some(c) = chars.next() {
            if c == '\x1b' {
                assert_eq!(chars.next(), Some('['));
                for c2 in chars.by_ref() {
                    if c2 == 'm' {
                        break;
                    }
                }
            } else {
                stripped.push(c);
            }
        }
        assert_eq!(stripped, export_txt(&doc));
    }

    #[test]
    fn rect_helpers() {
        let r = Rect::from_points(5, 5, 2, 2);
        assert_eq!(r, Rect::new(2, 2, 4, 4));
        assert!(r.contains(2, 2));
        assert!(r.contains(5, 5));
        assert!(!r.contains(6, 2));
        assert!(!r.contains(1, 2));
        assert!(Rect::new(0, 0, 0, 3).is_empty());
        assert!(!r.is_empty());
    }
}
