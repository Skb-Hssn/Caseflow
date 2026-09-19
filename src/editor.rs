use crate::error::{AppError, AppResult};
use crate::suggest::{self, Completion};
use crate::terminal as terminal_state;
use crossterm::cursor::{MoveTo, Show};
use crossterm::event::{
    self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEvent, KeyEventKind,
    KeyModifiers, MouseButton, MouseEventKind,
};
use crossterm::style::{Attribute, Print, SetAttribute};
use crossterm::terminal::{self, Clear, ClearType, ScrollUp};
use crossterm::{execute, queue};
use std::fs::{self, OpenOptions};
use std::io::{self, IsTerminal, Write};
use std::path::PathBuf;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

pub enum EditorSignal {
    Success(String),
    CtrlC,
    CtrlD,
}

pub struct LineEditor {
    history: Vec<String>,
    history_path: Option<PathBuf>,
    mouse: bool,
    warning: Option<String>,
}

struct RawGuard;

impl RawGuard {
    fn enter() -> AppResult<Self> {
        terminal::enable_raw_mode()
            .map_err(|error| AppError::new(format!("cannot enter terminal raw mode: {error}")))?;
        execute!(io::stderr(), Show)?;
        Ok(Self)
    }
}

impl Drop for RawGuard {
    fn drop(&mut self) {
        terminal_state::restore_terminal();
    }
}

#[derive(Default)]
struct CompletionMenu {
    candidates: Vec<Completion>,
    selected: usize,
    offset: usize,
}

#[derive(Default, Clone, Copy)]
struct Layout {
    candidate_row: u16,
    visible_candidates: usize,
    button_row: u16,
    select_end: u16,
    cancel_start: u16,
    cancel_end: u16,
}

impl LineEditor {
    pub fn new(history_path: PathBuf, mouse: bool) -> Self {
        let mut warning = None;
        let history = match fs::read_to_string(&history_path) {
            Ok(contents) => contents.lines().map(str::to_owned).collect(),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Vec::new(),
            Err(error) => {
                warning = Some(format!("Command history will not be loaded: {error}"));
                Vec::new()
            }
        };
        let history_path = match history_path.parent() {
            Some(parent) => match fs::create_dir_all(parent) {
                Ok(()) => Some(history_path),
                Err(error) => {
                    warning = Some(format!("Command history will not be persisted: {error}"));
                    None
                }
            },
            None => Some(history_path),
        };
        Self {
            history,
            history_path,
            mouse,
            warning,
        }
    }

    pub fn take_warning(&mut self) -> Option<String> {
        self.warning.take()
    }

    pub fn set_mouse(&mut self, mouse: bool) {
        self.mouse = mouse;
    }

    pub fn read_line(&mut self, prompt: &str) -> AppResult<EditorSignal> {
        if !io::stdin().is_terminal() || !io::stderr().is_terminal() {
            return Err(AppError::new("interactive editor requires a terminal"));
        }
        let _guard = RawGuard::enter()?;
        let (_, mut anchor_row) = crossterm::cursor::position()
            .map_err(|error| AppError::new(format!("cannot read cursor position: {error}")))?;
        let mut buffer = String::new();
        let mut cursor = 0_usize;
        let mut menu: Option<CompletionMenu> = None;
        let mut history_index: Option<usize> = None;
        let mut draft = String::new();
        let mut mouse_capture = false;

        loop {
            let layout = render(prompt, &buffer, cursor, menu.as_mut(), &mut anchor_row)?;
            let should_capture = self.mouse && menu.is_some();
            if should_capture != mouse_capture {
                if should_capture {
                    execute!(io::stderr(), EnableMouseCapture)?;
                } else {
                    execute!(io::stderr(), DisableMouseCapture)?;
                }
                mouse_capture = should_capture;
            }

            match event::read()? {
                Event::Key(key)
                    if matches!(key.kind, KeyEventKind::Press | KeyEventKind::Repeat) =>
                {
                    if let Some(signal) = handle_key(
                        key,
                        &mut buffer,
                        &mut cursor,
                        &mut menu,
                        &self.history,
                        &mut history_index,
                        &mut draft,
                    )? {
                        if mouse_capture {
                            execute!(io::stderr(), DisableMouseCapture)?;
                        }
                        finish_line(prompt, &buffer, anchor_row)?;
                        if let EditorSignal::Success(line) = &signal {
                            self.save_history(line);
                        }
                        return Ok(signal);
                    }
                }
                Event::Paste(value) => {
                    buffer.insert_str(cursor, &value);
                    cursor += value.len();
                    menu = None;
                }
                Event::Mouse(mouse_event) if mouse_capture => match mouse_event.kind {
                    MouseEventKind::Down(MouseButton::Left) => {
                        if mouse_event.row >= layout.candidate_row
                            && mouse_event.row
                                < layout.candidate_row + layout.visible_candidates as u16
                        {
                            let visible = (mouse_event.row - layout.candidate_row) as usize;
                            if let Some(active) = menu.as_mut() {
                                active.selected = active.offset + visible;
                                apply_selected(&mut buffer, &mut cursor, active)?;
                                menu = None;
                            }
                        } else if mouse_event.row == layout.button_row {
                            if mouse_event.column < layout.select_end {
                                if let Some(active) = menu.as_mut() {
                                    apply_selected(&mut buffer, &mut cursor, active)?;
                                }
                                menu = None;
                            } else if (layout.cancel_start..layout.cancel_end)
                                .contains(&mouse_event.column)
                            {
                                menu = None;
                            }
                        }
                    }
                    MouseEventKind::ScrollUp => move_selection(menu.as_mut(), -1),
                    MouseEventKind::ScrollDown => move_selection(menu.as_mut(), 1),
                    _ => {}
                },
                Event::Resize(_, _) => {}
                _ => {}
            }
        }
    }

