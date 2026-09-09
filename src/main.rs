//! omaframe — native terminal TUI wireframing app.
//!
//! `omaframe [FILE]`: opens the file or creates an 80×24 doc bound to that
//! path. Ctrl-S saves, exit autosaves to the same path (prompt-free).

mod app;
mod ui;

use std::io::{self, Write};
use std::path::PathBuf;

use crossterm::{
    event::{
        self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyModifiers,
        MouseButton, MouseEventKind,
    },
    execute,
    terminal::{self, EnterAlternateScreen, LeaveAlternateScreen},
};
use omaframe::model::{Document, load_file};
use omaframe::{clipboard, theme};
use ratatui::{Terminal, backend::CrosstermBackend, layout::Rect};

use app::{App, Tool, palette_chars};
use ui::LayoutAreas;

fn doc_name_for(path: &std::path::Path) -> String {
    path.file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("untitled")
        .to_string()
}

fn open_or_create(path: Option<PathBuf>) -> (Document, Option<PathBuf>) {
    match path {
        Some(p) => {
            if p.exists() {
                match load_file(&p) {
                    Ok(doc) => (doc, Some(p)),
                    Err(e) => {
                        eprintln!("omaframe: cannot load {}: {e}", p.display());
                        std::process::exit(1);
                    }
                }
            } else {
                let doc = Document::new(doc_name_for(&p), 80, 24);
                (doc, Some(p))
            }
        }
        None => (Document::new("untitled", 80, 24), None),
    }
}

fn restore_terminal() {
    let _ = terminal::disable_raw_mode();
    let mut out = io::stdout();
    let _ = execute!(out, LeaveAlternateScreen, DisableMouseCapture);
    let _ = out.flush();
}

fn install_panic_hook() {
    let prev = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        restore_terminal();
        prev(info);
    }));
}

/// Current terminal area for mouse mapping (updated on resize).
fn term_area(w: u16, h: u16) -> Rect {
    Rect::new(0, 0, w, h)
}

fn handle_canvas_press(app: &mut App, doc_pos: (i32, i32), shift: bool) {
    app.start_stroke(doc_pos.0, doc_pos.1, shift);
}

