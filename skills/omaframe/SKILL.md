---
name: omaframe
description: Read, render, and patch omaframe TUI wireframe files (.oframe): schema, text export via layer compose, edits (move/recolor/label), and guardrails. Use when viewing or modifying wireframes in this repo.
---

# omaframe skill

An `.oframe` file is a complete TUI wireframe: a fixed grid of
character cells plus stacked layers. Read it, render it to text, or patch it
with small scripts. The app (`omaframe`) reads and writes this same schema —
there is no parallel format.

Reference schema: `skills/omaframe/schema.json` (JSON Schema, draft 2020-12).
Example wireframe + golden text export: `testdata/demo.oframe`,
`testdata/demo.txt`.

## 1. Schema explanation

Top-level object:

| Field | Type | Required | Meaning |
|---|---|---|---|
| `version` | integer | yes | Always `1` in v1. |
| `name` | string | yes | Wireframe name, e.g. `"settings-panel"`. |
| `grid` | `{w, h}` | yes | Grid size in cells. 1 cell = 1 terminal character. Soft cap 500x200. |
| `layers` | array | yes | Bottom layer first. Default trio: `Background` + `Frames` (structure) + `Text` (labels). |
| `widgets` | array | no | Parametric entities (`{kind, x, y, w, h, label?, style?}`); the resize/relabel source of truth. `layers` holds the baked cells used for export. |
| `preview` | `{dark?}` | no | Dark/light canvas-background hint. Flips the background surface only; wireframe colors never change. |
| `paletteTab` | string | no | Last-active Palette tab (e.g. `"outlines"`). Informational only. |
| `activeLayer` | integer | no | Paint-target layer index (bottom-first). Absent in older files (= top layer active). |

Layer: `{name, visible, cells[]}`. `visible: false` layers are skipped when
composing. **Always preserve existing layer names** — tools and undo history
refer to them.

Cell: `{x, y, ch, fg, bg}`.

- `x`, `y`: 0-based, origin top-left. Must satisfy `0 <= x < grid.w`,
  `0 <= y < grid.h`.
- `ch`: exactly one character. Never store `null`, `""`, or `" "` — a missing
  entry already means "transparent" (sparse storage, like a map; at most one
  entry per `(x, y)` per layer).
- `fg`: a live ANSI slot `0`–`15` (follows the active theme), a live
  Omarchy theme variable (`"red"`, `"background"`, `"accent"`, … — also
  follows the theme; this is how the Colors panel paints), or a frozen
  truecolor hex `"#rrggbb"` (legacy files only, never re-tinted).
- `bg`: `-1` = transparent (no background paint), else like `fg`
  (slot, theme variable, or frozen hex).

Transparency and compose ("top-wins"):

- A cell is transparent when its layer is hidden, when it has no entry at
  `(x, y)`, or when the entry's `ch` is a space (spaces are never stored, so
  in practice: missing entry = transparent).
- The eraser writes transparency (deletes the entry); there is no magic
  transparency character.
- Compose order: layers array order, bottom first — the **topmost visible
  layer with a non-transparent cell wins** at each position.
- Typical pattern: the `Frames` layer draws full borders/widget frames and
  the `Text` layer overwrites spans of them with text (e.g. a title sitting
  on the top border). That is how `testdata/demo.oframe` works: the
  `Settings` title cells live in `Text` and cover the `─` cells beneath them.

## 2. How to render to text

Plain-text export (`.txt`): characters only, ANSI stripped.

```
for each y in 0..grid.h-1:
  for each x in 0..grid.w-1:
    ch = " "
    for layer in layers (bottom to top):
      if layer.visible and layer has entry at (x, y)
         and entry.ch is not space-like:
        ch = entry.ch
    emit ch
  strip trailing spaces from the line (leading spaces are kept)
emit all grid.h lines (including all-space lines, which become empty)
```

Width rule: terminals are monospace, but some glyphs (CJK, some Nerd icons)
are 2 cells wide. When emitting text, assume width 1 unless you have
`unicode-width` data; never split a wide char across columns. The demo files
use width-1 glyphs only.

The CLI equivalent (once built): `omaframe --open <file> --export txt --to <out>`.

## 3. How to modify

Edit the JSON (or the file via the app), keeping `widgets[]` and baked cells
in sync: if a widget moves or relabels, update its `widgets[]` entry **and**
its baked cells.

- **Move** = recompute coordinates. Shift every affected cell by `(dx, dy)`
  and update the matching `widgets[]` `{x, y}`. Touch all layers that contain
  parts of the moved object (e.g. brackets in `Frames`, letters in
  `Text`). Delete entries that would leave the grid instead of clamping
  them into place silently — or rather: refuse and report; never wrap.
