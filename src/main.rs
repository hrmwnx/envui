mod app;
mod storage;
mod ui;

use std::{error::Error, io};

use crossterm::{
    event::{self, Event, KeyCode, KeyEventKind, KeyModifiers},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::backend::CrosstermBackend;

fn main() -> Result<(), Box<dyn Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|arg| arg == "--help" || arg == "-h") {
        if args.len() != 1 {
            return Err("--help cannot be combined with other options".into());
        }
        println!(
            "envui - browse environment variables\n\nUsage: envui [--list] [--dotenv]\n       envui --help\n\
            \nOn Windows, the default view groups persistent User and System variables. Use --dotenv to edit .env."
        );
        return Ok(());
    }
    let list_only = args.iter().any(|arg| arg == "--list");
    let dotenv_mode = args.iter().any(|arg| arg == "--dotenv");
    if args.iter().any(|arg| arg != "--list" && arg != "--dotenv") {
        return Err("Usage: envui [--list] [--dotenv]".into());
    }

    let entries = if dotenv_mode {
        storage::load_dotenv(".env")?
    } else {
        storage::load_env(".env")?
    };
    if list_only {
        println!("SOURCE\tKEY\tVALUE");
        for entry in entries {
            println!("{}\t{}\t{}", entry.source.label(), entry.key, entry.value);
        }
        return Ok(());
    }

    let mut app = app::App::new(entries);
    app.set_system_write_allowed(storage::has_admin_privileges());
    let mut terminal = setup_terminal()?;

    let result = run(&mut terminal, &mut app);
    restore_terminal(&mut terminal)?;
    result?;
    Ok(())
}

fn setup_terminal() -> Result<ratatui::Terminal<CrosstermBackend<io::Stdout>>, Box<dyn Error>> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    Ok(ratatui::Terminal::new(backend)?)
}

fn restore_terminal(
    terminal: &mut ratatui::Terminal<CrosstermBackend<io::Stdout>>,
) -> Result<(), Box<dyn Error>> {
    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;
    Ok(())
}

fn run(
    terminal: &mut ratatui::Terminal<CrosstermBackend<io::Stdout>>,
    app: &mut app::App,
) -> Result<(), Box<dyn Error>> {
    loop {
        terminal.draw(|frame| ui::draw(frame, app))?;
        if let Event::Key(key) = event::read()? {
            if key.kind != KeyEventKind::Press {
                continue;
            }
            let selected_before_key = app.selected;
            app.dismiss_system_access_warning();
            if app.add_variable.is_some() {
                match key.code {
                    KeyCode::Esc => app.cancel_add_variable(),
                    KeyCode::Tab => app.toggle_add_variable_field(),
                    KeyCode::Backspace => app.add_variable_backspace(),
                    KeyCode::Enter => app.commit_add_variable(),
                    KeyCode::Char(character)
                        if !key
                            .modifiers
                            .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
                    {
                        app.add_variable_char(character);
                    }
                    _ => {}
                }
            } else if app.delete_confirmation.is_some() {
                match key.code {
                    KeyCode::Char('y') | KeyCode::Char('Y') | KeyCode::Enter => {
                        app.confirm_delete()
                    }
                    KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => app.cancel_delete(),
                    _ => {}
                }
            } else if app.edit_buffer.is_some() {
                let display_width = (usize::from(terminal.size()?.width) * 80 / 100)
                    .saturating_sub(2)
                    .max(1);
                match key.code {
                    KeyCode::Esc => app.cancel_edit(),
                    KeyCode::Enter if app.editing_list_item => app.commit_edit(),
                    KeyCode::Enter if key.modifiers.contains(KeyModifiers::SHIFT) => {
                        app.edit_char('\n');
                    }
                    KeyCode::Enter => app.commit_edit(),
                    KeyCode::Backspace => app.edit_backspace(),
                    KeyCode::Delete => app.edit_delete(),
                    KeyCode::Left => app.edit_move_left(),
                    KeyCode::Right => app.edit_move_right(),
                    KeyCode::Up => app.edit_move_up(display_width),
                    KeyCode::Down => app.edit_move_down(display_width),
                    KeyCode::Home => app.edit_home(),
                    KeyCode::End => app.edit_end(),
                    KeyCode::Char(character)
                        if !key
                            .modifiers
                            .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
                    {
                        app.edit_char(character);
                    }
                    _ => {}
                }
            } else if app.list_items.is_some() {
                match key.code {
                    KeyCode::Esc => app.cancel_edit(),
                    KeyCode::Enter if key.modifiers.contains(KeyModifiers::CONTROL) => {
                        app.commit_edit()
                    }
                    KeyCode::Enter => app.begin_list_item_edit(),
                    KeyCode::Up if key.modifiers.contains(KeyModifiers::CONTROL) => {
                        app.reorder_list_item(false)
                    }
                    KeyCode::Down if key.modifiers.contains(KeyModifiers::CONTROL) => {
                        app.reorder_list_item(true)
                    }
                    KeyCode::Up => app.move_list_selection(false),
                    KeyCode::Down => app.move_list_selection(true),
                    KeyCode::PageUp => app.page_list_selection(
                        false,
                        (usize::from(terminal.size()?.height) * 75 / 100).saturating_sub(2),
                    ),
                    KeyCode::PageDown => app.page_list_selection(
                        true,
                        (usize::from(terminal.size()?.height) * 75 / 100).saturating_sub(2),
                    ),
                    KeyCode::Char('a') | KeyCode::Char('A') => app.add_list_item(),
                    KeyCode::Char('d') | KeyCode::Char('D') => app.delete_list_item(),
                    _ => {}
                }
            } else {
                match key.code {
                    KeyCode::Char('q') | KeyCode::Esc => break,
                    KeyCode::Down | KeyCode::Char('j') => app.select_next(),
                    KeyCode::Up | KeyCode::Char('k') => app.select_previous(),
                    KeyCode::Char('e') | KeyCode::Char('E') | KeyCode::Enter => app.begin_edit(),
                    KeyCode::Char('a') | KeyCode::Char('A') => app.begin_add_variable(),
                    KeyCode::Char('d') | KeyCode::Char('D') => app.begin_delete_confirmation(),
                    KeyCode::Char('s') | KeyCode::Char('S')
                        if key.modifiers.contains(KeyModifiers::CONTROL)
                            || matches!(key.code, KeyCode::Char('s' | 'S')) =>
                    {
                        match storage::save_changes(".env", &app.original_entries, &app.entries) {
                            Ok(()) => app.mark_saved(),
                            Err(error) => app.status = format!("Save failed: {error}"),
                        }
                    }
                    _ => {}
                }
            }
            if app.selected != selected_before_key {
                app.show_system_access_warning_if_selected();
            }
        }
    }
    Ok(())
}
