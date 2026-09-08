# omaframe TUI — Tools Spec

> Implementor spec for all 9 tools on the Rust/Ratatui port.
> Ported from (behavior only, not code):
> `client/draw/box.ts`, `client/draw/line.ts`, `client/draw/utils.ts`,
> `client/draw/select.ts`, `client/draw/entity.ts`, `client/draw/move.ts`,
> `client/draw/text.ts`, `client/draw/erase.ts`, `client/draw/freeform.ts`,
> `client/draw/function.ts`, plus junction logic in `client/characters.ts`,
> `client/snap.ts`, charsets in `client/constants.ts`, undo in
> `client/store/canvas.ts`, `client/layer.ts`.
> New tools with no TS source (rounded-rect, oval/circle, grab, pencil-with-color)
> are marked **[NEW]** — the algorithm given there is normative for v1.

## 0. Shared tool protocol

Every tool implements the `IDrawFunction` shape (`client/draw/function.ts`):

```
start(pos, mods) -> begin gesture, write preview into scratch
move(pos, mods)  -> rebuild scratch preview (pure function of anchor+pos+mods)
end()            -> commitScratch() OR clearScratch() (see §9, undo rules)
handleKey(k, mods) -> live redraw (line-axis flip) / text input / style cycle
getCursor(pos, mods) -> cursor name for status bar / terminal cursor shape
cleanup()        -> tool switch: drop drag state; text tool COMMITS (see §6)
```

Rules that apply to **all** tools:

- `move()` **never** touches committed state. It rebuilds a fresh scratch
  layer from `(anchor, currentPos, mods)` every call. Preview is idempotent:
  calling `move(p)` twice yields the same scratch.
- `commitScratch()` = `apply(scratch)` → push undo diff **only if the diff is
  non-empty** (`client/store/canvas.ts:219-238`). No-op gestures
  (1×1 box, unmoved tip reshape, Esc-cancel) push **nothing**.
- Undo depth cap: **50** (`MAX_UNDO`, `client/constants.ts:111`). TS grows the
  array unbounded in practice; Rust **must** truncate (`drain(..len-50)`).
- Any new commit clears the redo stack.
- All drawing acts on the **active layer only**. Compose/preview always shows
  the full stack (see `docs/model-api.md`).
- Coordinates are terminal cell coords `(x: i32, y: i32)`, y grows downward.
  Tools must accept negative coords (drawing off-viewport is legal; the
  viewport clips at render). Do **not** clamp to grid in tools — clamp in the
  viewport renderer. (TS grid is effectively unbounded, `MAX_GRID_*` is only a
  recenter default.)
- `Esc` during any drag = cancel: `clearScratch()`, drop gesture state,
  push nothing.
- Modifier snapshot: `mods = { shift, ctrl, alt }`. `ctrl` and `meta` are
  treated identically wherever the TS checks `ctrl || shift`
  (`line.ts:64`, `select.ts:321`, `text.ts:39`).
  `Alt` = straight-only for line (plan §3.4, **[NEW]**, §3.5).

Cursor names (map to terminal cursor shapes in the TUI as you see fit):
`crosshair` (box/line/oval/eraser/pencil), `text` (text),
`move` (select drag / word / interior), `ns-resize` (grabbed `─`),
`ew-resize` (grabbed `│`), `default` (empty space).

---

## 1. Pencil

TS analog: `DrawFreeform` (`client/draw/freeform.ts`) + color pots (plan §4.2,
no TS source — colors are **[NEW]**).

- `start(pos)`: scratch = `{ pos: (activeCh, fg, bg) }`.
- `move(pos)`: **accumulate** — clone current scratch, set `pos`. (Unlike
  box/line, pencil is additive across a drag; TS does
  `new Layer().apply(this.currentLayer)` then set, `freeform.ts:16-19`.)
- `end()`: `commitScratch()`. One drag = **one** undo entry no matter how
  many cells were painted.
- `handleKey(ch)`: if single printable char, it becomes `activeCh`
  (TS: `store.setFreeformCharacter`, `freeform.ts:29-33`). Rust: same, plus it
  highlights the matching Palette cell if present.
