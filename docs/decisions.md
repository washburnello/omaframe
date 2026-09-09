# Manager decisions (Wave 1 QA)

Recorded by manager during unattended build. These resolve open questions
from `docs/tools-spec.md` and `skills/omaframe/` so Wave 2 agents are unblocked.

## Spec questions (from docs/tools-spec.md)

- **Q1 heavy/double/rounded junction tables**: v1 ports the light-weight
  tables only (as in TS). Styled boxes (heavy/double/rounded) are draw-only:
  they render their own corners but do NOT auto-junction with neighbours.
  Full per-style tables are a post-v1 issue.
- **Q2 45° diagonals**: DEFERRED (upstream issue #43). Shift constrains lines
  to axis-only. `╱╲╳` remain available as pencil/palette chars, not routed.
- **Q3 oval outline**: slope-picked `─`/`│` as spec'd. Pencil-char ovals are
  a post-v1 enhancement.
- **Q4 right-click on canvas**: GRAB (eyedropper: copies ch+fg+bg into pencil),
  matching Playscii. Painting with the bg pot happens via left-drag (the
  pencil always paints ch+fg+bg from the pots). Right-click on a *palette
  color* sets the bg pot.
- **Q5 text Backspace**: clamps at session left edge (spec'd). Never eats
  into pre-existing drawing.
- **Q6 moveCells protect set**: protect moved cells (safer than TS). Box-move
  semantics everywhere.
- **Q7 pencil**: writes raw cells, no snap pass. Confirmed.

## Schema notes (from skills agent)

- `version: {const: 1}` stays hard for v1. Bump deliberately in v2.
- Grid 500×200 stays a *soft* cap enforced warn-only in app code; the
  schema `maximum` is tolerated but the loader must accept larger (current
  `load_json` does — keep it).
- `additionalProperties: false` stays strict for v1 (catches agent typos).
- `widgets[]` ↔ baked-cells consistency is an app/test check, not schema.
- `export_txt` skips fully-empty trailing rows (model behavior wins over
  SKILL.md §2 wording; demo unaffected). SKILL.md to be amended in Wave 2
  if the skill agent's text and model diverge anywhere else.

## Wave 6: truecolor, grouped Colors, file dialog, extended chars

- **PaintColor**: cells are `fg: PaintColor` (`Ansi(u8)` live theme slot |
  `Rgb` frozen) + `bg: Option<PaintColor>` (`None` = transparent). Files
  store ints (legacy, `-1` = transparent, still written for stability) or
  `#rrggbb`. ANSI export uses `38;2`/`48;2` for RGB. Two model rules worth
  remembering: (1) palette swatches are frozen RGB snapshots, canvas ANSI
  slots stay live; (2) theme switching mid-session needs a restart (theme
  loads once at startup — watcher is future work).
- **Grouped Colors**: Backgrounds / Foregrounds / Accent / Colors /
  Brights (theme snapshots) + Pico-8 + Picotron (constants). Headers scroll
  with rows; F/B/# markers; left=fg, right=bg.
- **macOS-style files**: New asks for a path FIRST (Save dialog), then
  every stroke/text/undo/redo/delete autosaves. Ctrl-S = explicit save.
  Inline path prompt deleted, replaced by the modal.
- **File dialog**: centered modal, Cancel / +Folder (auto-numbered) /
  Open|Save buttons, editable path row, Places sidebar (nerd glyphs,
  existing dirs), Name/Size/Type/Modified list (dirs + *.omaframe.json),
  click-select + click-again confirms, Enter/Esc/Up/Down/Backspace/Alt+Up.
- **Chars**: Symbols 32→100 (arrows, triangles, bullets, suits, checks,
  math), Outlines 27→41, Blocks 21→29; width-1 enforced by test,
  fontconfig spot-checked, zero drops.
- **Chrome**: active tool `►`, `Size:WxH` in canvas bottom border,
  palette ▲▼ paging.
- QA: 73 tests, clippy clean, golden intact, menu build reinstalled.
- Known gaps: no Colors ▲▼ affordance; orange/brown unmapped; theme
  watcher; widget stamps; command palette.

- **Corner-flanked titles** everywhere: `┌─┐Tools┌──`, menu
  `┌─┐New┌─┐Save┌─┐Load┌──`, canvas `┌─┐0,0┌──` (click rects re-derived
  and tested: New +3/3, Save +9/4, Load +16/4).
- **Layers panel** bottom-right (`┌─┐Layers┌──`), rows topmost-first with
  eye + `*` active markers (click = select, first 2 cols = eye). Bottom
  layers row removed; status bar kept (prompt home + cursor/fg-bg info).
- **New-file layers** are now `Background/Frames/Text`, painting starts on
  Frames; loads activate the top layer. `activeLayer` persisted (optional,
  backward-compatible; old files load top-active) to keep save→load
  round-trip stable. Schema + SKILL.md updated; demo file untouched as a
  legacy fixture.
- **Palette ▲▼ arrows**: last slot of first/last grid rows (prefix rows on
  Widgets tab), click pages, render/hit share one hint helper. Fixed a
  real overscroll bug found by the test (scroll clamped in row units —
  `scroll_palette` now takes cols/visible).
- **Colors swatches widened to 9 blocks** per mockup (markers kept:
  `>`/`*`/`#`).
- QA: 55 tests green (arrows render+hit, layers mapping, chrome
  snapshot), clippy clean, golden intact, menu build reinstalled.
- Deviation noted: Colors panel has no ▲▼ affordance yet (17 rows scroll
  silently); status bar kept though absent from the mockup (prompt home).

User supplied an ASCII wireframe for the app chrome; implemented as spec'd:

- **Menu bar**: New / Save / Load embedded in the center column's top
  border (clickable), `File: <full path>*` row below (`~`-shortened).
  New keeps the path slot; Save without a path opens Save As.
- **Inline path prompt** in the status bar (Load / Save As): type, Enter
  confirms (stays open on error), Esc cancels. `~` + relative expansion.
- **Colors panel** (left, under Tools): transparent row + 16 ANSI swatches
  with `>` fg / `*` bg / `#` both markers; left-click fg, right-click bg,
  wheel scrolls. Old palette fg/bg pots removed.
- **Pan tool** (`_` / Space, 11th rail entry): left-drag pans anywhere;
  middle-drag still pans. Space/Space-switch skips the Text tool (typing
  unaffected).
- **Boxed panels** with titles in borders; canvas origin (`ox,oy`) in its
  top border; palette divider spans full width (`├┤`).
- **Tab renames** (labels only, ids stable for file compat):
  Outlines→Outline, Nerds→Glyphs.
- **Theme ANSI mapping**: real Omarchy themes use named colors, so unset
  `colorN` slots now fall back to names (1 red … 7 foreground, 9–15
  brights, 0 muted, 8 dark_foreground); explicit colorN always wins.
  Canvas/Colors swatches now follow the live Omarchy theme.
- QA: 53 tests (incl. headless wireframe-chrome snapshot + prompt
  round-trip + layout hit-region tests), clippy clean, golden intact,
  menu build reinstalled.

User reports: (1) rulers confusing → infinite canvas; (2) oval/rect should
use the palette char; (3) highlighting not working.

- **Rulers removed entirely** (`ui.rs`, `app.rs`, `main.rs`): no
  `show_rulers`, no `toggle_rulers`, Ctrl+R unbound. The repeating 0–9
  digits with no tens marker were the confusion; nothing replaces them.
- **Infinite canvas**: viewport/cursor unclamped (may go negative), no `~`
  filler rows, wheel pans vertical / Shift+wheel horizontal / middle-drag
  pans anywhere. `Document.grid` is now the new-file hint + minimum export
  frame only. Export (`txt`/`md`/`ansi`) expands the frame to content
  bounds, never shrinks: in-grid files render byte-identically (golden
  holds). `export_selection` no longer clips to the grid.
- **Rect tool** (`Tool::Rect`, Alt+D): plain rectangle outline in the active
  palette char + pots; Box stays the smart auto-junction tool. **Oval now
  paints `ellipse_cells` with the palette char** (`draw_ellipse` kept for
  compat/tests). `draw.rs` gained `rect_cells`/`ellipse_cells` (+3 tests,
  incl. key-parity with `draw_ellipse`).
- **Highlighting**: the select rubber-band had NO visual (only a status
  line) — that was the bug. Canvas now tints live-selection cells with the
  theme highlight bg, glyph preserved. (If the user meant painted-cell
  colors or cursor instead, those paths verified working.)
- QA: 46 tests green (incl. new bounds/frame + rect/oval-char tests),
  clippy clean, golden byte-identical, release rebuilt + menu reinstalled.

- `cargo test`: 42 passed, 0 failed (29 lib incl. golden demo, 7 app, 6 CLI).
- `cargo clippy --all-targets`: clean, zero warnings.
- Integration fix by manager: `src/bin/omaframe.rs` collided with
  `src/main.rs` (duplicate bin name). Resolved in Cargo.toml with
  `autobins = false` + `[[bin]] omaframe` (TUI) + `[[bin]] omaframe-export`
  (headless CLI). No agent files changed for this.
- Smoke: `omaframe-export --open testdata/demo.omaframe.json --export txt`
  is byte-identical to `testdata/demo.txt`; `--export md` wraps correctly.
- TUI with no TTY now exits with a friendly hint pointing at
  `omaframe-export` (manager micro-fix in `src/main.rs`).
- Theme probe: `~/.config/omarchy/current/theme` absent on this machine;
  falls back to `themes/picotron/colors.toml` as designed.
- Known gaps deferred to Phase 3: entity-aware select/move with handles,
  widget stamps (tab reports "Phase 3"), full cellContext line heuristic,
  live theme reload.

- `cargo test`: 15 passed, 0 failed (incl. golden demo.json→demo.txt byte-identical).
- `cargo clippy --all-targets`: 3 warnings, all fixed by manager (useless
  conversion + 2× clone_on_copy in src/model.rs). Tree is clippy-clean.
- `cargo build`: warning-free.