fn handle_key(app: &mut App, code: KeyCode, mods: KeyModifiers) -> bool {
    let ctrl = mods.contains(KeyModifiers::CONTROL);
    let alt = mods.contains(KeyModifiers::ALT);
    let shift = mods.contains(KeyModifiers::SHIFT);

    // --- Ctrl combos (never typed as text) ---
    if ctrl {
        match code {
            KeyCode::Char('s') | KeyCode::Char('S') => {
                app.save();
                return true;
            }
            KeyCode::Char('z') | KeyCode::Char('Z') => {
                if shift {
                    app.redo();
                } else {
                    app.undo();
                }
                return true;
            }
            KeyCode::Char('y') | KeyCode::Char('Y') => {
                app.redo();
                return true;
            }
            KeyCode::Char('k') | KeyCode::Char('K') => {
                app.set_status("command palette: coming in Phase 6");
                return true;
            }
            KeyCode::Char('q') | KeyCode::Char('Q') | KeyCode::Char('c') | KeyCode::Char('C') => {
                app.autosave();
                app.should_quit = true;
                return true;
            }
            KeyCode::Char('p') | KeyCode::Char('P') => {
                app.toggle_preview();
                return true;
            }
            KeyCode::Char('f') | KeyCode::Char('F') => {
                app.cycle_fg();
                return true;
            }
            KeyCode::Char('b') | KeyCode::Char('B') => {
                app.cycle_bg();
                return true;
            }
            KeyCode::Char('e') | KeyCode::Char('E') => {
                export_clipboard(app);
                return true;
            }
            KeyCode::Char('h') | KeyCode::Char('H') => {
                let idx = app.doc.active;
                app.toggle_layer_visible(idx);
                return true;
            }
            _ => return false,
        }
    }

    // --- Inline path prompt (menu Load / Save As): captures all typing ---
    if app.prompt.is_some() {
        match code {
            KeyCode::Esc => app.cancel_prompt(),
            KeyCode::Enter => app.confirm_prompt(),
            KeyCode::Backspace => app.prompt_backspace(),
            KeyCode::Char(c) => {
                if !ctrl && !alt {
                    app.prompt_push(c);
                }
            }
            _ => {}
        }
        return true;
    }

    // --- Alt combos: tool switching + pots (never typed) ---
    if alt {
        if let KeyCode::Char(c) = code {
            let lc = c.to_ascii_lowercase();
            if let Some(t) = Tool::from_shortcut(lc) {
                app.set_tool(t);
                return true;
            }
            match lc {
                'f' => {
                    app.cycle_fg();
                    return true;
                }
                'c' => {
                    app.cycle_bg();
                    return true;
                }
                _ => return false,
            }
        }
        return false;
    }

    // --- Non-Ctrl/Alt keys ---
    match code {
        KeyCode::Esc => {
            app.cancel_stroke();
            true
        }
        KeyCode::Enter => {
            if app.tool == Tool::Text && app.is_text_session() {
                if shift {
                    app.text_newline();
                } else {
                    app.commit_text();
                }
                true
            } else {
                false
            }
        }
        KeyCode::Tab => {
            app.cycle_layer(1);
            true
        }
        KeyCode::BackTab => {
            app.cycle_layer(-1);
            true
        }
        KeyCode::Left => {
            app.move_cursor(-1, 0);
            true
        }
        KeyCode::Right => {
            app.move_cursor(1, 0);
            true
        }
        KeyCode::Up => {
            app.move_cursor(0, -1);
            true
        }
        KeyCode::Down => {
            app.move_cursor(0, 1);
            true
        }
        KeyCode::Delete => {
            if app.tool == Tool::Text && app.is_text_session() {
                app.text_delete();
            } else {
                app.delete_at_cursor();
            }
            true
        }
        KeyCode::Backspace => {
            if app.tool == Tool::Text && app.is_text_session() {
                app.text_backspace();
            } else {
                app.delete_at_cursor();
            }
            true
        }
        KeyCode::PageUp => {
            app.cycle_palette_tab(-1);
            true
        }
        KeyCode::PageDown => {
            app.cycle_palette_tab(1);
            true
        }
        KeyCode::Char(c) => {
            // Text tool types; pencil captures chars as activeCh
            // (palette-spec §4); other tools use single-letter shortcuts.
            if app.tool == Tool::Text {
                app.type_char(c);
                return true;
            }
            if app.tool == Tool::Pencil {
                // Uppercase L/A are commands even in pencil (style/arrow);
                // `_`/space switch to Pan; everything else becomes the
                // pencil char.
                if c == 'L' {
                    app.cycle_box_style();
                    return true;
                }
                if c == 'A' {
                    app.toggle_arrow();
                    return true;
                }
                if c == '[' {
                    app.cycle_palette_tab(-1);
                    return true;
                }
                if c == ']' {
                    app.cycle_palette_tab(1);
                    return true;
                }
                if c == '_' || c == ' ' {
                    app.set_tool(Tool::Pan);
                    return true;
                }
                app.pick_palette_char(&c.to_string());
                return true;
            }
            // Other tools: single-letter shortcuts (`_`/space pan).
            if c == '_' || c == ' ' {
                app.set_tool(Tool::Pan);
                return true;
            }
            // Other tools: single-letter shortcuts.
            if let Some(t) = Tool::from_shortcut(c) {
                // 'L' (shift+l) cycles style instead of reselecting line.
                if c == 'L' {
                    app.cycle_box_style();
                } else {
                    app.set_tool(t);
                }
                return true;
            }
            match c {
                'L' => {
                    app.cycle_box_style();
                    true
                }
                'A' => {
                    app.toggle_arrow();
                    true
                }
                '[' => {
                    app.cycle_palette_tab(-1);
                    true
                }
                ']' => {
                    app.cycle_palette_tab(1);
                    true
                }
                ' ' => false,
                _ => false,
            }
        }
        _ => false,
    }
}

