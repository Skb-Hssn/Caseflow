use crate::error::{AppError, AppResult};
use crate::suggest::{self, Completion};
use crate::terminal as terminal_state;
use crossterm::cursor::{MoveTo, Show};
use crossterm::event::{
    self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEvent, KeyEventKind,
    KeyModifiers, MouseButton, MouseEventKind,
};
use crossterm::style::{
    Attribute, Color, Print, ResetColor, SetAttribute, SetBackgroundColor, SetForegroundColor,
};
use crossterm::terminal::{self, Clear, ClearType, ScrollUp};
use crossterm::{execute, queue};
use std::fs::{self, OpenOptions};
use std::io::{self, IsTerminal, Write};
use std::path::PathBuf;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

pub enum EditorSignal {
    Success(String),
    Action(EditorAction),
    CtrlC,
    CtrlD,
}

#[derive(Clone, Copy)]
pub enum EditorAction {
    Run,
    Build,
    Test,
}

pub struct LineEditor {
    history: Vec<String>,
    history_path: Option<PathBuf>,
    mouse: bool,
    color: bool,
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
    dimmed: bool,
}

#[derive(Default, Clone, Copy)]
struct Layout {
    candidate_row: u16,
    visible_candidates: usize,
    button_row: u16,
    select_end: u16,
    cancel_start: u16,
    cancel_end: u16,
    toolbar_row: u16,
    run_start: u16,
    run_end: u16,
    build_start: u16,
    build_end: u16,
    test_start: u16,
    test_end: u16,
}

#[derive(Default)]
struct RenderState {
    dynamic_start: u16,
    dynamic_end: u16,
    has_dynamic: bool,
    toolbar: Option<Layout>,
    toolbar_size: Option<(u16, u16)>,
    toolbar_dirty: bool,
}

#[derive(Clone, Copy)]
struct RenderOptions {
    show_toolbar: bool,
    color: bool,
}

