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

## Wave 2 QA result

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
