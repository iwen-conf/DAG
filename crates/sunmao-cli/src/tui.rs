//! Minimal ratatui screens for config base_url and project list (D-19).

use std::io::{self, stdout};

use anyhow::{Context, Result};
use crossterm::event::{self, Event, KeyCode, KeyEventKind};
use crossterm::terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen};
use crossterm::ExecutableCommand;
use ratatui::layout::{Constraint, Direction, Layout};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, ListState, Paragraph};
use ratatui::{Frame, Terminal};
use ratatui::backend::CrosstermBackend;

use crate::config::{save_global, GlobalConfig};

pub fn run_config_tui(initial: &str) -> Result<String> {
    let mut value = initial.to_string();
    let mut cursor = value.len();
    with_terminal(|term| {
        loop {
            term.draw(|f| draw_config(f, &value, cursor))?;
            if let Event::Key(key) = event::read()? {
                if key.kind != KeyEventKind::Press {
                    continue;
                }
                match key.code {
                    KeyCode::Esc => return Ok(initial.to_string()),
                    KeyCode::Enter => return Ok(value.clone()),
                    KeyCode::Backspace => {
                        if cursor > 0 {
                            value.remove(cursor - 1);
                            cursor -= 1;
                        }
                    }
                    KeyCode::Left => cursor = cursor.saturating_sub(1),
                    KeyCode::Right => {
                        if cursor < value.len() {
                            cursor += 1;
                        }
                    }
                    KeyCode::Char(c) => {
                        value.insert(cursor, c);
                        cursor += 1;
                    }
                    _ => {}
                }
            }
        }
    })
}

fn draw_config(f: &mut Frame, value: &str, cursor: usize) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(3), Constraint::Min(3), Constraint::Length(2)])
        .split(f.area());
    f.render_widget(
        Paragraph::new("sunmao config — set base_url (Enter save, Esc cancel)")
            .block(Block::default().borders(Borders::ALL).title("config")),
        chunks[0],
    );
    let mut display = value.to_string();
    if cursor <= display.len() {
        display.insert(cursor, '▌');
    }
    f.render_widget(
        Paragraph::new(display).block(Block::default().borders(Borders::ALL).title("base_url")),
        chunks[1],
    );
    f.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("Enter", Style::default().add_modifier(Modifier::BOLD)),
            Span::raw(" save  "),
            Span::styled("Esc", Style::default().add_modifier(Modifier::BOLD)),
            Span::raw(" cancel"),
        ])),
        chunks[2],
    );
}

#[derive(Clone)]
pub struct ProjectRow {
    pub id: String,
    pub name: String,
    pub repo_path: String,
}

/// Interactive project picker. Returns selected project id.
pub fn run_projects_tui(projects: &[ProjectRow]) -> Result<Option<String>> {
    if projects.is_empty() {
        return Ok(None);
    }
    let mut state = ListState::default();
    state.select(Some(0));
    with_terminal(|term| {
        loop {
            term.draw(|f| draw_projects(f, projects, &mut state))?;
            if let Event::Key(key) = event::read()? {
                if key.kind != KeyEventKind::Press {
                    continue;
                }
                match key.code {
                    KeyCode::Esc | KeyCode::Char('q') => return Ok(None),
                    KeyCode::Enter => {
                        if let Some(i) = state.selected() {
                            return Ok(Some(projects[i].id.clone()));
                        }
                    }
                    KeyCode::Down | KeyCode::Char('j') => {
                        let i = state.selected().unwrap_or(0);
                        let next = (i + 1).min(projects.len() - 1);
                        state.select(Some(next));
                    }
                    KeyCode::Up | KeyCode::Char('k') => {
                        let i = state.selected().unwrap_or(0);
                        state.select(Some(i.saturating_sub(1)));
                    }
                    _ => {}
                }
            }
        }
    })
}

fn draw_projects(f: &mut Frame, projects: &[ProjectRow], state: &mut ListState) {
    let items: Vec<ListItem> = projects
        .iter()
        .map(|p| {
            ListItem::new(Line::from(vec![
                Span::styled(format!("{:<24}", p.name), Style::default().fg(Color::Cyan)),
                Span::raw("  "),
                Span::raw(&p.id),
                Span::raw("  "),
                Span::styled(&p.repo_path, Style::default().fg(Color::DarkGray)),
            ]))
        })
        .collect();
    let list = List::new(items)
        .block(Block::default().borders(Borders::ALL).title("sunmao projects (↑↓ Enter · q quit)"))
        .highlight_style(
            Style::default()
                .bg(Color::DarkGray)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol("› ");
    f.render_stateful_widget(list, f.area(), state);
}

fn with_terminal<T>(f: impl FnOnce(&mut Terminal<CrosstermBackend<io::Stdout>>) -> Result<T>) -> Result<T> {
    enable_raw_mode().context("raw mode")?;
    stdout().execute(EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout());
    let mut terminal = Terminal::new(backend)?;
    let res = f(&mut terminal);
    disable_raw_mode()?;
    stdout().execute(LeaveAlternateScreen)?;
    res
}

/// Save config from TUI and return chosen url.
pub fn config_interactive(current: &str) -> Result<String> {
    let url = run_config_tui(current)?;
    save_global(&GlobalConfig {
        base_url: Some(url.clone()),
    })?;
    Ok(url)
}
