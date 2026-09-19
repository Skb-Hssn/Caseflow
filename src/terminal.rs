use crate::error::{AppError, AppResult};
use crossterm::cursor::{Hide, MoveTo, MoveToColumn, Show};
use crossterm::event::{
    self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEventKind, MouseButton,
    MouseEventKind,
};
use crossterm::style::{Attribute, Print, SetAttribute};
use crossterm::terminal::{self, Clear, ClearType};
use crossterm::{execute, queue};
use std::io::{self, IsTerminal, Write};
use std::path::PathBuf;

struct TerminalGuard;

impl TerminalGuard {
    fn enter(mouse: bool) -> AppResult<Self> {
        terminal::enable_raw_mode()
            .map_err(|error| AppError::new(format!("cannot enter terminal raw mode: {error}")))?;
        let mut stderr = io::stderr();
        if mouse {
            execute!(stderr, EnableMouseCapture, Hide)?;
        } else {
            execute!(stderr, Hide)?;
        }
        Ok(Self)
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        restore_terminal();
    }
}

pub fn restore_terminal() {
    let mut stderr = io::stderr();
    let _ = execute!(
        stderr,
        DisableMouseCapture,
        Show,
        SetAttribute(Attribute::Reset)
    );
    let _ = terminal::disable_raw_mode();
}

pub fn select_source(sources: &[PathBuf], mouse: bool) -> AppResult<Option<PathBuf>> {
    if sources.is_empty() {
        return Ok(None);
    }
    if !io::stdin().is_terminal() || !io::stderr().is_terminal() {
        return Ok(sources.first().cloned());
    }
    let _guard = TerminalGuard::enter(mouse)?;
    let (_, start_row) = crossterm::cursor::position().unwrap_or((0, 0));
    let mut selected = 0_usize;
    let mut button = 0_usize;
    loop {
        render_picker(sources, selected, button, start_row)?;
        match event::read()? {
            Event::Key(key) if key.kind == KeyEventKind::Press => match key.code {
                KeyCode::Up => selected = selected.saturating_sub(1),
                KeyCode::Down => selected = (selected + 1).min(sources.len() - 1),
                KeyCode::Tab | KeyCode::Right | KeyCode::Left => button = 1 - button,
                KeyCode::Enter | KeyCode::Char(' ') => {
                    clear_picker(sources.len(), start_row)?;
                    return Ok((button == 0).then(|| sources[selected].clone()));
                }
                KeyCode::Char('s') => {
                    clear_picker(sources.len(), start_row)?;
                    return Ok(Some(sources[selected].clone()));
                }
                KeyCode::Esc | KeyCode::Char('q') => {
                    clear_picker(sources.len(), start_row)?;
                    return Ok(None);
                }
                _ => {}
            },
            Event::Mouse(mouse_event) if mouse => {
                if let MouseEventKind::Down(MouseButton::Left) = mouse_event.kind {
                    let row = mouse_event.row.saturating_sub(start_row) as usize;
                    if row >= 1 && row <= sources.len() {
                        selected = row - 1;
                    } else if row == sources.len() + 2 {
                        if mouse_event.column < 12 {
                            clear_picker(sources.len(), start_row)?;
                            return Ok(Some(sources[selected].clone()));
                        }
                        if (12..26).contains(&mouse_event.column) {
                            clear_picker(sources.len(), start_row)?;
                            return Ok(None);
                        }
                    }
                }
            }
            Event::Resize(_, _) => {}
            _ => {}
        }
    }
}