- **Recolor** = change `fg` (or `bg`) in place. Coordinates and `ch`
  stay identical. Example: selected-state highlight = `fg 7` -> `fg 6`.
  Integer slots and theme-variable names are live (follow the theme);
  `"#rrggbb"` strings are frozen RGB (legacy).
- **Label** = append cells to the topmost text layer (`Text`): one cell per
  character, starting at the label's `(x, y)`, skipping spaces (transparent —
  do not store them). Overwriting border cells underneath is expected and
  correct (top-wins).

After any patch: re-render to text and diff against the previous rendering to
confirm only the intended lines changed.

## 4. Guardrails

1. **Stay in the grid.** `0 <= x < w`, `0 <= y < h`. No wrapping, no negative
   coordinates, no resizing the grid to fit a misplaced edit.
2. **Keep layer names.** Never rename, delete, or reorder layers unless asked.
   Paint into the layer that owns the object kind (structure -> `Frames`,
   words -> `Text`).
3. **Only Palette codepoints.** Use characters from the Palette tabs
   (Letters, Numbers, Symbols, Outlines, Blocks, curated Nerds, widget kit).
   Never invent codepoints, emoji, or lookalike glyphs. Box drawing:
   light `─ │ ┌ ┐ └ ┘ ├ ┤ ┬ ┴ ┼`, rounded `╭ ╮ ╯ ╰`, heavy `━ ┃`, double
   `═ ║ ╔ ╗`; blocks `█ ▓ ▒ ░ ▀ ▄ ▌ ▐`; symbols `[]()|/\<>+-:.%#*`.
4. **Nerd availability.** Nerd Font glyphs render only where a Nerd Font is
   installed. Prefer plain glyphs; if you must use a Nerd icon, use one from
   the curated set, say so explicitly, and never use it for load-bearing
   structure (borders, alignment).
5. **One entry per (x, y) per layer.** A layer is a map. Duplicates are a bug.
6. **Colors are slots, variables, or hex.** `fg` is `0`–`15`, a theme
    variable name (`"red"`, …), or `"#rrggbb"`; `bg` adds `-1` for
    transparent. Prefer theme variables for anything that should re-tint
    with the theme. `bg: -1` unless you mean to paint a background swatch.
    The demo uses integer slots only.
7. **Round-trip check.** After patching, validate the JSON parses, re-export
   text, and confirm the diff is exactly what you intended.

## 5. Worked example

Goal: inspect the demo, move the `[ OK ]` button one cell right, and verify
via the golden export. Paths are relative to the repo root.

Read (shape of the file — grid, layers, widgets):

```python
import json
doc = json.load(open("testdata/demo.oframe"))
print(doc["grid"], [l["name"] for l in doc["layers"]], len(doc["widgets"]))
```

Patch (move the OK button at `(5, 2)` right by 1: brackets in the structure
layer, letters in the text layer — `wireframe`/`labels` in this demo file,
`Frames`/`Text` in new files — plus the `widgets[]` entry):

```python
import json
p = "testdata/demo.oframe"
doc = json.load(open(p))
ok = {"x0": 5, "x1": 10, "y": 2}          # button span in baked cells
for layer in doc["layers"]:               # shift baked cells in both layers
    for c in layer["cells"]:
        if c["y"] == ok["y"] and ok["x0"] <= c["x"] <= ok["x1"]:
            c["x"] += 1                   # recompute: rigid shift, no wrap
doc["widgets"][1]["x"] += 1               # keep the parametric entry in sync
json.dump(doc, open(p, "w"), indent=2, ensure_ascii=False)
```

Export round-trip (compose per section 2, diff against the golden snapshot):

```python
import json
doc = json.load(open("testdata/demo.oframe"))
w, h = doc["grid"]["w"], doc["grid"]["h"]
grid = [[" "] * w for _ in range(h)]
for layer in doc["layers"]:
    if layer["visible"]:
        for c in layer["cells"]:
            if c["ch"].strip():
                grid[c["y"]][c["x"]] = c["ch"]
open("/tmp/roundtrip.txt", "w").write("\n".join("".join(r).rstrip() for r in grid) + "\n")
```

```bash
diff /tmp/roundtrip.txt testdata/demo.txt && echo EXPORT-OK
```

For the unmodified demo the diff is empty (`EXPORT-OK`). After the patch
above, the diff shows exactly one changed line (the buttons row) — that
single-line diff is the signal the move was clean and nothing else shifted.
```

