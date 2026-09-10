//! omaframe — native terminal TUI wireframing app.
//!
//! `omaframe [FILE]`: opens the file or creates an 80×24 doc bound to that
//! path. Ctrl-S saves (SaveAs dialog when pathless), exit autosaves to the
//! same path. File picking is modal: menu New/Load open a file dialog
//! (`app.file_dialog`), and while it is open all keys/mouse go to the
//! dialog delegates.

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
use omaframe::model::{Document, PaintColor, load_file};
use omaframe::{clipboard, theme};
use ratatui::{Terminal, backend::CrosstermBackend, layout::Rect};

use app::{App, DialogMode, Tool};
use ui::LayoutAreas;

fn doc_name_for(path: &std::path::Path) -> String {
    let file_name = path
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("");
    let stripped = file_name
        .strip_suffix(".oframe")
        .unwrap_or(file_name);
    if stripped.is_empty() {
        "untitled".to_string()
    } else {
        stripped.to_string()
    }
}

fn open_or_create(path: Option<PathBuf>) -> (Document, Option<PathBuf>) {
    // Tilde-expand CLI paths (`omaframe ~/x.oframe` just works).
    let path = path.map(|p| App::expand_path(&p.to_string_lossy()));
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

/// True when the open dialog is an Open picker (vs a save picker).
fn dialog_is_open_mode(app: &App) -> bool {
    app.file_dialog
        .as_ref()
        .is_some_and(|d| d.mode == DialogMode::Open)
}

/// True when the dialog highlight sits on a file (not a directory).
fn dialog_selected_is_file(app: &App) -> bool {
    app.file_dialog.as_ref().is_some_and(|d| {
        d.selected
            .and_then(|i| d.entries.get(i))
            .is_some_and(|e| !e.is_dir)
    })
}

/// True when the dialog highlight sits on a directory.
fn dialog_selected_is_dir(app: &App) -> bool {
    app.file_dialog.as_ref().is_some_and(|d| {
        d.selected
            .and_then(|i| d.entries.get(i))
            .is_some_and(|e| e.is_dir)
    })
}

/// Swatch color for a [`theme::ColorRow::Entry`] address (None when the
/// address is stale — rows and groups are built from the same theme, so
/// this only fires across a theme reload mid-click).
fn color_at(theme: &theme::Theme, group: usize, index: usize) -> Option<PaintColor> {
    theme::color_groups(theme)
        .get(group)?
        .entries
        .get(index)
        .map(|e| e.color.clone())
}

fn handle_key(app: &mut App, code: KeyCode, mods: KeyModifiers) -> bool {
    let ctrl = mods.contains(KeyModifiers::CONTROL);
    let alt = mods.contains(KeyModifiers::ALT);
    let shift = mods.contains(KeyModifiers::SHIFT);

    // --- Modal file dialog: captures all keys ---
    if app.file_dialog.is_some() {
        if alt && code == KeyCode::Up {
            app.dialog_up();
            return true;
        }
        if ctrl {
            match code {
                KeyCode::Char('q') | KeyCode::Char('Q') | KeyCode::Char('c') | KeyCode::Char('C') => {
                    app.dialog_cancel();
                }
                // Leave Ctrl-S on its normal save path (never confirms).
                KeyCode::Char('s') | KeyCode::Char('S') => {
                    app.menu_save();
                }
                // Swallow every other Ctrl combo (no undo/redo/etc. inside
                // the dialog).
                _ => {}
            }
            return true;
        }
        match code {
            KeyCode::Esc => app.dialog_cancel(),
            KeyCode::Enter => {
                // Files confirm (Open pickers load, Save pickers create);
                // Save pickers also confirm on an empty highlight (the
                // filename box is the target). Highlighted folders descend.
                if dialog_selected_is_dir(app) {
                    app.dialog_enter();
                } else {
                    app.dialog_confirm();
                }
            }
            KeyCode::Up => app.dialog_move_selection(-1),
            KeyCode::Down => app.dialog_move_selection(1),
            KeyCode::Backspace => app.dialog_backspace(),
            KeyCode::Char(c) if !alt => {
                // Save pickers type the filename here; Open mode ignores
                // typing (`dialog_type` itself decides).
                app.dialog_type(c);
            }
            _ => {}
        }
        return true;
    }

    // --- Ctrl combos (never typed as text) ---
    if ctrl {
        match code {
            KeyCode::Char('s') | KeyCode::Char('S') => {
                app.menu_save();
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
                app.request_quit();
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
            if app.search_focused {
                app.clear_search();
            } else {
                app.cancel_stroke();
            }
            true
        }
        KeyCode::Enter => {
            if app.search_focused {
                app.unfocus_search();
                return true;
            }
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
            if app.search_focused {
                app.pop_search();
            } else if app.tool == Tool::Text && app.is_text_session() {
                app.text_delete();
            } else {
                app.delete_at_cursor();
            }
            true
        }
        KeyCode::Backspace => {
            if app.search_focused {
                app.pop_search();
            } else if app.tool == Tool::Text && app.is_text_session() {
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
            // Focused palette search captures everything (any tool).
            if app.search_focused {
                app.push_search(c);
                return true;
            }
            // Text tool types; pencil captures chars as activeCh
            // (palette-spec §4); other tools use single-letter shortcuts.
            if app.tool == Tool::Text {
                app.type_char(c);
                return true;
            }
            // `/` focuses palette search (pencil keeps `/` only via click).
            if c == '/' {
                app.focus_search();
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

#[allow(clippy::too_many_arguments)]
fn handle_mouse(
    app: &mut App,
    areas: &LayoutAreas,
    term: Rect,
    theme: &theme::Theme,
    kind: MouseEventKind,
    col: u16,
    row: u16,
    mods: KeyModifiers,
) {
    let shift = mods.contains(KeyModifiers::SHIFT);

    // --- Modal file dialog: ONLY dialog hits, everything else ignored ---
    if app.file_dialog.is_some() {
        if kind == MouseEventKind::Down(MouseButton::Left) {
            // `dialog_layout` is None on tiny terminals: no dialog targets
            // exist, so the click is ignored like any other non-dialog one.
            if let Some(dlg_layout) = ui::dialog_layout(term) {
                if let Some(btn) = ui::hit_dialog_button(&dlg_layout, col, row) {
                    match btn {
                        ui::DialogButton::Cancel => app.dialog_cancel(),
                        ui::DialogButton::Folder => app.dialog_make_folder(),
                        ui::DialogButton::Save => app.dialog_confirm(),
                    }
                    return;
                }
                // Sidebar/list hits borrow the dialog immutably; the
                // borrows end here so the delegates below may mutate.
                let (side_hit, list_hit) = match app.file_dialog.as_ref() {
                    Some(d) => (
                        ui::hit_dialog_sidebar(&dlg_layout, d, col, row),
                        ui::hit_dialog_list(&dlg_layout, d, col, row),
                    ),
                    None => (None, None),
                };
                if let Some(side) = side_hit {
                    app.dialog_goto_sidebar(side);
                    return;
                }
                if let Some(idx) = list_hit {
                    let already =
                        app.file_dialog.as_ref().and_then(|d| d.selected) == Some(idx);
                    if already {
                        // Re-click: files confirm (double-click convention).
                        // Save pickers adopt the clicked name first since
                        // confirm reads the filename box; the overwrite
                        // guard still applies. Folders descend.
                        if dialog_selected_is_file(app) {
                            if !dialog_is_open_mode(app) {
                                if let Some(d) = app.file_dialog.as_mut() {
                                    if let Some(name) = d
                                        .selected
                                        .and_then(|s| d.entries.get(s))
                                        .map(|e| e.name.clone())
                                    {
                                        d.filename = name;
                                    }
                                }
                            }
                            app.dialog_confirm();
                        } else {
                            app.dialog_enter();
                        }
                    } else if let Some(d) = app.file_dialog.as_mut() {
                        // `FileDialog.selected` is pub: set the highlight
                        // directly instead of stepping through the list.
                        d.selected = Some(idx);
                    }
                }
            }
        }
        return;
    }

    match kind {
        MouseEventKind::Down(MouseButton::Left) => {
            if let Some(action) = ui::hit_menu(areas, col, row) {
                match action {
                    ui::MenuAction::New => app.new_file_request(),
                    ui::MenuAction::Save => app.menu_save(),
                    ui::MenuAction::SaveAs => app.open_save_as_dialog(),
                    ui::MenuAction::Load => app.open_load_dialog(),
                }
                return;
            }
            if let Some(hit) = ui::hit_colors_bar(areas, app, col, row) {
                let total = ui::colors_total_rows();
                let visible = areas.colors.height as usize;
                match hit {
                    ui::ColorBarHit::Up => app.scroll_colors(-1, total, visible),
                    ui::ColorBarHit::Down => app.scroll_colors(1, total, visible),
                    ui::ColorBarHit::Jump(tr) => {
                        app.colors_scroll =
                            ui::colors_bar_target(total, visible, tr);
                    }
                }
                return;
            }
            if let Some(row_idx) = ui::hit_colors(areas, app, col, row) {
                match theme::color_rows(theme).get(row_idx) {
                    Some(theme::ColorRow::Header(name)) => {
                        app.set_status(format!(
                            "{name}: left-click sets fg, right-click sets bg"
                        ));
                    }
                    Some(theme::ColorRow::Transparent) => {
                        app.set_status("transparent applies to background (right-click)");
                    }
                    Some(theme::ColorRow::Entry { group, index }) => {
                        if let Some(color) = color_at(theme, *group, *index) {
                            app.set_fg(color);
                        }
                    }
                    None => {}
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
            if let Some(dir) = ui::hit_glyph_category(areas, app, col, row) {
                app.step_glyph_category(dir);
                return;
            }
            if ui::hit_search_field(areas, col, row) {
                app.focus_search();
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
                let items = app.palette_visible();
                if app.palette_tab == 6 {
                    if let Some(kind) = items.get(idx) {
                        app.arm_widget(kind);
                    }
                    return;
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
                } else if app.pending_widget.is_some() {
                    // Armed widget stamp bypasses the active tool.
                    app.start_stamp(pos.0, pos.1, shift);
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
                // Right-click sets the background pot (Transparent = none).
                match theme::color_rows(theme).get(row_idx) {
                    Some(theme::ColorRow::Transparent) => app.set_bg(None),
                    Some(theme::ColorRow::Entry { group, index }) => {
                        if let Some(color) = color_at(theme, *group, *index) {
                            app.set_bg(Some(color));
                        }
                    }
                    Some(theme::ColorRow::Header(_)) | None => {}
                }
                return;
            }
            if let Some(pos) = ui::hit_canvas(areas, app, col, row) {
                // decisions.md Q4: right-click = grab into pencil.
                app.grab_at(pos.0, pos.1);
            } else if let Some(idx) = ui::hit_palette_grid(areas, app, col, row) {
                // Right-click a palette char: textured-panel accent hint.
                let items = app.palette_visible();
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
                let total = theme::color_rows(theme).len();
                app.scroll_colors(1, total, areas.colors.height as usize);
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
                let total = theme::color_rows(theme).len();
                app.scroll_colors(-1, total, areas.colors.height as usize);
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
    // Main owns the theme instance for the whole loop: canvas styles and
    // the Colors-panel rows/groups all resolve against it. The watcher
    // reloads it live when the Omarchy theme changes mid-session.
    let mut theme = theme::load();
    let mut watch = theme::ThemeWatch::new();

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
            let areas = ui::compute_layout(term_area(tw, th), app.doc.layers.len(), app.palette_tab);
            app.ensure_cursor_visible(areas.canvas.width as i32, areas.canvas.height as i32);
        }
        if app.should_quit {
            break;
        }
        // Poll (don't block forever) so the theme watcher gets a timeslice
        // even with no input. 250ms is imperceptible and ~free.
        if event::poll(std::time::Duration::from_millis(250))? {
            let ev = event::read()?;
            match ev {
                Event::Key(k) => {
                    handle_key(&mut app, k.code, k.modifiers);
                }
                Event::Mouse(m) => {
                    let term = term_area(tw, th);
                    let areas = ui::compute_layout(term, app.doc.layers.len(), app.palette_tab);
                    handle_mouse(&mut app, &areas, term, &theme, m.kind, m.column, m.row, m.modifiers);
                }
                Event::Resize(w, h) => {
                    tw = w;
                    th = h;
                }
                _ => {}
            }
        }
        // Live theme reload: re-tints chrome, ANSI slots, and panel
        // swatches. Frozen RGB cells are untouched by design.
        if let Some(path) = watch.check() {
            theme = theme::load();
            app.set_status(format!("theme: {}", theme::theme_name(&path)));
        }
    }

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

#[cfg(test)]
mod tests {
    use super::*;

    fn isolated_root(tag: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "omaframe-enter-{tag}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        root
    }

    /// Regression: Enter in a Save dialog must confirm (bind + save).
    /// Previously Enter only descended into highlighted folders, so
    /// keyboard users could never create a file.
    #[test]
    fn enter_in_save_dialog_confirms() {
        let root = isolated_root("save");
        let mut app = App::new(Document::new("t", 80, 24), None);
        app.open_save_as_dialog();
        app.file_dialog.as_mut().unwrap().goto(root.clone());
        app.file_dialog.as_mut().unwrap().filename.clear();
        for c in "n.oframe".chars() {
            app.dialog_type(c);
        }
        assert!(app.file_dialog.as_ref().unwrap().confirm_path().is_some());
        handle_key(&mut app, KeyCode::Enter, KeyModifiers::empty());
        assert!(app.file_dialog.is_none(), "Enter confirms SaveAs");
        let path = app.file_path.clone().expect("bound");
        assert_eq!(path, root.join("n.oframe"));
        assert!(path.exists(), "file created");
        let _ = std::fs::remove_dir_all(&root);
    }

    /// Enter on a highlighted folder still descends instead of confirming.
    #[test]
    fn enter_on_highlighted_folder_descends() {
        let root = isolated_root("descend");
        std::fs::create_dir_all(root.join("sub")).unwrap();
        let mut app = App::new(Document::new("t", 80, 24), None);
        app.open_save_as_dialog();
        app.file_dialog.as_mut().unwrap().goto(root.clone());
        app.dialog_move_selection(1); // first row: the subdir
        assert!(dialog_selected_is_dir(&app));
        handle_key(&mut app, KeyCode::Enter, KeyModifiers::empty());
        assert!(app.file_dialog.is_some(), "dialog stays open");
        assert_eq!(
            app.file_dialog.as_ref().unwrap().cwd,
            root.join("sub")
        );
        let _ = std::fs::remove_dir_all(&root);
    }
}