- Click without drag paints exactly one cell. Painting the same ch+colors over
  an identical cell still commits (diff is computed vs committed, so a true
  no-op push is impossible — `apply` yields an empty undo layer).
- **No junction auto-connect.** Pencil writes raw cells, even when `activeCh`
  is a box-drawing char. Fix-up happens only when a line/box/select gesture's
  `snap()` pass touches those cells afterwards. (Decision — see open Q7.)
- Right-click with pencil = set **bg pot** to the current fg color
  (Playscii convention, plan §3.4), does not paint.
- Edge: painting a wide char (CJK / some Nerd glyphs) — store at the anchor
  cell; renderer treats the next cell as continuation (see `docs/model-api.md`).
  Painting onto a continuation cell overwrites the wide char's anchor
  implicitly (erase anchor when its continuation is overwritten).

## 2. Box

TS source: `client/draw/box.ts:16-45`.

Given `anchor` (`start`) and `cursor` (`move`):

1. Normalize: `left=min(ax,cx)`, `right=max(...)`, `top=min(ay,cy)`,
   `bottom=max(...)` (`Box`, `client/common.ts:10-24`).
2. If `left != right`: for `x in left..=right` set `(x,top)` and `(x,bottom)`
   to horizontal edge char (default `─`).
3. If `top != bottom`: for `y in top..=bottom` set `(left,y)` and `(right,y)`
   to vertical edge char (default `│`).
4. If **both** dimensions non-zero: overwrite the 4 corners with
   `┌ ┐ └ ┘` (`topLeft/topRight/bottomRight/bottomLeft`).
5. `scratch.setFrom(snap(scratch, committed))`, then `setScratchLayer`.
6. `end()` → `commitScratch()`.

Edge cases (normative):

| Gesture | Result |
|---|---|
| 1×1 (anchor == cursor) | **Empty scratch** — both `!=` guards fail and the corner guard fails. Commit is a no-op (no undo entry). |
| 2×1 / N×1 horizontal (`top == bottom`, `left != right`) | Single row of `─`, **no corners**. |
| 1×2 / 1×N vertical | Single column of `│`, **no corners**. |
| 2×2 | 4 corners only (edges coincide with corners; corners overwrite). |
| Drag in any direction | Identical output — normalization makes direction irrelevant. |