    fn save_history(&mut self, line: &str) {
        if line.is_empty() || self.history.last().is_some_and(|entry| entry == line) {
            return;
        }
        self.history.push(line.to_string());
        if self.history.len() > 1000 {
            self.history.remove(0);
        }
        let Some(path) = &self.history_path else {
            return;
        };
        match OpenOptions::new().create(true).append(true).open(path) {
            Ok(mut file) => {
                if let Err(error) = writeln!(file, "{line}") {
                    self.warning = Some(format!("Command history could not be saved: {error}"));
                }
            }
            Err(error) => {
                self.warning = Some(format!("Command history could not be saved: {error}"));
                self.history_path = None;
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn handle_key(
    key: KeyEvent,
    buffer: &mut String,
    cursor: &mut usize,
    menu: &mut Option<CompletionMenu>,
    history: &[String],
    history_index: &mut Option<usize>,
    draft: &mut String,
) -> AppResult<Option<EditorSignal>> {
    if key.modifiers.contains(KeyModifiers::CONTROL) {
        match key.code {
            KeyCode::Char('c') => return Ok(Some(EditorSignal::CtrlC)),
            KeyCode::Char('d') if buffer.is_empty() => return Ok(Some(EditorSignal::CtrlD)),
            KeyCode::Char('a') => *cursor = 0,
            KeyCode::Char('e') => *cursor = buffer.len(),
            _ => {}
        }
        return Ok(None);
    }

    match key.code {
        KeyCode::Enter => {
            if let Some(active) = menu.as_mut() {
                apply_selected(buffer, cursor, active)?;
                *menu = None;
            } else {
                return Ok(Some(EditorSignal::Success(buffer.clone())));
            }
        }
        KeyCode::Tab => {
            let candidates = suggest::complete_repl(buffer, *cursor);
            if candidates.len() == 1 {
                let mut active = CompletionMenu {
                    candidates,
                    ..CompletionMenu::default()
                };
                apply_selected(buffer, cursor, &mut active)?;
            } else if !candidates.is_empty() {
                *menu = Some(CompletionMenu {
                    candidates,
                    ..CompletionMenu::default()
                });
            }
        }
        KeyCode::Esc => *menu = None,
        KeyCode::Up if menu.is_some() => move_selection(menu.as_mut(), -1),
        KeyCode::Down if menu.is_some() => move_selection(menu.as_mut(), 1),
        KeyCode::Up => navigate_history(buffer, cursor, history, history_index, draft, -1),
        KeyCode::Down => navigate_history(buffer, cursor, history, history_index, draft, 1),
        KeyCode::Left => *cursor = previous_boundary(buffer, *cursor),
        KeyCode::Right => *cursor = next_boundary(buffer, *cursor),
        KeyCode::Home => *cursor = 0,
        KeyCode::End => *cursor = buffer.len(),
        KeyCode::Backspace => {
            let previous = previous_boundary(buffer, *cursor);
            if previous < *cursor {
                buffer.replace_range(previous..*cursor, "");
                *cursor = previous;
            }
            *menu = None;
        }
        KeyCode::Delete => {
            let next = next_boundary(buffer, *cursor);
            if next > *cursor {
                buffer.replace_range(*cursor..next, "");
            }
            *menu = None;
        }
        KeyCode::Char(character)
            if !key
                .modifiers
                .intersects(KeyModifiers::ALT | KeyModifiers::SUPER) =>
        {
            buffer.insert(*cursor, character);
            *cursor += character.len_utf8();
            *menu = None;
            *history_index = None;
        }
        _ => {}
    }
    Ok(None)
}

fn apply_selected(
    buffer: &mut String,
    cursor: &mut usize,
    menu: &mut CompletionMenu,
) -> AppResult<()> {
    let candidate = menu
        .candidates
        .get(menu.selected)
        .ok_or_else(|| AppError::new("completion selection is no longer valid"))?;
    if candidate.replace_start > candidate.replace_end
        || candidate.replace_end > buffer.len()
        || !buffer.is_char_boundary(candidate.replace_start)
        || !buffer.is_char_boundary(candidate.replace_end)
    {
        return Err(AppError::new("completion replacement range is invalid"));
    }
    let mut replacement = candidate.value.clone();
    if candidate.append_whitespace {
        replacement.push(' ');
    }
    buffer.replace_range(candidate.replace_start..candidate.replace_end, &replacement);
    *cursor = candidate.replace_start + replacement.len();
    Ok(())
}

fn move_selection(menu: Option<&mut CompletionMenu>, direction: i32) {
    let Some(menu) = menu else { return };
    if menu.candidates.is_empty() {
        return;
    }
    if direction < 0 {
        menu.selected = menu.selected.saturating_sub(1);
    } else {
        menu.selected = (menu.selected + 1).min(menu.candidates.len() - 1);
    }
}

fn navigate_history(
    buffer: &mut String,
    cursor: &mut usize,
    history: &[String],
    index: &mut Option<usize>,
    draft: &mut String,
    direction: i32,
) {
    if history.is_empty() {
        return;
    }
    if direction < 0 {
        let next = match *index {
            None => {
                *draft = buffer.clone();
                history.len() - 1
            }
            Some(current) => current.saturating_sub(1),
        };
        *index = Some(next);
        *buffer = history[next].clone();
    } else if let Some(current) = *index {
        if current + 1 < history.len() {
            *index = Some(current + 1);
            *buffer = history[current + 1].clone();
        } else {
            *index = None;
            *buffer = draft.clone();
        }
    }
    *cursor = buffer.len();
}

fn render(
    prompt: &str,
    buffer: &str,
    cursor: usize,
    menu: Option<&mut CompletionMenu>,
    anchor_row: &mut u16,
) -> AppResult<Layout> {
    let (width, height) = terminal::size().unwrap_or((80, 24));
    let width = width.max(1);
    let line_width = UnicodeWidthStr::width(prompt) + UnicodeWidthStr::width(buffer);
    let prompt_rows = (line_width / width as usize) as u16 + 1;
    let visible_capacity = height.saturating_sub(prompt_rows + 3).max(1) as usize;
    let visible = menu
        .as_ref()
        .map(|active| active.candidates.len().min(visible_capacity).min(10))
        .unwrap_or(0);
    let needed_rows = prompt_rows + if visible > 0 { visible as u16 + 1 } else { 0 };
    if anchor_row.saturating_add(needed_rows) >= height {
        let shift = anchor_row
            .saturating_add(needed_rows)
            .saturating_sub(height.saturating_sub(1));
        if shift > 0 {
            execute!(io::stderr(), ScrollUp(shift))?;
            *anchor_row = anchor_row.saturating_sub(shift);
        }
    }

    let mut stderr = io::stderr();
    queue!(
        stderr,
        MoveTo(0, *anchor_row),
        Clear(ClearType::FromCursorDown),
        Print(prompt),
        Print(buffer)
    )?;
    let mut layout = Layout::default();
    if let Some(active) = menu {
        if active.selected < active.offset {
            active.offset = active.selected;
        } else if active.selected >= active.offset + visible {
            active.offset = active.selected + 1 - visible;
        }
        layout.candidate_row = *anchor_row + prompt_rows;
        layout.visible_candidates = visible;
        for visible_index in 0..visible {
            let candidate_index = active.offset + visible_index;
            let candidate = &active.candidates[candidate_index];
            queue!(
                stderr,
                MoveTo(0, layout.candidate_row + visible_index as u16)
            )?;
            if candidate_index == active.selected {
                queue!(stderr, SetAttribute(Attribute::Reverse))?;
            }
            let label = format!("  {}  {}", candidate.value, candidate.description);
            queue!(stderr, Print(truncate_width(&label, width as usize)))?;
            if candidate_index == active.selected {
                queue!(stderr, SetAttribute(Attribute::NoReverse))?;
            }
            queue!(stderr, Clear(ClearType::UntilNewLine))?;
        }
        layout.button_row = layout.candidate_row + visible as u16;
        queue!(stderr, MoveTo(0, layout.button_row))?;
        if width >= 22 {
            draw_button(&mut stderr, "Select", true)?;
            queue!(stderr, Print("  "))?;
            draw_button(&mut stderr, "Cancel", false)?;
            layout.select_end = 10;
            layout.cancel_start = 12;
            layout.cancel_end = 22;
        } else {
            draw_compact_button(&mut stderr, "OK", true)?;
            queue!(stderr, Print(" "))?;
            draw_compact_button(&mut stderr, "X", false)?;
            layout.select_end = 4;
            layout.cancel_start = 5;
            layout.cancel_end = 8;
        }
        if active.candidates.len() > visible {
            queue!(
                stderr,
                Print(format!(
                    "  {}/{}",
                    active.selected + 1,
                    active.candidates.len()
                ))
            )?;
        }
        queue!(stderr, Clear(ClearType::UntilNewLine))?;
    }

    let prefix_width = UnicodeWidthStr::width(prompt)
        + UnicodeWidthStr::width(&buffer[..cursor.min(buffer.len())]);
    let cursor_row = *anchor_row + (prefix_width / width as usize) as u16;
    let cursor_column = (prefix_width % width as usize) as u16;
    queue!(stderr, MoveTo(cursor_column, cursor_row), Show)?;
    stderr.flush()?;
    Ok(layout)
}

fn finish_line(prompt: &str, buffer: &str, anchor_row: u16) -> AppResult<()> {
    let (width, _) = terminal::size().unwrap_or((80, 24));
    let end_width = UnicodeWidthStr::width(prompt) + UnicodeWidthStr::width(buffer);
    let row = anchor_row + (end_width / width.max(1) as usize) as u16;
    let mut stderr = io::stderr();
    execute!(
        stderr,
        MoveTo(0, anchor_row),
        Clear(ClearType::FromCursorDown),
        Print(prompt),
        Print(buffer),
        MoveTo(0, row + 1)
    )?;
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

fn draw_compact_button(output: &mut impl Write, label: &str, selected: bool) -> AppResult<()> {
    if selected {
        queue!(output, SetAttribute(Attribute::Reverse))?;
    }
    queue!(output, Print(format!("[{label}]")))?;
    if selected {
        queue!(output, SetAttribute(Attribute::NoReverse))?;
    }
    Ok(())
}

fn truncate_width(value: &str, width: usize) -> String {
    if UnicodeWidthStr::width(value) <= width {
        return value.to_string();
    }
    let target = width.saturating_sub(1);
    let mut current = 0;
    let mut result = String::new();
    for character in value.chars() {
        let character_width = character.width().unwrap_or(0);
        if current + character_width > target {
            break;
        }
        result.push(character);
        current += character_width;
    }
    result.push('…');
    result
}

fn previous_boundary(value: &str, cursor: usize) -> usize {
    value[..cursor.min(value.len())]
        .char_indices()
        .next_back()
        .map(|(index, _)| index)
        .unwrap_or(0)
}

fn next_boundary(value: &str, cursor: usize) -> usize {
    value[cursor.min(value.len())..]
        .char_indices()
        .nth(1)
        .map(|(offset, _)| cursor + offset)
        .unwrap_or(value.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn character_boundaries_handle_unicode() {
        let value = "aλz";
        assert_eq!(next_boundary(value, 1), 3);
        assert_eq!(previous_boundary(value, 3), 1);
    }

    #[test]
    fn truncation_respects_display_width() {
        assert_eq!(truncate_width("abcdef", 4), "abc…");
        assert_eq!(truncate_width("abc", 4), "abc");
    }
}