pub fn confirm(prompt: &str, action: &str, mouse: bool) -> AppResult<bool> {
    if !io::stdin().is_terminal() || !io::stderr().is_terminal() {
        return Ok(false);
    }
    let _guard = TerminalGuard::enter(mouse)?;
    let (_, row) = crossterm::cursor::position().unwrap_or((0, 0));
    let mut selected = 1_usize;
    loop {
        let mut stderr = io::stderr();
        queue!(
            stderr,
            MoveTo(0, row),
            Clear(ClearType::CurrentLine),
            Print(prompt),
            Print("  ")
        )?;
        draw_button(&mut stderr, action, selected == 0)?;
        queue!(stderr, Print("  "))?;
        draw_button(&mut stderr, "Cancel", selected == 1)?;
        stderr.flush()?;
        match event::read()? {
            Event::Key(key) if key.kind == KeyEventKind::Press => match key.code {
                KeyCode::Tab | KeyCode::Left | KeyCode::Right => selected = 1 - selected,
                KeyCode::Enter | KeyCode::Char(' ') => {
                    execute!(stderr, MoveTo(0, row), Clear(ClearType::CurrentLine))?;
                    return Ok(selected == 0);
                }
                KeyCode::Char('y') => {
                    execute!(stderr, MoveTo(0, row), Clear(ClearType::CurrentLine))?;
                    return Ok(true);
                }
                KeyCode::Esc | KeyCode::Char('n') => {
                    execute!(stderr, MoveTo(0, row), Clear(ClearType::CurrentLine))?;
                    return Ok(false);
                }
                _ => {}
            },
            Event::Mouse(mouse_event) if mouse => {
                if mouse_event.row == row {
                    if let MouseEventKind::Down(MouseButton::Left) = mouse_event.kind {
                        let action_start = prompt.chars().count() as u16 + 2;
                        let action_end = action_start + action.chars().count() as u16 + 4;
                        let cancel_start = action_end + 2;
                        let cancel_end = cancel_start + 10;
                        if (action_start..action_end).contains(&mouse_event.column) {
                            execute!(stderr, MoveTo(0, row), Clear(ClearType::CurrentLine))?;
                            return Ok(true);
                        }
                        if (cancel_start..cancel_end).contains(&mouse_event.column) {
                            execute!(stderr, MoveTo(0, row), Clear(ClearType::CurrentLine))?;
                            return Ok(false);
                        }
                    }
                }
            }
            Event::Resize(_, _) => {}
            _ => {}
        }
    }
}

fn render_picker(
    sources: &[PathBuf],
    selected: usize,
    button: usize,
    start_row: u16,
) -> AppResult<()> {
    let mut stderr = io::stderr();
    queue!(
        stderr,
        MoveTo(0, start_row),
        Clear(ClearType::FromCursorDown),
        Print("Select a source:\r\n")
    )?;
    for (index, source) in sources.iter().enumerate() {
        if index == selected {
            queue!(stderr, SetAttribute(Attribute::Reverse))?;
        }
        queue!(
            stderr,
            Print(format!("  {:>2}. {}", index + 1, source.display()))
        )?;
        if index == selected {
            queue!(stderr, SetAttribute(Attribute::NoReverse))?;
        }
        queue!(stderr, Print("\r\n"))?;
    }
    queue!(stderr, Print("\r\n"))?;
    draw_button(&mut stderr, "Select", button == 0)?;
    queue!(stderr, Print("  "))?;
    draw_button(&mut stderr, "Cancel", button == 1)?;
    stderr.flush()?;
    Ok(())
}

fn draw_button(output: &mut impl Write, label: &str, selected: bool) -> AppResult<()> {
    if selected {
        queue!(output, SetAttribute(Attribute::Reverse))?;
    }
    queue!(output, Print(format!("[ {label} ]")))?;
    if selected {
        queue!(output, SetAttribute(Attribute::NoReverse))?;
    }
    Ok(())
}

fn clear_picker(source_count: usize, start_row: u16) -> AppResult<()> {
    let mut stderr = io::stderr();
    execute!(
        stderr,
        MoveTo(0, start_row),
        Clear(ClearType::FromCursorDown),
        MoveToColumn(0)
    )?;
    let _ = source_count;
    Ok(())
}