impl LineEditor {
    pub fn new(history_path: PathBuf, mouse: bool, color: bool) -> Self {
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
            color,
            warning,
        }
    }

    pub fn take_warning(&mut self) -> Option<String> {
        self.warning.take()
    }

    pub fn set_mouse(&mut self, mouse: bool) {
        self.mouse = mouse;
    }

    pub fn set_color(&mut self, color: bool) {
        self.color = color;
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
        let mut render_state = RenderState::default();

        loop {
            let layout = render(
                prompt,
                &buffer,
                cursor,
                menu.as_mut(),
                &mut anchor_row,
                RenderOptions {
                    show_toolbar: self.mouse,
                    color: self.color,
                },
                &mut render_state,
            )?;
            let should_capture = self.mouse;
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
                        finish_line(prompt, &buffer, anchor_row, self.color)?;
                        if let EditorSignal::Success(line) = &signal {
                            self.save_history(line);
                        }
                        return Ok(signal);
                    }
                }
                Event::Paste(value) => {
                    buffer.insert_str(cursor, &value);
                    cursor += value.len();
                    refresh_command_menu(&buffer, cursor, &mut menu);
                }
                Event::Mouse(mouse_event) if mouse_capture => match mouse_event.kind {
                    MouseEventKind::Down(MouseButton::Left) => {
                        let action = if mouse_event.row == layout.toolbar_row
                            && (layout.run_start..layout.run_end).contains(&mouse_event.column)
                        {
                            Some(EditorAction::Run)
                        } else if mouse_event.row == layout.toolbar_row
                            && (layout.build_start..layout.build_end).contains(&mouse_event.column)
                        {
                            Some(EditorAction::Build)
                        } else if mouse_event.row == layout.toolbar_row
                            && (layout.test_start..layout.test_end).contains(&mouse_event.column)
                        {
                            Some(EditorAction::Test)
                        } else {
                            None
                        };
                        if let Some(action) = action {
                            execute!(io::stderr(), DisableMouseCapture)?;
                            finish_line(prompt, &buffer, anchor_row, self.color)?;
                            return Ok(EditorSignal::Action(action));
                        } else if mouse_event.row >= layout.candidate_row
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
                    MouseEventKind::ScrollUp => activate_selection(menu.as_mut(), -1),
                    MouseEventKind::ScrollDown => activate_selection(menu.as_mut(), 1),
                    _ => {}
                },
                Event::Resize(_, _) => render_state.toolbar_dirty = true,
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
            if menu.as_ref().is_some_and(|active| active.dimmed) {
                return Ok(Some(EditorSignal::Success(buffer.clone())));
            } else if let Some(active) = menu.as_mut() {
                apply_selected(buffer, cursor, active)?;
                *menu = None;
            } else {
                return Ok(Some(EditorSignal::Success(buffer.clone())));
            }
        }
        KeyCode::Tab => {
            if let Some(active) = menu.as_mut() {
                apply_selected(buffer, cursor, active)?;
                *menu = None;
            } else {
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
        }
        KeyCode::Esc => *menu = None,
        KeyCode::Up if menu.is_some() => activate_selection(menu.as_mut(), -1),
        KeyCode::Down if menu.is_some() => activate_selection(menu.as_mut(), 1),
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
            refresh_command_menu(buffer, *cursor, menu);
        }
        KeyCode::Delete => {
            let next = next_boundary(buffer, *cursor);
            if next > *cursor {
                buffer.replace_range(*cursor..next, "");
            }
            refresh_command_menu(buffer, *cursor, menu);
        }
        KeyCode::Char(character)
            if !key
                .modifiers
                .intersects(KeyModifiers::ALT | KeyModifiers::SUPER) =>
        {
            buffer.insert(*cursor, character);
            *cursor += character.len_utf8();
            refresh_command_menu(buffer, *cursor, menu);
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

fn activate_selection(menu: Option<&mut CompletionMenu>, direction: i32) {
    let Some(menu) = menu else { return };
    menu.dimmed = false;
    move_selection(Some(menu), direction);
}

fn refresh_command_menu(buffer: &str, cursor: usize, menu: &mut Option<CompletionMenu>) {
    let before_cursor = &buffer[..cursor.min(buffer.len())];
    if before_cursor.starts_with('/') && !before_cursor.chars().any(char::is_whitespace) {
        let candidates = suggest::complete_repl(buffer, cursor);
        if !candidates.is_empty() {
            *menu = Some(CompletionMenu {
                candidates,
                dimmed: true,
                ..CompletionMenu::default()
            });
            return;
        }
    }
    *menu = None;
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
    options: RenderOptions,
    state: &mut RenderState,
) -> AppResult<Layout> {
    let (width, height) = terminal::size().unwrap_or((80, 24));
    let width = width.max(1);
    let show_toolbar = options.show_toolbar && height >= 2;
    let color = options.color;
    let content_height = height.saturating_sub(u16::from(show_toolbar)).max(1);
    let line_width = UnicodeWidthStr::width(prompt) + UnicodeWidthStr::width(buffer);
    let prompt_rows = (line_width / width as usize) as u16 + 1;
    let visible_capacity = content_height.saturating_sub(prompt_rows + 3).max(1) as usize;
    let visible = menu
        .as_ref()
        .map(|active| active.candidates.len().min(visible_capacity).min(10))
        .unwrap_or(0);
    let needed_rows = prompt_rows + if visible > 0 { visible as u16 + 2 } else { 0 };
    if anchor_row.saturating_add(needed_rows) >= content_height {
        let shift = anchor_row
            .saturating_add(needed_rows)
            .saturating_sub(content_height.saturating_sub(1));
        if shift > 0 {
            execute!(io::stderr(), ScrollUp(shift))?;
            *anchor_row = anchor_row.saturating_sub(shift);
            if state.has_dynamic {
                state.dynamic_start = state.dynamic_start.saturating_sub(shift);
                state.dynamic_end = state.dynamic_end.saturating_sub(shift);
            }
            if let Some(toolbar) = state.toolbar.as_mut() {
                toolbar.toolbar_row = toolbar.toolbar_row.saturating_sub(shift);
            }
            state.toolbar_dirty = true;
        }
    }

    let dynamic_start = *anchor_row;
    let dynamic_end = anchor_row.saturating_add(needed_rows).min(content_height);
    let clear_start = if state.has_dynamic {
        state.dynamic_start.min(dynamic_start)
    } else {
        dynamic_start
    };
    let clear_end = if state.has_dynamic {
        state.dynamic_end.max(dynamic_end)
    } else {
        dynamic_end
    }
    .min(content_height);
    let mut stderr = io::stderr();
    for row in clear_start..clear_end {
        queue!(stderr, MoveTo(0, row), Clear(ClearType::CurrentLine))?;
    }
    queue!(stderr, MoveTo(0, *anchor_row))?;
    draw_prompt(&mut stderr, prompt, color)?;
    queue!(stderr, Print(buffer))?;
    let mut layout = Layout::default();
    if let Some(active) = menu {
        if active.selected < active.offset {
            active.offset = active.selected;
        } else if active.selected >= active.offset + visible {
            active.offset = active.selected + 1 - visible;
        }
        let header_row = *anchor_row + prompt_rows;
        queue!(stderr, MoveTo(0, header_row))?;
        if color {
            queue!(
                stderr,
                SetForegroundColor(if active.dimmed {
                    Color::DarkGrey
                } else {
                    Color::Cyan
                })
            )?;
        }
        if active.dimmed {
            queue!(stderr, SetAttribute(Attribute::Dim))?;
        } else {
            queue!(stderr, SetAttribute(Attribute::Bold))?;
        }
        queue!(
            stderr,
            Print(if active.dimmed {
                "  Commands"
            } else {
                "  Suggestions"
            }),
            SetAttribute(Attribute::Reset),
            ResetColor,
            SetAttribute(Attribute::Dim),
            Print(format!("  ·  {} matches", active.candidates.len())),
            SetAttribute(Attribute::Reset),
            Clear(ClearType::UntilNewLine)
        )?;
        layout.candidate_row = header_row + 1;
        layout.visible_candidates = visible;
        for visible_index in 0..visible {
            let candidate_index = active.offset + visible_index;
            let candidate = &active.candidates[candidate_index];
            queue!(
                stderr,
                MoveTo(0, layout.candidate_row + visible_index as u16)
            )?;
            let is_selected = candidate_index == active.selected;
            if active.dimmed {
                queue!(stderr, SetAttribute(Attribute::Dim))?;
                if color {
                    queue!(stderr, SetForegroundColor(Color::DarkGrey))?;
                }
            } else if is_selected && color {
                queue!(
                    stderr,
                    SetForegroundColor(Color::Black),
                    SetBackgroundColor(Color::Cyan),
                    SetAttribute(Attribute::Bold)
                )?;
            } else if is_selected {
                queue!(stderr, SetAttribute(Attribute::Reverse))?;
            }
            let marker = if active.dimmed {
                "·"
            } else if is_selected {
                "›"
            } else {
                " "
            };
            let label = format!(
                " {marker} {}  ·  {}",
                candidate.value, candidate.description
            );
            let label = truncate_width(&label, width.saturating_sub(1) as usize);
            if is_selected && !active.dimmed {
                queue!(
                    stderr,
                    Print(pad_width(&label, width.saturating_sub(1) as usize)),
                    SetAttribute(Attribute::Reset),
                    ResetColor
                )?;
            } else {
                queue!(
                    stderr,
                    Print(label),
                    SetAttribute(Attribute::Reset),
                    ResetColor
                )?;
            }
            queue!(stderr, Clear(ClearType::UntilNewLine))?;
        }
        layout.button_row = layout.candidate_row + visible as u16;
        queue!(stderr, MoveTo(0, layout.button_row))?;
        if active.dimmed {
            queue!(
                stderr,
                SetAttribute(Attribute::Dim),
                Print("  Tab complete · ↑↓ choose · Esc close"),
                SetAttribute(Attribute::Reset),
                Clear(ClearType::UntilNewLine)
            )?;
        } else if width >= 22 {
            draw_button(&mut stderr, "Select", true)?;
            queue!(stderr, Print("  "))?;
            draw_button(&mut stderr, "Close", false)?;
            layout.select_end = 10;
            layout.cancel_start = 12;
            layout.cancel_end = 21;
        } else {
            draw_compact_button(&mut stderr, "OK", true)?;
            queue!(stderr, Print(" "))?;
            draw_compact_button(&mut stderr, "X", false)?;
            layout.select_end = 4;
            layout.cancel_start = 5;
            layout.cancel_end = 8;
        }
        if !active.dimmed && active.candidates.len() > visible {
            queue!(
                stderr,
                Print(format!(
                    "  {}/{}",
                    active.selected + 1,
                    active.candidates.len()
                ))
            )?;
        }
        if !active.dimmed && width >= 72 {
            queue!(
                stderr,
                SetAttribute(Attribute::Dim),
                Print("  ↑↓ navigate · Enter select · Esc close"),
                SetAttribute(Attribute::Reset)
            )?;
        }
        queue!(stderr, Clear(ClearType::UntilNewLine))?;
    }

    if show_toolbar {
        let toolbar_changed = state.toolbar_dirty
            || state.toolbar_size != Some((width, height))
            || state.toolbar.is_none();
        if toolbar_changed {
            if let Some(previous) = state.toolbar {
                if previous.toolbar_row != height - 1 && previous.toolbar_row < height {
                    queue!(
                        stderr,
                        MoveTo(0, previous.toolbar_row),
                        Clear(ClearType::CurrentLine)
                    )?;
                }
            }
            draw_toolbar(&mut stderr, width, height - 1, color, &mut layout)?;
            state.toolbar = Some(toolbar_only(layout));
            state.toolbar_size = Some((width, height));
            state.toolbar_dirty = false;
        } else if let Some(toolbar) = state.toolbar {
            copy_toolbar(&mut layout, toolbar);
        }
    } else if let Some(previous) = state.toolbar.take() {
        if previous.toolbar_row < height {
            queue!(
                stderr,
                MoveTo(0, previous.toolbar_row),
                Clear(ClearType::CurrentLine)
            )?;
        }
        state.toolbar_size = None;
    }

    state.dynamic_start = dynamic_start;
    state.dynamic_end = dynamic_end;
    state.has_dynamic = true;

    let prefix_width = UnicodeWidthStr::width(prompt)
        + UnicodeWidthStr::width(&buffer[..cursor.min(buffer.len())]);
    let cursor_row = *anchor_row + (prefix_width / width as usize) as u16;
    let cursor_column = (prefix_width % width as usize) as u16;
    queue!(stderr, MoveTo(cursor_column, cursor_row), Show)?;
    stderr.flush()?;
    Ok(layout)
}

fn finish_line(prompt: &str, buffer: &str, anchor_row: u16, color: bool) -> AppResult<()> {
    let (width, _) = terminal::size().unwrap_or((80, 24));
    let end_width = UnicodeWidthStr::width(prompt) + UnicodeWidthStr::width(buffer);
    let row = anchor_row + (end_width / width.max(1) as usize) as u16;
    let mut stderr = io::stderr();
    queue!(
        stderr,
        MoveTo(0, anchor_row),
        Clear(ClearType::FromCursorDown)
    )?;
    draw_prompt(&mut stderr, prompt, color)?;
    execute!(stderr, Print(buffer), MoveTo(0, row + 1))?;
    Ok(())
}

fn draw_prompt(output: &mut impl Write, prompt: &str, color: bool) -> AppResult<()> {
    if !color {
        queue!(
            output,
            SetAttribute(Attribute::Bold),
            Print(prompt),
            SetAttribute(Attribute::Reset)
        )?;
        return Ok(());
    }

    let Some(rest) = prompt.strip_prefix("run-cli:") else {
        queue!(
            output,
            SetForegroundColor(Color::Cyan),
            SetAttribute(Attribute::Bold),
            Print(prompt),
            SetAttribute(Attribute::Reset),
            ResetColor
        )?;
        return Ok(());
    };
    let (source, mode) = rest.split_once(" · ").unwrap_or((rest, ""));
    let mode = mode.strip_suffix(" › ").unwrap_or(mode);
    queue!(
        output,
        SetForegroundColor(Color::Cyan),
        SetAttribute(Attribute::Bold),
        Print("run-cli"),
        SetAttribute(Attribute::Reset),
        SetForegroundColor(Color::DarkGrey),
        Print(":"),
        SetForegroundColor(Color::Yellow),
        SetAttribute(Attribute::Bold),
        Print(source),
        SetAttribute(Attribute::Reset),
        SetForegroundColor(Color::DarkGrey),
        Print(" · "),
        SetForegroundColor(Color::Magenta),
        Print(mode),
        SetForegroundColor(Color::Cyan),
        SetAttribute(Attribute::Bold),
        Print(" › "),
        SetAttribute(Attribute::Reset),
        ResetColor
    )?;
    Ok(())
}

fn draw_toolbar(
    output: &mut impl Write,
    width: u16,
    row: u16,
    color: bool,
    layout: &mut Layout,
) -> AppResult<()> {
    layout.toolbar_row = row;
    queue!(output, MoveTo(0, row), Clear(ClearType::CurrentLine))?;
    let tiny = width < 12;
    let compact = width < 30;
    let prefix = if tiny {
        ""
    } else if width >= 40 {
        "  Actions  "
    } else {
        " "
    };
    if color {
        queue!(output, SetForegroundColor(Color::DarkGrey))?;
    } else {
        queue!(output, SetAttribute(Attribute::Dim))?;
    }
    queue!(
        output,
        Print(prefix),
        SetAttribute(Attribute::Reset),
        ResetColor
    )?;
    let mut column = UnicodeWidthStr::width(prefix) as u16;
    let spacing = if tiny { " " } else { "  " };

    let run = toolbar_label("Run", "R", compact, tiny);
    layout.run_start = column;
    draw_toolbar_button(output, &run, color, Color::Green)?;
    column += UnicodeWidthStr::width(run.as_str()) as u16;
    layout.run_end = column;

    queue!(output, Print(spacing))?;
    column += UnicodeWidthStr::width(spacing) as u16;
    let build = toolbar_label("Build", "B", compact, tiny);
    layout.build_start = column;
    draw_toolbar_button(output, &build, color, Color::Yellow)?;
    column += UnicodeWidthStr::width(build.as_str()) as u16;
    layout.build_end = column;

    queue!(output, Print(spacing))?;
    column += UnicodeWidthStr::width(spacing) as u16;
    let test = toolbar_label("Test", "T", compact, tiny);
    layout.test_start = column;
    draw_toolbar_button(output, &test, color, Color::Magenta)?;
    column += UnicodeWidthStr::width(test.as_str()) as u16;
    layout.test_end = column;

    if width.saturating_sub(column) >= 21 {
        if color {
            queue!(output, SetForegroundColor(Color::DarkGrey))?;
        } else {
            queue!(output, SetAttribute(Attribute::Dim))?;
        }
        queue!(
            output,
            Print("  mouse quick actions"),
            SetAttribute(Attribute::Reset),
            ResetColor
        )?;
    }
    Ok(())
}

fn toolbar_only(layout: Layout) -> Layout {
    Layout {
        toolbar_row: layout.toolbar_row,
        run_start: layout.run_start,
        run_end: layout.run_end,
        build_start: layout.build_start,
        build_end: layout.build_end,
        test_start: layout.test_start,
        test_end: layout.test_end,
        ..Layout::default()
    }
}

fn copy_toolbar(target: &mut Layout, toolbar: Layout) {
    target.toolbar_row = toolbar.toolbar_row;
    target.run_start = toolbar.run_start;
    target.run_end = toolbar.run_end;
    target.build_start = toolbar.build_start;
    target.build_end = toolbar.build_end;
    target.test_start = toolbar.test_start;
    target.test_end = toolbar.test_end;
}

fn toolbar_label(full: &str, short: &str, compact: bool, tiny: bool) -> String {
    if tiny {
        short.to_string()
    } else if compact {
        format!("[{short}]")
    } else {
        format!("[ {full} ]")
    }
}

fn draw_toolbar_button(
    output: &mut impl Write,
    label: &str,
    color: bool,
    background: Color,
) -> AppResult<()> {
    if color {
        queue!(
            output,
            SetForegroundColor(Color::Black),
            SetBackgroundColor(background),
            SetAttribute(Attribute::Bold),
            Print(label),
            SetAttribute(Attribute::Reset),
            ResetColor
        )?;
    } else {
        queue!(
            output,
            SetAttribute(Attribute::Reverse),
            Print(label),
            SetAttribute(Attribute::Reset)
        )?;
    }
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

fn pad_width(value: &str, width: usize) -> String {
    let padding = width.saturating_sub(UnicodeWidthStr::width(value));
    format!("{value}{}", " ".repeat(padding))
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

    #[test]
    fn padding_respects_display_width() {
        assert_eq!(pad_width("λ", 3), "λ  ");
    }

    #[test]
    fn slash_opens_a_passive_command_menu() {
        let mut menu = None;
        refresh_command_menu("/", 1, &mut menu);
        let menu = menu.expect("slash should open commands");
        assert!(menu.dimmed);
        assert_eq!(menu.candidates[0].value, "/run");
    }

    #[test]
    fn toolbar_labels_collapse_for_narrow_terminals() {
        assert_eq!(toolbar_label("Run", "R", false, false), "[ Run ]");
        assert_eq!(toolbar_label("Run", "R", true, false), "[R]");
        assert_eq!(toolbar_label("Run", "R", true, true), "R");
    }
}
