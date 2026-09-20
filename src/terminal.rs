use crate::error::{AppError, AppResult};
use crate::theme;
use crossterm::cursor::{Hide, MoveTo, MoveToColumn, Show};
use crossterm::event::{self, Event, KeyCode, KeyEventKind, MouseButton, MouseEventKind};
use crossterm::style::{
    Attribute, Color, Print, ResetColor, SetAttribute, SetBackgroundColor, SetForegroundColor,
};
use crossterm::terminal::{self, Clear, ClearType, ScrollUp};
use crossterm::{execute, queue};
use std::io::{self, IsTerminal, Write};
use std::path::PathBuf;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

struct TerminalGuard;

// Normal tracking reports clicks, releases, and wheel events. Crossterm's
// EnableMouseCapture also enables all-motion tracking (1003), which floods an
// interactive editor with events whenever the pointer moves.
const ENABLE_MOUSE_CLICKS: &str = "\x1b[?1000h\x1b[?1006h";
const DISABLE_MOUSE_TRACKING: &str = "\x1b[?1006l\x1b[?1003l\x1b[?1002l\x1b[?1000l";

impl TerminalGuard {
    fn enter(mouse: bool) -> AppResult<Self> {
        terminal::enable_raw_mode()
            .map_err(|error| AppError::new(format!("cannot enter terminal raw mode: {error}")))?;
        let mut stderr = io::stderr();
        if mouse {
            execute!(stderr, Print(ENABLE_MOUSE_CLICKS), Hide)?;
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
        Print(DISABLE_MOUSE_TRACKING),
        Show,
        SetAttribute(Attribute::Reset)
    );
    let _ = terminal::disable_raw_mode();
}

pub fn enable_mouse_capture() -> AppResult<()> {
    execute!(io::stderr(), Print(ENABLE_MOUSE_CLICKS))?;
    Ok(())
}

pub fn disable_mouse_capture() -> AppResult<()> {
    execute!(io::stderr(), Print(DISABLE_MOUSE_TRACKING))?;
    Ok(())
}

pub fn select_source(sources: &[PathBuf], mouse: bool, color: bool) -> AppResult<Option<PathBuf>> {
    if sources.is_empty() {
        return Ok(None);
    }
    if !io::stdin().is_terminal() || !io::stderr().is_terminal() {
        return Ok(sources.first().cloned());
    }
    let _guard = TerminalGuard::enter(mouse)?;
    let (_, mut start_row) = crossterm::cursor::position().unwrap_or((0, 0));
    let mut query = String::new();
    let mut matches = filtered_sources(sources, &query);
    let mut selected = 0_usize;
    let mut offset = 0_usize;
    let mut button = 0_usize;
    loop {
        let layout = render_picker(
            sources,
            &matches,
            &query,
            selected,
            &mut offset,
            button,
            &mut start_row,
            color,
        )?;
        match event::read()? {
            Event::Key(key) if key.kind == KeyEventKind::Press => match key.code {
                KeyCode::Up => selected = selected.saturating_sub(1),
                KeyCode::Down => {
                    selected = (selected + 1).min(matches.len().saturating_sub(1));
                }
                KeyCode::Tab | KeyCode::Right | KeyCode::Left => button = 1 - button,
                KeyCode::Enter | KeyCode::Char(' ') => {
                    clear_picker(start_row)?;
                    return Ok((button == 0)
                        .then(|| {
                            matches
                                .get(selected)
                                .map(|item| sources[item.index].clone())
                        })
                        .flatten());
                }
                KeyCode::Backspace => {
                    query.pop();
                    matches = filtered_sources(sources, &query);
                    selected = 0;
                    offset = 0;
                }
                KeyCode::Esc => {
                    clear_picker(start_row)?;
                    return Ok(None);
                }
                KeyCode::Char(character)
                    if !key.modifiers.contains(event::KeyModifiers::CONTROL) =>
                {
                    query.push(character);
                    matches = filtered_sources(sources, &query);
                    selected = 0;
                    offset = 0;
                }
                _ => {}
            },
            Event::Mouse(mouse_event) if mouse => match mouse_event.kind {
                MouseEventKind::Down(MouseButton::Left) => {
                    if mouse_event.row >= layout.candidate_row
                        && mouse_event.row < layout.candidate_row + layout.visible_candidates as u16
                    {
                        selected = offset + (mouse_event.row - layout.candidate_row) as usize;
                    } else if mouse_event.row == layout.button_row {
                        if mouse_event.column < layout.select_end {
                            clear_picker(start_row)?;
                            return Ok(matches
                                .get(selected)
                                .map(|item| sources[item.index].clone()));
                        }
                        if (layout.cancel_start..layout.cancel_end).contains(&mouse_event.column) {
                            clear_picker(start_row)?;
                            return Ok(None);
                        }
                    }
                }
                MouseEventKind::ScrollUp => selected = selected.saturating_sub(1),
                MouseEventKind::ScrollDown => {
                    selected = (selected + 1).min(matches.len().saturating_sub(1));
                }
                _ => {}
            },
            Event::Resize(_, _) => {}
            _ => {}
        }
    }
}

pub fn confirm(prompt: &str, action: &str, mouse: bool, color: bool) -> AppResult<bool> {
    if !io::stdin().is_terminal() || !io::stderr().is_terminal() {
        return Ok(false);
    }
    let _guard = TerminalGuard::enter(mouse)?;
    let (_, row) = crossterm::cursor::position().unwrap_or((0, 0));
    let mut selected = 1_usize;
    loop {
        let (width, _) = terminal::size().unwrap_or((80, 24));
        let width = width.max(1) as usize;
        let compact = width < action.chars().count() + 18;
        let action_label = if compact { "Y" } else { action };
        let cancel_label = if compact { "N" } else { "Cancel" };
        let action_width = action_label.chars().count() + if compact { 2 } else { 4 };
        let cancel_width = cancel_label.chars().count() + if compact { 2 } else { 4 };
        let buttons_width = action_width + 2 + cancel_width;
        let prompt = truncate_width(prompt, width);
        let same_line = UnicodeWidthStr::width(prompt.as_str()) + 2 + buttons_width <= width;
        let button_row = row + u16::from(!same_line);
        let action_start = if same_line {
            UnicodeWidthStr::width(prompt.as_str()) as u16 + 2
        } else {
            0
        };
        let action_end = action_start + action_width as u16;
        let cancel_start = action_end + 2;
        let cancel_end = cancel_start + cancel_width as u16;
        let mut stderr = io::stderr();
        queue!(stderr, MoveTo(0, row), Clear(ClearType::FromCursorDown))?;
        if color {
            queue!(stderr, SetForegroundColor(theme::WARNING))?;
        }
        queue!(
            stderr,
            SetAttribute(Attribute::Bold),
            Print(&prompt),
            SetAttribute(Attribute::Reset),
            ResetColor
        )?;
        if same_line {
            queue!(stderr, Print("  "))?;
        } else {
            queue!(stderr, MoveTo(0, button_row))?;
        }
        draw_tinted_button(
            &mut stderr,
            action_label,
            selected == 0,
            compact,
            color,
            theme::DANGER,
        )?;
        queue!(stderr, Print("  "))?;
        draw_tinted_button(
            &mut stderr,
            cancel_label,
            selected == 1,
            compact,
            color,
            theme::MUTED,
        )?;
        stderr.flush()?;
        match event::read()? {
            Event::Key(key) if key.kind == KeyEventKind::Press => match key.code {
                KeyCode::Tab | KeyCode::Left | KeyCode::Right => selected = 1 - selected,
                KeyCode::Enter | KeyCode::Char(' ') => {
                    clear_from(row)?;
                    return Ok(selected == 0);
                }
                KeyCode::Char('y') => {
                    clear_from(row)?;
                    return Ok(true);
                }
                KeyCode::Esc | KeyCode::Char('n') => {
                    clear_from(row)?;
                    return Ok(false);
                }
                _ => {}
            },
            Event::Mouse(mouse_event) if mouse => {
                if mouse_event.row == button_row {
                    if let MouseEventKind::Down(MouseButton::Left) = mouse_event.kind {
                        if (action_start..action_end).contains(&mouse_event.column) {
                            clear_from(row)?;
                            return Ok(true);
                        }
                        if (cancel_start..cancel_end).contains(&mouse_event.column) {
                            clear_from(row)?;
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

fn clear_from(row: u16) -> AppResult<()> {
    execute!(
        io::stderr(),
        MoveTo(0, row),
        Clear(ClearType::FromCursorDown),
        MoveToColumn(0)
    )?;
    Ok(())
}

#[derive(Clone, Copy)]
struct SourceMatch {
    index: usize,
    score: i32,
}

#[derive(Default, Clone, Copy)]
struct PickerLayout {
    candidate_row: u16,
    visible_candidates: usize,
    button_row: u16,
    select_end: u16,
    cancel_start: u16,
    cancel_end: u16,
}

fn filtered_sources(sources: &[PathBuf], query: &str) -> Vec<SourceMatch> {
    let mut matches = sources
        .iter()
        .enumerate()
        .filter_map(|(index, source)| {
            fuzzy_score(&source.to_string_lossy(), query).map(|score| SourceMatch { index, score })
        })
        .collect::<Vec<_>>();
    matches.sort_by(|left, right| {
        right.score.cmp(&left.score).then_with(|| {
            sources[left.index]
                .to_string_lossy()
                .cmp(&sources[right.index].to_string_lossy())
        })
    });
    matches
}

fn fuzzy_score(candidate: &str, query: &str) -> Option<i32> {
    if query.is_empty() {
        return Some(0);
    }
    let candidate = candidate.to_lowercase();
    let query = query.to_lowercase();
    let mut score = 0_i32;
    let mut search_from = 0_usize;
    let mut previous = None;
    for wanted in query.chars() {
        let (relative, _) = candidate[search_from..]
            .char_indices()
            .find(|(_, character)| *character == wanted)?;
        let index = search_from + relative;
        score += 10;
        if previous.is_some_and(|position| position + 1 == index) {
            score += 12;
        }
        if index == 0
            || candidate[..index]
                .chars()
                .next_back()
                .is_some_and(|character| matches!(character, '/' | '\\' | '_' | '-' | '.'))
        {
            score += 8;
        }
        score -= relative.min(i32::MAX as usize) as i32;
        previous = Some(index);
        search_from = index + wanted.len_utf8();
    }
    score -= candidate.len().min(i32::MAX as usize) as i32 / 16;
    Some(score)
}

#[allow(clippy::too_many_arguments)]
fn render_picker(
    sources: &[PathBuf],
    matches: &[SourceMatch],
    query: &str,
    selected: usize,
    offset: &mut usize,
    button: usize,
    start_row: &mut u16,
    color: bool,
) -> AppResult<PickerLayout> {
    let (width, height) = terminal::size().unwrap_or((80, 24));
    let width = width.max(1);
    let visible = matches
        .len()
        .min(height.saturating_sub(5).max(1) as usize)
        .min(15);
    let required = visible as u16 + 4;
    if start_row.saturating_add(required) >= height {
        let shift = start_row
            .saturating_add(required)
            .saturating_sub(height.saturating_sub(1));
        if shift > 0 {
            execute!(io::stderr(), ScrollUp(shift))?;
            *start_row = start_row.saturating_sub(shift);
        }
    }
    if selected < *offset {
        *offset = selected;
    } else if visible > 0 && selected >= *offset + visible {
        *offset = selected + 1 - visible;
    }
    let candidate_row = *start_row + 2;
    let button_row = candidate_row + visible as u16 + 1;
    let mut stderr = io::stderr();
    queue!(
        stderr,
        MoveTo(0, *start_row),
        Clear(ClearType::FromCursorDown)
    )?;
    if color {
        queue!(stderr, SetForegroundColor(theme::PRIMARY))?;
    }
    queue!(
        stderr,
        SetAttribute(Attribute::Bold),
        Print("Select source"),
        SetAttribute(Attribute::Reset),
        ResetColor,
        SetAttribute(Attribute::Dim),
        Print(format!("  ·  {} files", sources.len())),
        SetAttribute(Attribute::Reset),
        Clear(ClearType::UntilNewLine),
        MoveTo(0, *start_row + 1)
    )?;
    if color {
        queue!(stderr, SetForegroundColor(theme::PRIMARY_SOFT))?;
    }
    queue!(
        stderr,
        SetAttribute(Attribute::Bold),
        Print("Search"),
        SetAttribute(Attribute::Reset),
        ResetColor,
        Print("  "),
        Print(truncate_width(
            &if matches.is_empty() {
                format!("{query}  ·  no matches")
            } else if query.is_empty() {
                format!("type to filter…  ·  {} shown", matches.len())
            } else {
                format!("{query}  ·  {} matches", matches.len())
            },
            width.saturating_sub(8) as usize
        )),
        Clear(ClearType::UntilNewLine)
    )?;
    for visible_index in 0..visible {
        let match_index = *offset + visible_index;
        let source_match = matches[match_index];
        let source = &sources[source_match.index];
        queue!(stderr, MoveTo(0, candidate_row + visible_index as u16))?;
        let is_selected = match_index == selected;
        if is_selected && color {
            queue!(
                stderr,
                SetForegroundColor(theme::ON_ACCENT),
                SetBackgroundColor(theme::PRIMARY),
                SetAttribute(Attribute::Bold)
            )?;
        } else if is_selected {
            queue!(stderr, SetAttribute(Attribute::Reverse))?;
        }
        let marker = if is_selected { "›" } else { " " };
        let label = format!(" {marker} {}", source.display());
        let label = truncate_width(&label, width.saturating_sub(1) as usize);
        if is_selected {
            queue!(
                stderr,
                Print(pad_width(&label, width.saturating_sub(1) as usize)),
                SetAttribute(Attribute::Reset),
                ResetColor
            )?;
        } else {
            queue!(stderr, Print(label))?;
        }
        queue!(stderr, Clear(ClearType::UntilNewLine))?;
    }
    queue!(stderr, MoveTo(0, button_row))?;
    let (select_end, cancel_start, cancel_end) = if width >= 22 {
        draw_tinted_button(
            &mut stderr,
            "Select",
            button == 0,
            false,
            color,
            theme::PRIMARY,
        )?;
        queue!(stderr, Print("  "))?;
        draw_tinted_button(
            &mut stderr,
            "Close",
            button == 1,
            false,
            color,
            theme::MUTED,
        )?;
        (10, 12, 21)
    } else {
        draw_tinted_button(&mut stderr, "OK", button == 0, true, color, theme::PRIMARY)?;
        queue!(stderr, Print(" "))?;
        draw_tinted_button(&mut stderr, "X", button == 1, true, color, theme::MUTED)?;
        (4, 5, 8)
    };
    if matches.len() > visible && !matches.is_empty() {
        queue!(
            stderr,
            Print(format!("  {}/{}", selected + 1, matches.len()))
        )?;
    }
    if width >= 72 {
        queue!(
            stderr,
            SetAttribute(Attribute::Dim),
            Print("  type to filter · ↑↓ navigate · Enter select"),
            SetAttribute(Attribute::Reset)
        )?;
    }
    queue!(stderr, Clear(ClearType::UntilNewLine))?;
    stderr.flush()?;
    Ok(PickerLayout {
        candidate_row,
        visible_candidates: visible,
        button_row,
        select_end,
        cancel_start,
        cancel_end,
    })
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

fn draw_tinted_button(
    output: &mut impl Write,
    label: &str,
    selected: bool,
    compact: bool,
    color: bool,
    tint: Color,
) -> AppResult<()> {
    if !color {
        return if compact {
            draw_compact_button(output, label, selected)
        } else {
            draw_button(output, label, selected)
        };
    }
    if selected {
        queue!(
            output,
            SetForegroundColor(theme::ON_ACCENT),
            SetBackgroundColor(tint),
            SetAttribute(Attribute::Bold)
        )?;
    } else {
        queue!(output, SetForegroundColor(tint))?;
    }
    let button = if compact {
        format!("[{label}]")
    } else {
        format!("[ {label} ]")
    };
    queue!(
        output,
        Print(button),
        SetAttribute(Attribute::Reset),
        ResetColor
    )?;
    Ok(())
}

fn clear_picker(start_row: u16) -> AppResult<()> {
    let mut stderr = io::stderr();
    execute!(
        stderr,
        MoveTo(0, start_row),
        Clear(ClearType::FromCursorDown),
        MoveToColumn(0)
    )?;
    Ok(())
}

fn truncate_width(value: &str, width: usize) -> String {
    if UnicodeWidthStr::width(value) <= width {
        return value.to_string();
    }
    let target = width.saturating_sub(1);
    let mut used = 0;
    let mut result = String::new();
    for character in value.chars() {
        let character_width = character.width().unwrap_or(0);
        if used + character_width > target {
            break;
        }
        result.push(character);
        used += character_width;
    }
    result.push('…');
    result
}

fn pad_width(value: &str, width: usize) -> String {
    let padding = width.saturating_sub(UnicodeWidthStr::width(value));
    format!("{value}{}", " ".repeat(padding))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fuzzy_matching_accepts_non_contiguous_text() {
        assert!(fuzzy_score("solutions/long_answer.cpp", "slncpp").is_some());
        assert!(fuzzy_score("solutions/long_answer.cpp", "zxy").is_none());
    }

    #[test]
    fn fuzzy_matching_prefers_compact_boundary_matches() {
        let compact = fuzzy_score("src/main.cpp", "main").unwrap();
        let scattered = fuzzy_score("many_assets_in_name.cpp", "main").unwrap();
        assert!(compact > scattered);
    }

    #[test]
    fn truncation_handles_narrow_terminals() {
        assert_eq!(truncate_width("abcdefgh", 4), "abc…");
    }

    #[test]
    fn selected_rows_can_fill_the_available_width() {
        assert_eq!(pad_width("λ", 3), "λ  ");
    }
}