fn export_clipboard(app: &mut App) {
    let txt = match app.history.selection() {
        Some(r) => omaframe::model::export_selection(&app.doc, &r),
        None => omaframe::model::export_txt(&app.doc),
    };
    match clipboard::copy_text(&txt) {
        Ok(msg) => app.set_status(msg),
        Err(msg) => app.set_status(msg),
    }
}

fn handle_mouse(
    app: &mut App,
    areas: &LayoutAreas,
    kind: MouseEventKind,
    col: u16,
    row: u16,
    mods: KeyModifiers,
) {
    let shift = mods.contains(KeyModifiers::SHIFT);
    match kind {
        MouseEventKind::Down(MouseButton::Left) => {
            if let Some(action) = ui::hit_menu(areas, col, row) {
                match action {
                    ui::MenuAction::New => app.new_file(),
                    ui::MenuAction::Save => app.menu_save(),
                    ui::MenuAction::Load => app.start_prompt(app::PromptKind::Load),
                }
                return;
            }
            if let Some(row_idx) = ui::hit_colors(areas, app, col, row) {
                // Row 0 is transparent-background; rows 1–16 are ANSI 0–15.
                if row_idx == 0 {
                    app.set_status("transparent applies to background (right-click)");
                } else {
                    app.set_fg(row_idx as i8 - 1);
                }
                return;
            }
            if let Some(t) = ui::hit_tool(areas, col, row) {
                app.set_tool(t);
                return;
            }
            if let Some(tab) = ui::hit_palette_tab(areas, col, row) {
                app.set_palette_tab(tab);
                app.set_status(format!("palette: {}", app::PALETTE_TABS[tab]));
                return;
            }
            if let Some(dir) = ui::hit_palette_scroll(areas, app, col, row) {
                // Scroll arrows page by nearly a full grid height.
                let gh = areas.palette_grid.height as usize;
                let page = gh.saturating_sub(1).max(1) as i32;
                let cols = ui::palette_grid_cols(areas, app.palette_tab);
                app.scroll_palette(dir * page, cols, gh);
                return;
            }
            if let Some(idx) = ui::hit_palette_grid(areas, app, col, row) {
                let items = palette_chars(app.palette_tab);
                if app.palette_tab == 6 {
                    app.set_status(format!(
                        "widget '{}': coming in Phase 3",
                        items.get(idx).map(String::as_str).unwrap_or("?")
                    ));
                } else if let Some(s) = items.get(idx) {
                    app.pick_palette_char(s);
                }
                return;
            }
            if let Some((idx, eye)) = ui::hit_layer_row(areas, app.doc.layers.len(), col, row) {
                if eye {
                    app.toggle_layer_visible(idx);
                } else {
                    app.set_active_layer(idx);
                }
                return;
            }
            if let Some(pos) = ui::hit_canvas(areas, app, col, row) {
                if app.tool == Tool::Pan {
                    app.start_pan(col, row);
                } else {
                    handle_canvas_press(app, pos, shift);
                }
            }
        }
        MouseEventKind::Drag(MouseButton::Left) => {
            if app.tool == Tool::Pan {
                // Pan drags work anywhere, not just over the canvas.
                app.update_pan(col, row);
                return;
            }
            if let Some(pos) = ui::hit_canvas(areas, app, col, row) {
                app.update_stroke(pos.0, pos.1, shift);
            }
        }
        MouseEventKind::Up(MouseButton::Left) => {
            app.end_pan();
            app.end_stroke();
        }
        MouseEventKind::Down(MouseButton::Right) => {
            if let Some(row_idx) = ui::hit_colors(areas, app, col, row) {
                // Right-click sets the background pot (row 0 = transparent).
                app.set_bg(row_idx as i8 - 1);
                return;
            }
            if let Some(pos) = ui::hit_canvas(areas, app, col, row) {
                // decisions.md Q4: right-click = grab into pencil.
                app.grab_at(pos.0, pos.1);
            } else if let Some(idx) = ui::hit_palette_grid(areas, app, col, row) {
                // Right-click a palette char: textured-panel accent hint.
                let items = palette_chars(app.palette_tab);
                if let Some(s) = items.get(idx) {
                    app.set_status(format!("bg-accent '{s}': use grab (right-click canvas) then paint"));
                }
            }
        }
        MouseEventKind::Down(MouseButton::Middle) => {
            // Middle-drag pans the infinite canvas (anywhere, not just canvas).
            app.start_pan(col, row);
        }
        MouseEventKind::Drag(MouseButton::Middle) => {
            app.update_pan(col, row);
        }
        MouseEventKind::Up(MouseButton::Middle) => {
            app.end_pan();
        }
        MouseEventKind::ScrollDown => {
            if ui::over_palette_grid(areas, col, row) {
                app.scroll_palette(
                    1,
                    ui::palette_grid_cols(areas, app.palette_tab),
                    areas.palette_grid.height as usize,
                );
            } else if ui::over_colors(areas, col, row) {
                app.scroll_colors(1, areas.colors.height as usize);
            } else if ui::over_canvas(areas, col, row) {
                // Infinite scroll: wheel pans (Shift+wheel goes horizontal).
                if shift {
                    app.viewport.0 += 3;
                } else {
                    app.viewport.1 += 3;
                }
            }
        }
        MouseEventKind::ScrollUp => {
            if ui::over_palette_grid(areas, col, row) {
                app.scroll_palette(
                    -1,
                    ui::palette_grid_cols(areas, app.palette_tab),
                    areas.palette_grid.height as usize,
                );
            } else if ui::over_colors(areas, col, row) {
                app.scroll_colors(-1, areas.colors.height as usize);
            } else if ui::over_canvas(areas, col, row) {
                if shift {
                    app.viewport.0 -= 3;
                } else {
                    app.viewport.1 -= 3;
                }
            }
        }
        MouseEventKind::Moved => {
            // Hover moves the cursor (ghost position) without drawing.
            if let Some(pos) = ui::hit_canvas(areas, app, col, row) {
                let drawing = app_draw_active(app);
                if !drawing {
                    app.cursor = pos;
                    app.clamp_cursor();
                }
            }
        }
        _ => {}
    }
}