Style cycler **[NEW]** (plan §4.2, issue #241): `L` key or toolbar dropdown
cycles `light → heavy → double → rounded → ascii`. Corner/edge mapping:

| Style | Horiz | Vert | Corners TL TR BR BL |
|---|---|---|---|
| light | `─` | `│` | `┌ ┐ ┘ └` |
| heavy | `━` | `┃` | `┏ ┓ ┛ ┗` |
| double | `═` | `║` | `╔ ╗ ╝ ╚` |
| rounded | `─` | `│` | `╭ ╮ ╯ ╰` |
| ascii | `-` | `\|` | `+ + + +` |

> The heavy/double corner sets have **no TS precedent** (`characters.ts` only
> knows the light set) — junction/snap tables for them are open Q1. v1 MUST at
> least draw them; auto-connect for non-light styles is SHOULD (fall back to
> no-connect if the tables are absent).

`Shift` + box = square: side = `max(right-left, bottom-top)`, expand the
shorter axis **away from the anchor** (anchor corner stays fixed). Show
`□ snap` hint in status bar while held (plan §4.2).

## 3. Line / Arrow

TS sources: `client/draw/line.ts:36-122`, `client/draw/utils.ts:5-76`.

### 3.1 Geometry — `line(start, end, horizontalFirst)`

- Axis-aligned (`start.x == end.x` or `start.y == end.y`): `straightLine` —
  fill the inclusive range with `│` (vertical) or `─` (horizontal).
- Otherwise `cornerLine`: single-bend polyline.
  - Bend point: `(end.x, start.y)` if `horizontalFirst`, else
    `(start.x, end.y)`.
  - Draw leg1 `start→bend` and leg2 `bend→end` as straight lines, then set
    the bend cell per this exact table (`utils.ts:35-50`):

```
horizontalFirst, start.x<end.x, start.y<end.y → ┐   (bend opens left-down)
horizontalFirst, start.x<end.x, start.y>end.y → ┘
horizontalFirst, start.x>end.x, start.y<end.y → ┌
horizontalFirst, start.x>end.x, start.y>end.y → └
verticalFirst,   start.y<end.y, start.x<end.x → └
verticalFirst,   start.y<end.y, start.x>end.x → ┘
verticalFirst,   start.y>end.y, start.x<end.x → ┌
verticalFirst,   start.y>end.y, start.x>end.x → ┐
```

  (Mnemonic: the corner glyph is the one whose two arms point back at `start`
  and `end`. Implement it that way — `cornerFor(a=start-bend, b=end-bend)` —
  rather than copying the nested ternary.)

### 3.2 Horizontal-first heuristic (`line.ts:52-64`)

```
hStart = (ctx(start).up && ctx(start).down)
      || (ctx(start).leftup && ctx(start).leftdown)
      || (ctx(start).rightup && ctx(start).rightdown)
vEnd   = (ctx(end).left && ctx(end).right)
      || (ctx(end).leftup && ctx(end).rightup)
      || (ctx(end).leftdown && ctx(end).rightdown)
horizontalFirst = (hStart || vEnd) XOR (mods.ctrl || mods.shift)
```

`ctx` = `cellContext` (`client/render_layer.ts:259-292`): the 8-neighbourhood
booleans, where a neighbour counts as `true` iff it holds any of
`ALL_SPECIAL_VALUES` (15 box/arrow glyphs, `constants.ts:107`). Diagonals are
only used in the `hStart`/`vEnd` patterns above — never as connections.

Rationale to preserve: a line **leaving** a vertical run starts horizontal;
a line **arriving** into a horizontal run arrives vertical. The default
(no context, no mods) is `false` = vertical-first.

`handleKey` re-runs `draw()` with the latest mods (`line.ts:130-132`), so
pressing/releasing Ctrl/Shift **mid-drag live-flips** the bend. Rust must do
the same (recompute scratch from stored anchor/cursor on mod change).

### 3.3 Arrowheads (`line.ts:68-93`, `select.ts:416-427`)

If arrow mode: overwrite the end cell:

```
end.x == start.x → end.y < start.y ? ▲ : ▼
end.y == start.y → end.x < start.x ? ◄ : ►
diagonal + horizontalFirst → ▲ / ▼ (by sign of end.y-start.y)
diagonal + verticalFirst   → ► / ◄ (by sign of end.x-start.x)
```

I.e. the head points along the **last leg's** direction.

### 3.4 Endpoint connect + disconnect (`line.ts:94-117`)

For `[start]` (arrow) or `[start, end]` (plain line):

```
incoming = dirs d where combined.get(pos+d) connects back (d.opposite)
           AND layer.get(pos) is connectable toward d
layer.set(pos, connect(layer.get(pos), incoming))
layer.set(pos, disconnect(layer.get(pos), ALL - incoming))
```

`combined = committed + scratch`. Arrows: `connectable` is only the backward
dir, so a line touching an arrowhead's point does **not** merge into it.
Then `snap(scratch, committed)` as usual.

### 3.5 Zero-length line

`start == end` → `straightLine` draws a **single `│`** cell (x-equal branch,
one iteration). Arrow mode overwrites it with `▼` (x-equal, `end.y < start.y`
false). Then endpoint normalization runs. So a click in line mode is a dot,
a click in arrow mode is a `▼`. (Quirk port — keep it; do not special-case.)

### 3.6 [NEW] Alt = straight-only

While `Alt` is held during a line drag, force `straightLine` along the
**dominant axis** (`|dx| >= |dy|` → horizontal at `start.y` from
`start.x→end.x`; else vertical). Bend/corners suppressed. `Shift` keeps its
TS meaning (axis **flip**, §3.2), NOT straight-constrain — the plan's
"line→axis/45°" row (§3.4) is otherwise in direct conflict with the ported
XOR heuristic. True 45° diagonals (`╱ ╲ ╳`, issue #43) are **deferred**;
do not invent diagonal routing in v1 (open Q2).

## 4. Rounded rect **[NEW]**

Byte-identical to §2 Box except the corner row of the style table:
`╭ ╮ ╯ ╰` with `─`/`│` edges, followed by the same `snap()` pass.
Junction connect tables treat `╭╮╯╰` as **non-connecting decoration**
(they are absent from `characters.ts`'s `BOX_DRAWING_INFO`): lines touching a
rounded corner do not auto-merge into it; lines touching its straight edges
`snap()` normally. (If we later add rounded corners to the connect tables,
that is a junction-table upgrade per Q1 — behavior change must be snapshotted
in tests.)

## 5. Oval / Circle **[NEW]**

No TS source (issue #229). Normative v1 algorithm:

1. Normalize anchor/cursor to a bounding box exactly as §2 step 1.
   `Shift` forces a square box first (same anchor-fixed rule as §2).
2. Degenerate cases: `w == 0 && h == 0` → single cell gets `─`… **no**:
   single cell gets the horizontal edge char `─` (matches line-dot
   convention of drawing *something*; snapshot it). `w == 0` → vertical
   `│` run; `h == 0` → horizontal `─` run. (Same as the 1-wide box rows.)
3. Otherwise run the **integer midpoint ellipse** over the box:
   - `cx = (left+right)/2.0`, `cy = (top+bottom)/2.0`,
     `rx = (right-left)/2.0`, `ry = (bottom-top)/2.0` (floats — this is what
     makes **odd dimensions center on a cell** and even dimensions straddle
     a grid line symmetrically).
   - Rasterize the standard midpoint/bresenham ellipse decision loop in the
     first quadrant, mirror to 4 quadrants, round each plotted point to the
     nearest cell, dedupe.
   - `rx == 0` / `ry == 0` fall back to the straight runs in step 2
     (the loop would divide by zero).
4. Glyph per outline cell by local slope: cells where the ellipse tangent is
   more horizontal (`|dy/dx| < 1` between neighbours) get `─`, more vertical
   get `│`. Axis extremes fall out naturally (top/bottom rows `─`,
   left/right columns `│`). **No corner glyphs, no `snap()`** — ovals never
   auto-junction in v1 (keeps snapshots stable; revisit with Q1).
5. `end()` → `commitScratch()`.

Even-dimension note (snapshot this): a 6-wide circle has its center on a grid
**line**; rounding to nearest cell yields a symmetric 2-cell-wide flat at each
extreme. Implementations must round halves **away from center** consistently
so left/right (and top/bottom) mirrors match exactly. Property test:
`outline == mirror_x(outline) == mirror_y(outline)` for all boxes 1×1..30×12.

Minimum visible ovals: 2×2 → 4-cell ring (`─` top/bottom rows coincide… in
practice the 4 cells: top row `──`, bottom row `──`); 3×3 → 8-cell ring with
the center empty. Snapshot both.

## 6. Text

TS source: `client/draw/text.ts` (overwrite semantics; insert mode deferred,
issue #240).

- `start(pos)`: set cursor + `newLineAlignment = pos`; show 1-cell selection
  box at cursor as caret; ensure a working `textLayer` (empty scratch).
- Printable key: `textLayer.set(cursor, ch)` (fg/bg = pots), cursor moves
  **right by 1**; update scratch + caret. Multi-cell paste: iterate chars,
  `\n` moves to `(newLineAlignment.x, cursor.y+1)` — reuse `drawText`
  (`utils.ts:82-93`): `x/y` offsets from click point, `\n` resets `x=0, y++`.
- `Enter` (no mods): **commit** scratch, clear `textLayer` (session ends, one
  undo entry for the whole typed string).
- `Shift/Ctrl/Meta+Enter`: newline to `(newLineAlignment.x, cursor.y+1)`
  (stays in session).
- `Backspace`: cursor moves **left 1**, then writes deletion at the new cursor
  (`text.ts:49-53` writes `" "`; TS `apply` treats `" "`/`""` identically as
  delete, `layer.ts:96-97`). Rust: write **null/erase** on the active layer
  (identical compose result, since space is transparent — see model-api).
  Clamp: do not move left past `newLineAlignment.x`… TS does not clamp, but
  unbounded left-travel on repeated backspace is a UX trap; v1 clamps at the
  session's leftmost x (open Q5 if you disagree).
- Arrow keys move the cursor (no text change); caret follows.
- `Delete` key: same as Backspace but **without** moving first (erase at
  cursor; **[NEW]** — TS ignores Delete in text mode; plan §3.4 wants it).
- Text never triggers `snap()`. Typing over box-drawing chars overwrites them
  raw; neighbours are **not** unsnapped until a later erase/select gesture
  runs `snap()` (matches TS: no snap call in `text.ts` at all).
- `cleanup()` (tool switch) **commits** pending text (`text.ts:82-87`).
  `Esc` **[DECISION]**: Esc commits too (same as tool switch — typed chars are
  never lost; there is no cancel for text). Documented here so the scaffold
  agent doesn't invent Esc-discards-text.

## 7. Eraser

TS source: `client/draw/erase.ts:10-33`.

- `start(pos)` anchors; `move(pos)` rebuilds scratch as the **filled rect**
  `anchor..cursor` (inclusive, normalized) with every cell set to deletion
  (`""` in TS = null/erase in Rust — active layer only).
- `end()` → `commitScratch()`. The commit path **must** run the snap unsnap
  pass (`snap.ts:69-96`): neighbours of deleted cells that pointed at them get
  `disconnect()`ed (e.g. erasing the middle of `──┼──` leaves `──┤ ├──`-style
  tidied ends after normalization — snapshot this).
- Single click = erase exactly one cell. Small eraser only in v1 (no size
  slider; full-rect semantics make size control unnecessary).
- Eraser writes affect **active layer only**; lower layers show through
  (space/null transparent). Erasing on the top layer over a lower-layer char
  reveals the lower char — demo this in the new-file template (plan §4.5).

## 8. Grab (eyedropper) **[NEW]**

Playscii convention (plan §2.2, §3.4). No TS source.

- Click: read `compose(pos)` (topmost visible non-transparent cell):
  - Hit (ch + fg + bg): set pencil `activeCh = ch`, fg pot = fg, bg pot = bg.
  - Miss (transparent all the way down): change nothing (status bar shows
    `empty` flash; no error, no undo entry).
- No drag, no scratch, no undo entry, no selection change. `G` or
  middle-click also grabs regardless of active tool (plan §3.4); `Esc`/tool
  switch is a no-op.
- Wide-char continuation cell: resolve to the anchor cell first.

## 9. Select / Move (entity-aware)

TS sources: `client/draw/select.ts` (priority dispatch, drags),
`client/draw/entity.ts` (detection + reflow math),
`client/draw/move.ts` (legacy line-slide sub-tool).

### 9.1 Dispatch — `start(pos, mods)` priority (normative order)

1. **Inside live selection** (`!shift`, `selectedCells` non-empty,
   `selectBox.contains(pos)` and canvas selection live): `beginDrag` (move
   the whole selection). Note the subtlety (`select.ts:80-87`): containment
   is by **bounding box**, not cell membership — grabbing empty space inside
   the bbox still drags.
2. **`shift` held**: `entityAt(pos)` → word else box-cells → **add** to
   selection (union+dedupe, `select.ts:90-98,226-228`); if no entity, start
   rubber-band. Shift never replaces.
3. **Free line tip** (`detectLineTip`): enter tip-reshape mode (§9.3).
4. **Word** (`detectWord`): select word cells, `beginDrag` immediately
   (word drag, §9.4).
5. **Box-drawing char** (`isBoxDrawing(value)`): delegate to `DrawMove`
   sub-tool (§9.6). Box borders stay resizable this way; interior moves via
   rule 6.
6. **Box interior** (`findBox`): select `cellsInBox`, set `activeBox`,
   `beginDrag` (box move with reflow, §9.5).
7. **Else**: rubber-band select (§9.7).

Cursor mirrors this (`select.ts:356-376`): in-selection→`move`,
tip→`crosshair`, word→`move`, box-drawing→DrawMove cursor (`─`→`ns-resize`,
`│`→`ew-resize`, other special→`move`), interior→`move`, else→`default`.

### 9.2 Primitives

- `isContent(v) = v != null && v != "" && v != " "` (`entity.ts:18-20`).
- `isText(v) = isContent(v) && !isBoxDrawing(v)` — arrows count as
  box-drawing (break words); rounded `╭╮╯╰` / heavy / double are **not** in
  the TS set, so they count as *text* for word detection. Rust: word detection
  must use the **extended** box set (light+heavy+double+rounded+ascii+junction
  tables) or words will swallow rounded corners. (Decision forced by §2's
  style cycler — snapshot `detectWord` stopping at `╭`.)
- `boundingBox(cells)`: min/max; null on empty (`entity.ts:585-600`).
- `cellsInBox(layer, box)`: committed keys with `box.contains && isContent`
  — content only, empties are not selected (`entity.ts:297-302`).

### 9.3 Line-tip reshape

- `detectLineTip(layer, pos)` (`entity.ts:63-97`): value must be `─` (axis
  horizontal), `│` (vertical), or arrow (`◄►` horizontal, `▲▼` vertical);
  exactly **one** neighbour along that axis must `connect` back. Corners,
  junctions (`┼├┤┬┴`), and mid-line straights (two neighbours) are NOT tips.
- `traceLineFromTip(tip, bodyDir)` (`entity.ts:105-127`): walk from
  `tip+bodyDir` through straight chars; stop **including** the first `BEND`
  (`┌┐└┘` connecting back) — that corner is the pivot/`anchor`. Straight run
  with no bend → anchor = far end. Cap 1000 steps. Returns `{cells, anchor}`:
  `cells` = old run to erase.
- Drag: `base = committed − cells` (so snap doesn't latch onto the old run);
  scratch clears `cells`, draws `line(anchor, target, horizontalFirst)` +
  arrowhead if the tip was an arrow. Axis rule here differs from §3.2:
  `horizontalFirst = horizontalSegment XOR flip` where `horizontalSegment` is
  the tip's own axis and `flip = ctrl||shift` (`select.ts:321-322`) — bending
  "turns" the last segment rather than re-inferring context. Endpoint
  connect/disconnect on `[anchor]` (arrow) or `[anchor, target]` (plain),
  then `snap(scratch, base)`, `clearSelection()`.
- `anchor == target`: draw nothing (empty scratch over the erasures — net
  effect deletes the run; matches TS `if (!anchor.equals(target))` guard,
  `select.ts:317`).
- `end()`: moved → `commitScratch()` (one undo entry); unmoved →
  `clearScratch()` (nothing pushed).

### 9.4 Word drag

- `detectWord(layer, pos)` (`entity.ts:31-48`): if `isText(pos)`, expand left
  and right while `isText`; return the full run (single row). A lone char is
  a 1-cell word. Spaces/box-drawing bound the run.
- `moveCells(committed, cells, delta)` (`entity.ts:308-331`): scratch erases
  originals, redraws translated content (overlapping erasures overwritten by
  the moved chars — draw order matters), then `snap(layer, committed)`.
  Note: TS passes **no protect set** here (unlike box move), so normalization
  may retouch moved line glyphs — port as-is; flag in Q6.

### 9.5 Box move with attached-line reflow

- `findBox(layer, pos)` (`entity.ts:246-295`): two candidate paths, smallest
  area wins:
  - **Seed rays** (`boxFromSeed`, `RAY_LIMIT=400`): from interior seeds cast
    LEFT/RIGHT for a vertical-border cell and UP/DOWN for a horizontal-border
    cell; build the rect; require `w,h ≥ 1` and `verifyPerimeter` (every edge
    cell `isBoxDrawing`). Seeds: `[pos]` if pos is not box-drawing, else all 8
    neighbours (4 orthogonal + 4 diagonal — corners need the diagonal seed).
  - **Border flood** (`borderComponent`, `COMPONENT_LIMIT=4000`): 4-way flood
    over box-drawing cells from pos; bbox accepted iff `w,h ≥ 1` and
    `verifyPerimeter`. This is the only path for thin boxes with no interior
    (e.g. 2 rows tall). Attached lines make the bbox over-grow → perimeter
    check rejects → correctly returns null (it's not an isolated box).
  - Nested boxes: smallest area wins.
- `traceBoxAttachments(layer, box)` (`entity.ts:390-442`): per perimeter cell
  + outward dir, two cases: **A** — straight char directly outside pointing
  back; **B** — arrowhead pointing into the box with a straight char behind
  it. Walk each out through straights + `BENDS` (`turnFrom`) up to 1000 steps
  to the terminal `far` (arrow / junction / other box / empty). Record
  `{anchor, out, far, runCells, arrowIntoBox}`.
- `moveBoxWithAttachments(committed, box, attachments, delta)`
  (`entity.ts:536-582`): erase box content + old runs; redraw content at
  `+delta`; per attachment `drawConnector(newAnchor, far, out, farValue)`:
  L-shape, horizontal-first iff `out` is LEFT/RIGHT, bend at the elbow,
  `cornerFor` glyph unless the bend coincides with an endpoint, re-point arrow
  at `far` to the new approach dir; arrow-into-box rides along (re-stamped at
  the moved edge, still pointing in). `snap(..., protect=movedBoxCells)` —
  the box itself is **never** normalized, only connectors + surroundings.
- `end()` follows the content: recompute `cellsInBox` at the moved rect, keep
  it highlighted and keep `activeBox` live for re-drags (`select.ts:166-188`).
  Non-box drags translate `selectedCells` and re-set the bbox
  (`keepScratch=true` — scratch already shows the result; do not clear it).

### 9.6 Legacy line-slide (`DrawMove`, `client/draw/move.ts`)

Reached only via §9.1 rule 5, and only **starts** on `─`/`│` (other
box-drawing → `trace` stays null → `move` no-ops, `move.ts:13-24`).

- `traceLine`: walk the axis while mutual `connects` hold (both sides agree);
  collect `positions`. Perpendicular `connects` (+ arrow-behind-line case)
  become `attachments` with traced straight runs.
- `move`: clamp the cursor into the attachment bounding box
  (`minX/maxX/minY/maxY` from attachment ends — the line cannot be dragged
  past what it's attached to); slide the whole run **perpendicular** by
  `moveUnits`; clear attachments on the push side, extend them on the trail
  side (`move.ts:32-106`). Arrows in the trail path are a known TODO
  (`move.ts:95`) — port the TODO, don't fix it.
- `end()` → `commitScratch()`.

### 9.7 Rubber-band

- `startSelect`: anchor=pos, `selectBox = Box(pos,pos)`, clear cells.
- `moveSelect`: `selectBox = Box(anchor, pos)` (direction-free).
- `finishSelect`: `selectedCells = cellsInBox(committed, selectBox)` and
  **`activeBox = selectBox`** (`select.ts:253-261`): the rectangle thereafter
  behaves like a box — dragging it reflows edge-crossing lines and the whole
  rect stays highlighted (even where content doesn't fill it).
- Shift+click entities union into `selectedCells` (`addToSelection`); any
  non-box selection sets `activeBox = null` → plain `moveCells` drag
  (`setSelection`, `select.ts:215-224`).
- `cutSelection` (Delete/Backspace keys, `select.ts:381-398`): erase
  `selectedCells` + `snap` + commit = one undo entry. Empty selection → no-op.
- `cleanup()` on tool switch: drop cells/box/attachments/reshape/move/drag
  state + `clearSelection()` + `clearScratch()` (`select.ts:401-413`).

## 10. Shift-snap summary (status-bar hints)

| Tool | Shift effect | Hint text |
|---|---|---|
| Box / rounded-rect / oval | square / 1:1 circle (anchor-fixed) | `□ square` / `○ circle` |
| Line/arrow | flip bend axis (XOR, §3.2); tip-reshape turns (§9.3) | `⇄ flip axis` |
| Line/arrow + Alt | straight-only, dominant axis (§3.5) | `━ straight` |
| Text Enter | newline instead of commit | `⏎ newline` |
| Select + click | add entity to selection | `＋ add to selection` |

Show the hint in the status bar whenever the modifier is held with the
corresponding tool active (plan §4.2).

## 11. Undo commit points (normative)

**One gesture = at most one undo entry, pushed at `end()` (or Enter/Delete
for text/select-keys). `move()`/`handleKey()`-typing never push.**

| Gesture | Undo entry? |
|---|---|
| Pencil drag (any length, incl. single click) | 1 |
| Box / rounded-rect / oval drag | 1 (empty 1×1 → 0) |
| Line / arrow drag (incl. mid-drag axis flips) | 1 |
| Text session (click … Enter / tool-switch / Esc) | 1 |
| Eraser drag (rect) | 1 |
| Grab | 0 (read-only) |
| Select: box/word/block drag | 1 |
| Select: tip reshape (moved) | 1; unmoved → 0 |
| Select: rubber-band alone (no drag) | 0 (selection only) |
| Select: Delete/Backspace/cut | 1; empty selection → 0 |
| DrawMove line-slide | 1 |
| Esc-cancel anywhere | 0 |

## 12. Edge-case checklist (must be snapshot/unit tested)

1. 1×1 box → empty scratch, no undo entry.
2. 2×1 box row = `──`, 1×2 box column = `││`, no corners; 2×2 = 4 corners.
3. Zero-length line = single `│`; zero-length arrow = `▼`.
4. Line bend table: all 8 orientations (§3.1) + axis-flip via Ctrl.
5. `─` touched mid-span by `│` from above → `┴`… wait: horizontal line met
   from above gains UP → `┴`? UP-connecting junction on a horizontal run is
   `┴` (`connect`: `lineHorizontal + UP → junctionUp = ┴`). Snapshot the
   full `connect`/`disconnect` lattice from `characters.ts:170-322`.
6. Deleting (eraser/cut/move-away) the stem of a `┼` degrades it correctly
   (`┼→┤`-family via unsnap + normalization, `snap.ts:69-151`).
7. Moved box keeps connectors attached (L reflow + bend insertion,
   arrow re-pointing, arrow-into-box riding).
8. Tip reshape only moves the last segment (pivot = first bend preserved).
9. Word drag moves the maximal run, stops at spaces/box chars.
10. Rubber-band selects content cells only; empty rect → empty selection.
11. Even-width circle mirrors exactly (mirror-x == mirror-y == self).
12. Text Backspace at session left edge clamps (does not eat the previous line).
13. Wide char: paint `漢` then paint `x` on its continuation cell → anchor
    cleared, no orphan continuation.
14. Undo ×50 cap: 51 commits → oldest dropped; redo cleared by new commit.

## 13. Port notes (what was deliberately NOT ported)

- `LegacyRenderLayer` (`render_layer.ts:7-257`, the context-sensitive
  re-glyphing renderer) — **thrown away** per plan §2.1. What you store is
  what renders; `snap()` at draw time is the only normalization.
- `cellContext`'s *rendering* role is gone, but its *heuristic* role
  (line-axis inference, §3.2) is kept — reimplement the 8-neighbourhood
  count against the composed view, not against pixels.
- Pixel concepts (`font.ts`, DPR, zoom, pan offsets in `canvas.ts`) — gone.
  Viewport pan is in cells; there is no zoom (plan §3.3).
- `localStorage` persistence + share-URL serialization — replaced by
  `.omaframe.json` files (see `docs/model-api.md`).
