use crate::storage::Storage;
use crate::config::Config;
use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    backend::{Backend, CrosstermBackend},
    layout::{Constraint, Direction, Layout},
    style::{Color, Modifier, Style},
    widgets::{Block, Borders, Cell, Row, Table, TableState, Paragraph},
    Frame, Terminal,
};
use std::io;

pub fn run_dashboard() -> Result<(), Box<dyn std::error::Error>> {
    let db_path = Config::db_path();
    if !db_path.exists() {
        println!("💡 No ShellSense data yet — start typing commands!");
        return Ok(());
    }

    let storage = Storage::open(&db_path)?;
    if storage.total_commands().unwrap_or(0) == 0 {
        println!("💡 No ShellSense data yet — start typing commands!");
        return Ok(());
    }

    // setup terminal
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    // create app and run it
    let res = run_app(&mut terminal, &storage);

    // restore terminal
    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )?;
    terminal.show_cursor()?;

    if let Err(err) = res {
        eprintln!("{:?}", err)
    }

    Ok(())
}

fn run_app<B: Backend>(terminal: &mut Terminal<B>, storage: &Storage) -> Result<(), Box<dyn std::error::Error>> 
where <B as Backend>::Error: 'static {
    let mut table_state = TableState::default();
    table_state.select(Some(0));

    loop {
        // Fetch fresh top commands each frame in case of deletion
        let top_commands = storage.get_top_commands(20).unwrap_or_default();
        if top_commands.is_empty() {
            table_state.select(None);
        } else if let Some(selected) = table_state.selected() {
            if selected >= top_commands.len() {
                table_state.select(Some(top_commands.len() - 1));
            }
        }

        terminal.draw(|f| ui(f, storage, &top_commands, &mut table_state))?;

        if let Event::Key(key) = event::read()? {
            match key.code {
                KeyCode::Char('q') | KeyCode::Esc => return Ok(()),
                KeyCode::Down | KeyCode::Char('j') => {
                    if !top_commands.is_empty() {
                        let i = match table_state.selected() {
                            Some(i) => {
                                if i >= top_commands.len() - 1 { 0 } else { i + 1 }
                            }
                            None => 0,
                        };
                        table_state.select(Some(i));
                    }
                }
                KeyCode::Up | KeyCode::Char('k') => {
                    if !top_commands.is_empty() {
                        let i = match table_state.selected() {
                            Some(i) => {
                                if i == 0 { top_commands.len() - 1 } else { i - 1 }
                            }
                            None => 0,
                        };
                        table_state.select(Some(i));
                    }
                }
                KeyCode::Char('d') | KeyCode::Delete => {
                    if let Some(i) = table_state.selected() {
                        if let Some((cmd, _)) = top_commands.get(i) {
                            let _ = storage.delete_command(cmd);
                        }
                    }
                }
                _ => {}
            }
        }
    }
}

fn ui(f: &mut Frame, storage: &Storage, top_commands: &[(String, u32)], table_state: &mut TableState) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .margin(1)
        .constraints(
            [
                Constraint::Length(3),
                Constraint::Min(0),
                Constraint::Length(2),
            ]
            .as_ref(),
        )
        .split(f.area());

    let (total, unique, db_size) = (
        storage.total_commands().unwrap_or(0),
        storage.unique_commands().unwrap_or(0),
        storage.db_size_bytes().unwrap_or(0) / 1024,
    );

    let header_text = format!(" 🧠 ShellSense Dashboard  |  Total: {}  |  Unique: {}  |  DB Size: {} KB ", total, unique, db_size);
    
    let header = Paragraph::new(header_text)
        .style(Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD))
        .block(Block::default().borders(Borders::ALL).title(" Status "));
    
    f.render_widget(header, chunks[0]);

    let rows = top_commands.iter().enumerate().map(|(i, (cmd, count))| {
        Row::new(vec![
            Cell::from((i + 1).to_string()),
            Cell::from(cmd.clone()),
            Cell::from(count.to_string()),
        ])
    });

    let widths = [Constraint::Length(5), Constraint::Min(50), Constraint::Length(10)];
    let table = Table::new(rows, widths)
        .header(Row::new(vec!["Rank", "Command", "Count"]).style(Style::default().add_modifier(Modifier::BOLD).fg(Color::Yellow)))
        .block(Block::default().borders(Borders::ALL).title(" Top Commands (Scroll to view, 'd' to Delete) "))
        .column_spacing(2)
        .row_highlight_style(Style::default().add_modifier(Modifier::REVERSED))
        .highlight_symbol("> ");

    f.render_stateful_widget(table, chunks[1], table_state);

    let footer = Paragraph::new(" [Up/k] [Down/j] Navigate   |   [d] Delete Command   |   [q] Quit ")
        .style(Style::default().fg(Color::DarkGray));
    f.render_widget(footer, chunks[2]);
}