fn app_draw_active(app: &App) -> bool {
    // `drawing` is private; infer from scratch non-emptiness is wrong for
    // 1×1 box previews, so track via cursor-follows-mouse suppression only
    // when a text session is open or scratch has content. Conservative: never
    // suppress hover unless scratch is non-empty.
    !app.scratch.is_empty() || app.is_text_session()
}

fn real_main() -> Result<(), Box<dyn std::error::Error>> {
    use std::io::IsTerminal as _;
    if !io::stdout().is_terminal() {
        return Err("needs a terminal to run (try `omaframe-export` for headless export)".into());
    }
    let arg_path = std::env::args().nth(1).map(PathBuf::from);
    let (doc, file_path) = open_or_create(arg_path);
    let mut app = App::new(doc, file_path);
    let theme = theme::load();

    terminal::enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let (mut tw, mut th) = terminal::size().unwrap_or((80, 24));

    loop {
        terminal.draw(|f| ui::render(f, &mut app, &theme))?;
        // Keep the cursor visible after keyboard moves.
        {
            let areas = ui::compute_layout(term_area(tw, th), app.doc.layers.len());
            app.ensure_cursor_visible(areas.canvas.width as i32, areas.canvas.height as i32);
        }
        if app.should_quit {
            break;
        }
        let ev = event::read()?;
        match ev {
            Event::Key(k) => {
                handle_key(&mut app, k.code, k.modifiers);
            }
            Event::Mouse(m) => {
                let areas = ui::compute_layout(term_area(tw, th), app.doc.layers.len());
                handle_mouse(&mut app, &areas, m.kind, m.column, m.row, m.modifiers);
            }
            Event::Resize(w, h) => {
                tw = w;
                th = h;
            }
            _ => {}
        }
    }

    app.autosave();
    restore_terminal();
    Ok(())
}

fn main() {
    install_panic_hook();
    if let Err(e) = real_main() {
        restore_terminal();
        eprintln!("omaframe: {e}");
        std::process::exit(1);
    }
}
