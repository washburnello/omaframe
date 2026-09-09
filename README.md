# omaframe

**omaframe** is a native terminal TUI app for sketching TUI wireframes —
with real terminal characters, real ANSI colors, and real terminal
constraints (fixed cell grid, monospace, Nerd Font glyphs).

Omarchy-first. Mouse-friendly. No vim bindings. Clean, simple, intuitive.

> Fork of [lewish/asciiflow](https://github.com/lewish/asciiflow),
> reimagined from a web app as a native terminal wireframing tool.
> See [`plan.md`](./plan.md) for the full vision, architecture, and phased
> build plan.

## Vision

What you draw is what the terminal renders: 1 cell = 1 character. No pixel
canvas, no fake zoom, no proportional fonts on the canvas.

- **Omarchy-first, canvas-exempt.** App chrome follows the live Omarchy
  theme; the canvas shows your wireframe's own ANSI-16 colors, with a
  light/dark preview toggle to check contrast.
- **Paint-program mental model, not CAD.** Foreground pot + background pot,
  pencil, eyedropper, big scrolling palette.
- **Mouse-first.** Click, drag, handles, scroll. Keyboard is shortcuts plus
  a command palette (`Ctrl-K`) — never modal vim bindings.
- **Agent-native.** Every wireframe is a `.oframe` file an agent can
  read, diff, generate, and modify, plus one-shot text export for pasting
  into a chat window.

## Status

**Wave 1: Rust rewrite in progress on this branch** (`omaframe-tui`).

- The native TUI (Rust + Ratatui + crossterm) is being built on this branch.
- The legacy web code in `client/` (Bazel build) is untouched.
- `plan.md` is the source of truth for scope and phases; start there.

## Repo layout

- `plan.md` — vision, architecture, UX design, phased build plan.
- `assets/` — seed data: `nerd.txt` (curated Nerd Font glyph list),
  `widgets.toml` (widget stamp catalog, see `plan.md` §4.4).
- `client/` — legacy asciiflow web app (Bazel). Untouched by the rewrite.

## License

MIT. Upstream license retained as-is — copyright © 2021 Lewis Hemens
(see [`LICENSE`](./LICENSE)). Attribution to the original project:
[lewish/asciiflow](https://github.com/lewish/asciiflow).
