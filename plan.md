# omaframe — Plan

> Fork of [lewish/asciiflow](https://github.com/lewish/asciiflow) reimagined as the best
> wireframing app for building TUI applications. Omarchy-first. Mouse-friendly.
> No vim bindings. Clean, simple, intuitive.

## 1. Vision

**omaframe** is a native terminal application for sketching TUI wireframes with
real terminal characters, real ANSI colors, and real terminal constraints
(fixed cell grid, monospace, Nerd Font glyphs).

Design principles:

1. **What you draw is what the terminal renders.** 1 cell = 1 character. No
   pixel canvas, no fake zoom, no proportional fonts on the canvas.
2. **Omarchy-first, canvas-exempt.** The app chrome follows the live Omarchy
   theme (`colors.toml`). The canvas shows *your wireframe's own* ANSI-16
   colors, plus a light/dark preview toggle so you can check contrast.
3. **Paint-program mental model, not CAD.** Foreground pot + background pot,
   pencil, eyedropper, big scrolling Palette — borrowed from Playscii, not
   from Figma.
4. **Mouse-first.** Click, drag, handles, scroll. Keyboard is shortcuts +
   command palette (`Ctrl-K`), never modal vim bindings.
5. **Agent-native.** Every wireframe is a `.oframe` file an agent can
   read, diff, generate, and modify, plus one-shot text export for pasting
   into a chat window.

### Non-goals (v1)

- No Electron (deleted), no Tauri webview, no browser build as primary target.
- No grid snapping — the character cell *is* the grid. Rulers/guides are
  visual only.
- No truecolor/RGB canvas in v1. ANSI-16 only (`color0`–`color15` from the
  active Omarchy theme). RGB is an explicitly deferred post-v1 item.
- No multi-user collaboration (PabloDraw-style). Local-first single user.
- No animation/onion-skinning (Playscii-style). Static wireframes only.

## 2. Background research

### 2.1 What asciiflow gives us (current `omaframe/main`)

| Area | Location | Reusable idea |
|---|---|---|
| Tools | `client/draw/box.ts`, `line.ts`, `select.ts`, `move.ts`, `text.ts`, `erase.ts`, `freeform.ts`, interface `draw/function.ts:IDrawFunction` | Command pattern: `start/move/end/handleKey/getCursor`. Keep box/line attach semantics. |
| Entity-aware select | PR #368, `draw/entity.ts`, `draw/select.ts` | Box-from-interior move, attached-line reflow, line-tip reshape, word drag, rubber-band. Port this logic, don't lose it. |
| Snap/normalize | `draw/snap.ts`, `draw/characters.ts`, `draw/utils.ts` | Junction auto-connect (`─│┌┐├┤┬┴┼`), deletion unsnap. Port to Rust. |
| Data model | `client/layer.ts:Layer(Map<Vector,string>)`, `LayerView` top-wins | Extend cell from `string` to `Cell{ch,fg,bg}`. Keep top-wins compose + undo-diff idea. |
| Rendering | `client/view.tsx` (HTML5 canvas, `font.ts` measuring, DPR scaling) | **Throw away.** Terminal cells are fixed; no font measuring, no DPR. |
| Persistence | `store/canvas.ts`, `store/index.ts` (localStorage), `drawing_stringifier.ts` (share URLs) | **Replace** with filesystem files + CLI export. |
| Build | Bazel 8 + esbuild + Electron 29 (`electron/BUILD`, `electron/index.js` ~40 LOC, no Node APIs) | **Replace** with Cargo. Electron deletion is trivial. |

### 2.2 ASCII paint programs (borrowed patterns)

- **Playscii** (primary model): pencil/fill/select/zoom/grab tools; space-hold
  quick picker; **left-click = foreground, right-click = background**; charsets
  and palettes as swappable sets; layers for fg/bg separation; plain-text
  import/export plugins; custom charset/palette files. We copy: fg/bg pots,
  grab (eyedropper), charset tabs, layers, text export.
- **PabloDraw / Sixteen Colors / ASCII Art Paint**: ANSI-first editing,
  16-color discipline, simple toolbars. Confirms ANSI-16 default is right for
  TUI work.
- **Monotext / Textik (web)**: drag-drop widget stamps for boxes/buttons.
  Confirms widget-stamp UX works for wireframes.

### 2.3 Unicode inventory (what goes in Palette)

- **Box Drawing** `U+2500–U+257F`: `─ │ ┌ ┐ └ ┘ ├ ┤ ┬ ┴ ┼`, heavy `━ ┃`,
  dashed `┄┆┈┊`, double `═ ║ ╔ ╗`, rounded `╭ ╮ ╯ ╰` (btop-style), diagonals
  `╱ ╲ ╳`.
- **Block Elements** `U+2580–U+259F` (the "textured checkbox" family):
  full `█`, shades `░ ▒ ▓` (25/50/75%), halves `▀ ▄ ▌ ▐`,
  eighths `▁▂▃▅▆▇ ▏▎▍▊▋`, quadrants `▖ ▗ ▘ ▝ ▚ ▞ ▟` (checkerboards).
- **Symbols**: `· • ◦ ○ ◉ ● ◆ ▲ ▼ ◄ ► ✓ ✗ ✕ │─ + - | _ / \ < > ^ v`
- **Nerd Fonts** (~11k PUA glyphs; curate ~100 to start): pc ``, folder,
  file/notepad, git, gear ``, terminal ``, box `󰏖`, check ``. Requires
  Nerd Font in terminal; warn + test page if missing.

### 2.4 Omarchy integration

- Theme source: `~/.config/omarchy/themes/*/colors.toml`
  (`accent, background, foreground, selection, muted, color0–color15`,
  plus bright variants). Live-switch via theme-set hook; subscribe or
  re-read on focus.
- Reference theme (dark `picotron`): bg `#1e1e2e`, fg `#cdd6f4`,
  accent `#89b4fa`, selection `#45475a`, muted `#585b70`.
- Font: **JetBrainsMono Nerd Font** default. Terminal font cannot be changed
  per-app, so "font preview" = test-page renderer + font-name readout +
  install hint, not in-canvas font switching.
- Packaging target: AUR + `omarchy install`-friendly script; single static
  Rust binary.

### 2.5 Native decision (locked)

User chose **terminal TUI app**, not Tauri desktop. Stack: **Rust +
Ratatui + crossterm**. Rationale: runs where the wireframes will run,
single binary, excellent mouse + ANSI support on Arch, no Chromium, no
webview rendering drift. Cost: full rewrite of view/controller/toolbar;
port (don't copy) layer/snap/entity logic.

## 3. Architecture

### 3.1 Process layout

```
omaframe (single Rust binary)
├── tui loop (crossterm events: mouse drag/scroll, keys, resize)
├── document model (grid + layers + undo)
├── tools (pencil/box/line/shapes/text/erase/grab/select/widget-stamp)
├── palette provider (built-in tabs + user .toml additions)
├── theme provider (read colors.toml → Ratatui Style)
├── file store (~/.local/share/omaframe/ + explicit .oframe paths)
├── exporters (txt, md, clipboard via wl-copy + OSC52 fallback)
└── cli (headless --export/--import for agents) + SKILL.md
```

### 3.2 Document model

```jsonc
// .oframe (v1)
{
  "version": 1,
  "name": "settings-screen",
  "grid": { "w": 80, "h": 24 },
  "layers": [
    { "name": "wireframe", "visible": true,
      "cells": [{ "x": 2, "y": 1, "ch": "╭", "fg": 4, "bg": -1 }] },
    { "name": "labels", "visible": true, "cells": [] }
  ],
  "preview": { "dark": true },
  "paletteTab": "outlines"
}
```

Rules:

- `fg`: 0–15 (Omarchy ANSI index). `bg`: 0–15 or `-1` = transparent.
- Compose: topmost visible layer wins per cell. **Space and erased (null)
  are transparent** — lower layers show through. No magic transparency char
  needed; eraser writes null (keeps layering predictable).
- Sparse storage (only non-transparent cells), like current `Layer` map.
- Undo: diff-stack per commit (port `apply→[new,undo]` idea), 50 steps,
  per-layer aware.
- Wide chars (CJK, some Nerd glyphs = 2 cells): `unicode-width` aware from
  day one; cursor skips continuation cells; export pads correctly.
- Max grid: 500×200 soft cap (terminal-realistic; current 2000×600 web cap
  is meaningless in-terminal).

### 3.3 Rendering

- Ratatui `Buffer`: one `Cell` per grid position, style from `fg/bg` indices
  resolved against *wireframe* palette (fixed ANSI slots), **not** app chrome
  theme. Chrome uses theme roles; canvas uses wireframe colors.
- Viewport: pan with middle-drag / scrollbars / `Ctrl-K → Go to`; no pixel
  zoom. Optional 2x block-preview (`▀` half-block trick) deferred to Phase 6
  if cheap.
- Rulers: row/col numbers on viewport edges (toggleable). No snapping.
- Dirty-region repaint; full repaint only on theme switch / resize.

### 3.4 Input (mouse-first, no vim)

- Click-drag draws; `Shift` constrains (square/circle, 1:1, 45° lines);
  `Alt` = straight-only for line; `Esc` cancels; `Ctrl-Z/Y` undo/redo;
  `Delete/Backspace` clears selection; arrows nudge selection by 1 cell.
- Right-click with pencil = pick background color (Playscii convention);
  middle-click or `G` tool = eyedropper (copies ch+fg+bg).
- Touch: treat as mouse (crossterm reports it as such where supported).
- All actions also in command palette (`Ctrl-K`), fully clickable.

## 4. UX design

### 4.1 Screen layout

```
┌─ toolbar (top, thin) ─────────────────────────────────────────┐
│ File  Undo  Tools  Layers  Preview ☾/☀  Export  ⧉ Cmd-K      │
├─ tools ┤┌─ canvas (wireframe colors) ─────────┐┌─ Palette ──┐│
│ pencil ││                                     ││ tabs:      ││
│ box    ││   click-drag to draw                ││ Letters    ││
│ line   ││   shift = snap ratio                ││ Numbers    ││
│ rrect  ││   handles ☐ drag to move/resize     ││ Symbols    ││
│ oval   ││                                     ││ Outlines   ││
│ text   ││                                     ││ Blocks     ││
│ erase  ││                                     ││ Nerds      ││
│ grab   ││                                     ││ Widgets    ││
│ select ││                                     ││────────────││
│        ││                                     ││ fg [ 4 ■]  ││
│        ││                                     ││ bg [ - ■]  ││
│        ││                                     ││ big scroll ││
│        ││                                     ││ grid of    ││
│        ││                                     ││ chars      ││
├─ layers┤└─────────────────────────────────────┘└────────────┘│
│ ▣ wire │ [ status: cell x,y · layer · fg/bg · file* · w×h ]  │
│ ▣ label│                                                     │
└─────────────────────────────────────────────────────────────┘
```

- **Toolbar**: File (new/open/save/save-as), Undo/Redo, Preview toggle,
  Export (txt/md/clipboard/selection), `Ctrl-K` hint. Text buttons, not
  icon-only (clarity over density).
- **Tools rail** (left, single column): pencil, box, line/arrow, rounded-rect,
  oval/circle, text, eraser, grab, select/move. Active tool highlighted with
  accent bar. Hover tooltip = name + shortcut.
- **Palette** (right, fixed name): tab row on top
  (`Letters Numbers Symbols Outlines Blocks Nerds Widgets`), fg/bg pots
  below tabs, then one big scrolling character grid. Click = set pencil char
  (or stamp widget). `Shift-click` or right-click = set as bg-accent where
  sensible. Tabs are sortable/reorderable; order persisted in config.
- **Layers** (bottom-left or toggleable left panel): list with
  show/hide eye, rename, add/delete/duplicate, drag-reorder. Suggested
  default: `wireframe` (structure) + `labels` (text overlay).
- **Status bar**: cursor cell, active layer, fg/bg swatches, file dirty dot,
  grid size, selection size when active.

### 4.2 Tools spec

| Tool | Behavior | Notes |
|---|---|---|
| Pencil | Drag paints active `ch` + fg/bg per cell | Core ask. Preview ghost on hover. |
| Box | Drag rect; light `┌┐└┘─│` default; style cycler (`L` key or toolbar dropdown: light/heavy/double/rounded/ascii `+-|`) | Keep asciiflow junction attach. |
| Line/Arrow | Drag polyline; horizontal-first heuristic (port `cellContext`), `Ctrl/Shift` flips axis, arrowhead toggle | Keep attach + `◄►▲▼` heads. |
| Rounded rect | Box with `╭╮╯╰` corners | Explicit ask (btop look). |
| Oval/Circle | Midpoint-ellipse outline on cell grid | `Shift` = circle / 1:1. Odd-dimension centering documented. |
| Text | Click → type overwrite; `Enter` commits, `Shift+Enter` newline, `Esc` cancels | Port overwrite semantics; insert-mode deferred. |
| Eraser | Drag writes transparent (null) | Respects active layer only. |
| Grab | Click copies ch+fg+bg into pencil | Playscii eyedropper. |
| Select/Move | Click entity or rubber-band; handles on corners/line-tips; drag moves, corners resize; arrows nudge | Port entity-aware logic (#368): box-interior grab, attached-line reflow, tip reshape, word drag. Extend handles to shapes + widgets. |

Shift-snap: rect→square, oval→circle, line→axis/45°. Show snap hint in
status bar while `Shift` held.

### 4.3 Palette tabs (initial content)

- **Letters**: `A-Z a-z` (52).
- **Numbers**: `0-9` + common numeric suffixes? Just digits to start.
- **Symbols**: `! @ # $ % ^ & * ( ) - _ = + [ ] { } ; : ' " , . < > / ? \ | ~ \``.
- **Outlines**: `─ │ ┌ ┐ └ ┘ ├ ┤ ┬ ┴ ┼ ╭ ╮ ╯ ╰ ━ ┃ ═ ║ ╔ ╗ ╱ ╲ ╳ + - |`.
- **Blocks** (shades/textures): `█ ▓ ▒ ░ ▀ ▄ ▌ ▐ ▖ ▗ ▘ ▝ ▚ ▞ ▟ ▁ ▂ ▃ ▅ ▆ ▇ ▏ ▎ ▍ ▊ ▋`.
- **Nerds** (curated ~100): pc ``, cpu ``, os ``, terminal ``, folder/file,
  gear ``, check ``, warn, branch, box `󰏖`, music/media, weather. Full 10k
  browser explicitly out of scope for v1; curated list in `assets/nerd.txt`.
- **Widgets** (stamps, not single chars — see §4.4).

Tab order user-sortable; persisted in `~/.config/omaframe/config.toml`.

### 4.4 Widget stamps (ASCII UI kit)

Stamp → drag-place → handles to move/resize. Resize reflows borders
(repeat `─`, shift label). v1 catalog:

- Button: `[ OK ]`, `[ Cancel ]` (focused: `⟨ OK ⟩` or `[>OK<]` variant)
- Input: `[________]`, `[ user_ ]`
- Dropdown: `[ option ▼ ]`
- Radio: `(◉) on  (○) off`
- Checkbox: `[x] yes  [ ] no`
- Toggle: `[●○]` / `[○●]`
- Close/X: `[X]`, `X`
- Scrollbar: `▲ ▼ █ ░` vertical/horizontal
- Progress: `[████░░░░] 60%`
- Divider: `────`, `════`
- Panel/window: rounded-rect + title bar `╭─ Title ───[X]─╮`
- Tabs: `[ Tab1 | Tab2 ]`

Each widget = parametric entity (`{kind, w, h, label, style}`), not baked
chars, so resize/relabel works. Serialized in `.oframe` as
`widgets[]` + baked cells fallback for export.

### 4.5 Layers UX

- Default two layers; add/rename/toggle freely.
- Paint/select/erase act on active layer only. Move across layers via
  `Ctrl-K → Move selection to layer`.
- Compose preview always on (stacked). Transparency = null/space shows
  through — demo this in the new-file template.
- No blend modes in v1 (top-wins only). Opacity deferred.

### 4.6 Command palette (`Ctrl-K`)

Fuzzy list over: tools, palette tabs, layer ops, file ops, export ops
(full/selection, txt/md/clipboard), preview toggle, go-to-cell, widget
insert, recent files. Mouse-clickable rows. This is the keyboard user's
front door — no vim modes required.

### 4.7 Fonts + Nerd icons

- Default: whatever the Omarchy terminal uses (expect JetBrainsMono Nerd
  Font). Show detected font name in status/settings (best-effort via
  `fc-match` + terminal query; never authoritative in-terminal).
- Settings → `Font test page`: renders outlines/blocks/nerds/curated widget
  rows so users can verify glyph coverage before wireframing.
- If Nerd glyphs show as tofu: banner with install hint
  (`omarchy install font` / [`nerdfonts.com`](https://www.nerdfonts.com)),
  plus one-click "hide Nerds tab" fallback.
- Font switching inside canvas is impossible in-terminal (cell grid is
  terminal-controlled) — document this; test page is the preview mechanism.

## 5. Files, export, clipboard

- Save: explicit `.oframe` anywhere (Ctrl-S / menu Save / Save As).
  Dirty dot, no autosave — writes happen only on explicit save.
- Export full or selection:
  - `.txt` — plain characters, trailing-space trimmed, ANSI stripped.
  - `.md` — fenced block (` ```text `) + optional title; for docs.
  - `.txt+ansi` (optional flag) — with SGR escapes for colored paste.
- Clipboard: prefer `wl-copy` (Wayland/Omarchy), fallback OSC52 for SSH.
  `Copy` button + `Ctrl-K → Copy selection as text` — the paste-into-agent
  flow.
- CLI (headless, for scripts/agents):
  `omaframe --open file.oframe --export txt --to out.txt [--selection x,y,w,h]`
  and `--export md`. Exit codes + stderr on bad geometry.

## 6. Agent skill

Ship `skills/omaframe/SKILL.md` + `schema.json`:

- What a `.oframe` means (grid, layers, fg/bg indices, transparency).
- How to render it to text (top-wins compose, trim, width rules).
- How to modify it (move widget = update cells/widgets[], recolor = change
  fg indices, add label = append text-layer cells).
- Guardrails: keep within grid, preserve layer names, don't invent codepoints
  outside Palette tabs, confirm Nerd glyph availability.
- Example: 3-line read + 5-line patch + export-to-txt round-trip.

The skill reads/writes the same schema as the app — no parallel format.

## 7. Theming (app chrome, not canvas)

- `theme provider` reads active `colors.toml` on start + on focus regain +
  on `theme-set` hook signal. Maps: `background→app bg`, `foreground→text`,
  `accent→active/selected`, `selection→highlight`, `muted→dim/borders`,
  `color0–15→wireframe palette slots`.
- Canvas cells resolve fg/bg via wireframe slots (stable indices), so a theme
  switch re-tints wireframe hues but never remaps structure.
- Preview toggle `☾/☀`: flips canvas *background* between Omarchy dark/light
  surfaces to check contrast; wireframe fg indices unchanged.
- Never do pixel-font work; never theme the terminal itself.

## 8. Phased build

- **Phase 0 — Repo hygiene** (small): `README` vision + attribution to
  asciiflow, `LICENSE` check, this `plan.md`, `assets/` seed
  (`nerd.txt`, widget catalog), `config.toml` sketch. Acceptance: docs only,
  no code.
- **Phase 1 — Rust scaffold + core grid**: Cargo bin, Ratatui shell, toolbar/
  status chrome, grid viewport, mouse drag, box/line/select port with attach +
  undo (50). Acceptance: draw box+line, move/resize, undo, save/load JSON.
- **Phase 2 — Paint core**: pencil, fg/bg pots (L=fg/R=bg), eyedropper,
  Palette tabs (Letters/Numbers/Symbols/Outlines/Blocks) + sort order,
  shift-snap. Acceptance: paint with any char + colors, eyedropper round-trip.
- **Phase 3 — Shapes + handles** ✅ SHIPPED (Wave 11): rounded-rect,
  oval/circle, widget stamps v1 (12 parametric kinds from
  `assets/widgets.toml`, re-baking resize) + 8 selection handles
  (corners resize widgets, else rubber-adjust) + block move + line-tip
  reshape, all single-entry undo. Deferred: box attach reflow, full
  cellContext heuristic.
- **Phase 4 — Layers + glyphs**: multi-layer compose + transparency, Nerds tab
  (~100) + font test page + missing-glyph banner. Acceptance: bg wireframe +
  text overlay composes; test page renders.
- **Phase 5 — Files + skill**: explicit save, txt/md/clipboard (full +
  selection), CLI export, `SKILL.md` + schema. Acceptance: file→export→paste-
  to-agent round-trip + agent JSON patch test.
- **Phase 6 — Command palette + polish**: `Ctrl-K`, templates
  (dashboard/form/split-pane), rulers toggle, preview toggle, AUR/packaging,
  snapshot tests on txt export. Acceptance: all actions reachable via palette;
  clean install on Omarchy.

## 9. Testing

- Unit (Rust): compose/top-wins, transparency, ellipse rasterization, junction
  attach, wide-char cursor math, JSON round-trip.
- Snapshot: `testdata/*.oframe → expected/*.txt` golden files
  (replaces Playwright e2e; terminal app has no DOM).
- Manual checklist per phase: mouse drag, shift-snap, handle resize, theme
  switch, Nerd-missing fallback, SSH/OSC52 clipboard.

## 10. Risks

| Risk | Mitigation |
|---|---|
| Full rewrite cost (React→Rust) | Phase 1 is minimal grid+box/line; port specs, not code; no web parity goal. |
| Wide-char/Nerd drift across terminals | `unicode-width` + test page + curated list; document Ghostty/Foot/Kitty variance. |
| Mouse in tmux/SSH | crossterm SGR mouse mode + fallback keyboard nudge; OSC52 clipboard. |
| Theme churn (Omarchy renames keys) | Tolerant TOML parse + defaults; never crash on missing key. |
| Scope creep (RGB, animation, collab) | Explicit non-goals; RGB tracked as post-v1 issue only. |

## 11. Open questions (for build-time confirmation)

1. Rust+Ratatui confirmed (vs Go Bubble Tea)? Assumed yes per "Terminal TUI".
2. Curated ~100 Nerd icons to start — OK, or want a different seed list?
3. Default grid size for new files (propose 80×24)?
4. Keep asciiflow-style share URLs at all, or filesystem-only? (Plan assumes filesystem-only.)

## Appendix A. Widget catalog (v1 seed)

`[ OK ]`, `[ Cancel ]`, `[________]`, `[ option ▼ ]`, `(◉)`, `(○)`,
`[x]`, `[ ]`, `[●○]`, `[X]`, `▲ ▼ █ ░` bars, `[████░░]`, `────`,
`╭─ Title ─[X]─╮` panel, `[ Tab1 | Tab2 ]`.

## Appendix B. Palette tab seeds

- Letters: `A–Z a–z`; Numbers: `0–9`; Symbols: `!@#$%^&*()-_=+[]{};:'",.<>/?\|~``
- Outlines: `─ │ ┌ ┐ └ ┘ ├ ┤ ┬ ┴ ┼ ╭ ╮ ╯ ╰ ━ ┃ ═ ║ ╔ ╗ ╱ ╲ ╳`
- Blocks: `█ ▓ ▒ ░ ▀ ▄ ▌ ▐ ▖ ▗ ▘ ▝ ▚ ▞ ▟`
- Nerds: see `assets/nerd.txt` (to be seeded in Phase 0).

## Appendix C. Example `.oframe` (tiny)

```json
{
  "version": 1,
  "name": "demo",
  "grid": { "w": 20, "h": 6 },
  "layers": [
    { "name": "wireframe", "visible": true, "cells": [
      { "x": 1, "y": 1, "ch": "╭", "fg": 4, "bg": -1 },
      { "x": 2, "y": 1, "ch": "─", "fg": 4, "bg": -1 }
    ]},
    { "name": "labels", "visible": true, "cells": [
      { "x": 3, "y": 3, "ch": "O", "fg": 7, "bg": -1 },
      { "x": 4, "y": 3, "ch": "K", "fg": 7, "bg": -1 }
    ]}
  ]
}
```
