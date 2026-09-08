# omaframe TUI — Model API (proposal)

> Small, practical Rust model API for the scaffold agent to expose.
> Extends `client/layer.ts` (`Map<Vector,string>` + `apply→[new,undo]` +
> `LayerView` top-wins) from `Cell=char` to `Cell{ch,fg,bg}` with ANSI-16
> colors, per plan §3.2. **Proposal, not law** — keep the shape, rename
> freely, but preserve the semantics (transparency, diff-stack undo,
> top-wins compose, width handling) so tools-spec and the agent skill hold.

## 1. Core types (normative semantics)

```rust
/// One painted cell. `None` == erased (see §3).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Cell {
    pub ch: Option<char>, // None = transparent/erased
    pub fg: i8,           // 0..=15 ANSI slot; ignored when ch.is_none()
    pub bg: i8,           // -1 = transparent, else 0..=15
}

impl Cell {
    pub fn new(ch: char, fg: i8, bg: i8) -> Self;
    pub fn erased() -> Self;              // { None, 0, -1 }
    pub fn is_transparent(&self) -> bool; // ch.is_none() || ch == Some(' ') || ch == Some('\0')
}

/// Sparse layer: only non-transparent cells stored (like TS `Layer.map`).
#[derive(Clone, Default, Debug)]
pub struct Layer {
    cells: HashMap<(i32, i32), Cell>,
}

/// Ordered stack, index 0 = bottom.
#[derive(Clone, Debug)]
pub struct Document {
    pub grid: (u32, u32),        // (w, h); new files default 80x24; soft cap 500x200
    pub layers: Vec<NamedLayer>, // >= 1; defaults: ["wireframe", "labels"]
    pub active: usize,           // index into layers; tools paint here
    pub widgets: Vec<Widget>,    // parametric stamps (see §6)
    pub preview_dark: bool,      // ☾/☀ toggle (chrome only, not serialized into cells)
}

#[derive(Clone, Debug)]
pub struct NamedLayer {
    pub name: String,
    pub visible: bool,
    pub layer: Layer,
}
```

Validation (load + API boundary): `fg` must be `0..=15`, `bg` `-1..=15`;
`ch` must be a single `char` (never `""`, never multi-char — TS `Layer.set`
accepts strings; Rust must reject `""` at parse and map it to `erased()`).
Out-of-range colors → `Err(ModelError::BadColor)` (CLI exit non-zero), never
silent clamp — agents must see their mistakes.

## 2. Layer ops (port of `client/layer.ts:59-106`)

```rust
impl Layer {
    pub fn get(&self, x: i32, y: i32) -> Option<Cell>;   // None = transparent
    pub fn set(&mut self, x: i32, y: i32, c: Cell);      // transparent => remove key
    pub fn has(&self, x: i32, y: i32) -> bool;
    pub fn keys(&self) -> impl Iterator<Item = (i32, i32)> + '_;
    pub fn entries(&self) -> impl Iterator<Item = ((i32,i32), Cell)> + '_;
    pub fn clear(&mut self);
    pub fn len(&self) -> usize;

    /// Merge another layer's entries (scratch builders, snap buffers).
    pub fn set_from(&mut self, other: &Layer);

    /// Commit semantics: fold `patch` into self, return the inverse diff.
    /// Deletion chars (transparent cells in patch) remove keys.
    /// Undo diff records `old.unwrap_or(erased)` per touched key whose value changed.
    /// Returns None when the diff is empty (=> push NOTHING on undo stack).
    pub fn apply(&mut self, patch: &Layer) -> Option<Layer>;
}
```

`apply` is the undo atom (`canvas.ts:219-238`): `commitScratch` calls
`committed.apply(scratch)`; empty diff → no history entry, redo cleared
regardless.

## 3. Transparency (normative — plan §3.2)

A composed cell is transparent iff **any** of: key absent, `ch.is_none()`,
`ch == ' '`. (TS treats `""` and `" "` identically as deletion,
`layer.ts:96-104`, and space shows lower layers through.) Consequences:

- Eraser writes `Cell::erased()` (key removal) — plan: "eraser writes null".
- Text Backspace writes `Cell::erased()` (TS wrote `" "`; identical compose).
- Export trims trailing transparent cells per row (so space-filled tails and
  erased tails export identically — issue #211 stays fixed).

## 4. Compose — top-wins (port of `LayerView`, `layer.ts:109-136`)

```rust
impl Document {
    /// Topmost visible non-transparent cell at (x,y), scratch on top.
    pub fn compose(&self, scratch: &Layer, x: i32, y: i32) -> Option<Cell>;
    /// Full-row render helper for exporters/preview.
    pub fn compose_row(&self, scratch: &Layer, y: i32) -> Vec<Option<Cell>>;
}
```

Order: `layers[0]` bottom → `layers[last]` top → `scratch` topmost.
Invisible layers skipped. First (from top) non-transparent cell wins.
No blend modes in v1 (plan §4.5).

## 5. Undo — diff stack (port of `client/store/canvas.ts:219-278`)

```rust#[derive(Default)]
pub struct History {
    undo: Vec<HistoryEntry>, // capped at 50 (MAX_UNDO)
    redo: Vec<HistoryEntry>,
    pending_selection: Option<Rect>, // captured when scratch goes empty->non-empty
}

pub struct HistoryEntry {
    pub layer_idx: usize, // which layer the diff applies to (per-layer aware)
    pub diff: Layer,      // inverse patch: apply() to undo
    pub selection: Option<Rect>,
}

impl History {
    pub fn commit(&mut self, doc: &mut Document, patch: &Layer); // apply+push+clear redo
    pub fn undo(&mut self, doc: &mut Document) -> bool;
    pub fn redo(&mut self, doc: &mut Document) -> bool;
}
```

Rules: `commit` pushes only when `apply` returns `Some` (non-empty);
truncate to 50 (`drain(..len-50)`); any commit clears redo; selection
snapshots ride alongside (restore pre-gesture selection on undo, stash
current for redo — `canvas.ts:240-278`). Multi-layer gestures (box move
touches one layer in v1 — keep `layer_idx`; cross-layer move is a
`Ctrl-K` op that commits one entry per affected layer, ordered bottom-up).

## 6. Widgets (parametric records + baked fallback, plan §4.4)

```rust
#[derive(Clone, Debug)]
pub struct Widget {
    pub kind: WidgetKind, // Button, Input, Dropdown, Radio, Checkbox, Toggle,
                          // Close, ScrollbarV/H, Progress, Divider, Panel, Tabs
    pub rect: Rect,       // cell bbox
    pub label: String,
    pub style: WidgetStyle, // Light | Heavy | Double | Rounded | Ascii
    pub state: WidgetState, // e.g. On/Off, percent, focused
}
```

Bake rule: widget owns its `rect` — bake writes widget cells into a reserved
`widgets` contribution consulted **above** top layer but **below** scratch in
`compose` (so pencil can annotate over widgets, and select can still grab
them by hit-testing `widgets[]` first). Re-bake on move/resize/relabel.
Hand edits to baked cells are overlay, never back-propagated (one-way).

## 7. Unicode width (from day one, plan §3.2)

- Use the `unicode-width` crate: `width(ch) = 0 | 1 | 2`.
- Width-2 chars occupy `(x,y)` + continuation guard at `(x+1,y)`: renderer
  skips the next cell; cursor jumps over it; export pads correctly.
- Zero-width (combining) chars: reject at input boundary (status-bar notice)
  — canvas stores spacing chars only.
- Painting any cell whose position is currently a continuation clears the
  wide anchor (no orphans). Loading a file with overlapping wide anchors:
  last-writer-wins in file order, drop the orphaned anchor (document + test).

## 8. `.omaframe.json` schema (v1, normative — plan §3.2 + §4.4)

```jsonc
{
  "version": 1,
  "name": "settings-screen",
  "grid": { "w": 80, "h": 24 },
  "layers": [
    { "name": "wireframe", "visible": true,
      "cells": [{ "x": 2, "y": 1, "ch": "╭", "fg": 4, "bg": -1 }] },
    { "name": "labels", "visible": true, "cells": [] }
  ],
  "widgets": [
    { "kind": "button", "x": 4, "y": 5, "w": 8, "h": 1,
      "label": "OK", "style": "rounded", "state": "default" }
  ],
  "preview": { "dark": true },
  "paletteTab": "outlines"
}
```

Field rules: `version: 1` required (reject others with a clear CLI error);
`grid.w/h` integers, soft cap 500×200 (warn + accept above, render clips);
`fg: 0–15`, `bg: -1–15`; `ch`: single-char string (a `" "` entry is legal
but composes transparent — exporters may drop them on load with a warning);
`cells` order insignificant, duplicate `(x,y)` = last wins; `widgets`
optional (default `[]`); `preview`/`paletteTab` optional hints (defaults
`{dark:true}` / `"outlines"`); unknown top-level keys **ignored**
(forward-compat, same tolerant policy as config/theme parsing).

Canonical file ops: explicit save anywhere + autosave to
`~/.local/share/omaframe/autosave/`; dirty dot + `Ctrl-S` (plan §5).
CLI: `--open F --export {txt,md,ansi} --to OUT [--selection x,y,w,h]`.

## 9. Export (character view of compose)

- `.txt`: per row `y in 0..h`: collect `compose()` chars (`None` → `' '`),
  trim trailing spaces, skip fully-empty trailing rows. No ANSI.
- `.md`: fenced block + optional title (`# name` + ```` ```text ````).
- `.txt+ansi` / clipboard: same text with SGR escapes from fg/bg slots
  (`30–37/90–97`, bg `40–47/100–107`, `-1` = no code). Prefer `wl-copy`,
  fallback OSC52 (plan §5).
- Snapshot tests: `testdata/*.omaframe.json → expected/*.txt` golden files
  (plan §9) — the model crate must expose `export_txt(&Document) -> String`
  pure of all TUI state so tests don't boot Ratatui.

## 10. Minimal surface checklist (keep it small)

Types: `Cell Cell::erased Layer NamedLayer Document Rect Widget(+enums)
History HistoryEntry ModelError`. Methods: `get/set/has/keys/set_from/apply`
(Layer); `compose/compose_row/active_layer/active_layer_mut` (Document);
`commit/undo/redo` (History); `export_txt/export_md/export_ansi` (free fns);
`load_json/save_json` (free fns, schema §8); `width_of` (width helper).
Target: **~300 lines** of model code + tests. Everything else (tools, snap,
entity, palette, theme, CLI) builds on these primitives — if a tool needs a
method that isn't here, add it to the tool's module, not the model.
