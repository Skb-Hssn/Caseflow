use crate::error::{AppError, AppResult};
use crate::suggest::{self, Completion};
use crate::terminal as terminal_state;
use crate::theme;
use crossterm::cursor::{MoveTo, Show};
use crossterm::event::{
    self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEventKind,
};
use crossterm::style::{
    Attribute, Color, Print, ResetColor, SetAttribute, SetBackgroundColor, SetForegroundColor,
};
use crossterm::terminal::{self, Clear, ClearType};
use crossterm::{execute, queue};
use std::fs::{self, OpenOptions};
use std::hash::{DefaultHasher, Hash, Hasher};
use std::io::{self, IsTerminal, Write};
use std::path::PathBuf;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

pub enum EditorSignal {
    Success(String),
    Action(EditorAction),
    CtrlC,
    CtrlD,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EditorAction {
    RunInteractive,
    RunClipboard,
    Build,
    SelectText,
    TestCase(u64),
    TestAll,
}

pub struct LineEditor {
    history: Vec<String>,
    history_path: Option<PathBuf>,
    mouse: bool,
    color: bool,
    case_ids: Vec<u64>,
    context: EditorContext,
    output_lines: Vec<String>,
    scroll_offset: usize,
    warning: Option<String>,
}

#[derive(Default)]
struct EditorContext {
    source: String,
    language: String,
    mode: String,
    mouse: bool,
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

#[derive(Clone, Copy)]
struct ToolbarRegion {
    start: u16,
    end: u16,
    action: EditorAction,
}

#[derive(Default, Clone)]
struct Layout {
    candidate_row: u16,
    visible_candidates: usize,
    button_row: u16,
    select_end: u16,
    cancel_start: u16,
    cancel_end: u16,
    toolbar_row: u16,
    toolbar_regions: Vec<ToolbarRegion>,
}

#[derive(Default)]
struct RenderState {
    frame_size: Option<(u16, u16)>,
    content_signature: u64,
    header_signature: u64,
    toolbar: Option<Layout>,
    toolbar_size: Option<(u16, u16)>,
    toolbar_dirty: bool,
}

#[derive(Clone, Copy)]
struct RenderOptions<'a> {
    show_toolbar: bool,
    selection_mode: bool,
    color: bool,
    case_ids: &'a [u64],
    context: &'a EditorContext,
    output_lines: &'a [String],
    scroll_offset: usize,
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
            case_ids: Vec::new(),
            context: EditorContext::default(),
            output_lines: Vec::new(),
            scroll_offset: 0,
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

    pub fn set_case_ids(&mut self, case_ids: Vec<u64>) {
        self.case_ids = case_ids;
    }

    pub fn set_context(
        &mut self,
        source: impl Into<String>,
        language: impl Into<String>,
        mode: impl Into<String>,
        mouse: bool,
    ) {
        self.context = EditorContext {
            source: source.into(),
            language: language.into(),
            mode: mode.into(),
            mouse,
        };
    }

    pub fn append_output(&mut self, output: &[u8]) {
        self.append_captured_output(output, &[]);
    }

    pub fn append_captured_output(&mut self, output: &[u8], input: &[u8]) {
        let text = strip_terminal_sequences(&String::from_utf8_lossy(output));
        let mut lines = text
            .lines()
            .map(|line| line.trim_end_matches('\r').to_string())
            .collect::<Vec<_>>();
        if !input.is_empty() {
            let transcript = strip_terminal_sequences(&String::from_utf8_lossy(input));
            let insert_at = lines
                .iter()
                .rposition(|line| line.starts_with("━━ RUN · interactive input"))
                .map_or(lines.len(), |index| index + 1);
            let mut input_lines = vec!["  Input".to_string()];
            input_lines.extend(transcript.lines().map(|line| format!("    {line}")));
            input_lines.push("  Output".to_string());
            lines.splice(insert_at..insert_at, input_lines);
        }
        self.output_lines.extend(lines);
        if text.ends_with('\n') && self.output_lines.last().is_some_and(String::is_empty) {
            self.output_lines.pop();
        }
        const MAX_OUTPUT_LINES: usize = 20_000;
        if self.output_lines.len() > MAX_OUTPUT_LINES {
            self.output_lines
                .drain(..self.output_lines.len() - MAX_OUTPUT_LINES);
        }
        self.scroll_offset = 0;
    }

    pub fn begin_command(&mut self, command: &str) {
        if !self.output_lines.is_empty()
            && self
                .output_lines
                .last()
                .is_some_and(|line| !line.is_empty())
        {
            self.output_lines.push(String::new());
        }
        self.output_lines.push(format!("› {command}"));
        self.scroll_offset = 0;
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
        let mut selection_mode = false;
        let mut render_state = RenderState::default();
        let mut layout = Layout::default();
        let mut redraw = true;

        loop {
            if redraw {
                layout = render_workspace(
                    prompt,
                    &buffer,
                    cursor,
                    menu.as_mut(),
                    &mut anchor_row,
                    RenderOptions {
                        show_toolbar: self.mouse,
                        selection_mode,
                        color: self.color,
                        case_ids: &self.case_ids,
                        context: &self.context,
                        output_lines: &self.output_lines,
                        scroll_offset: self.scroll_offset,
                    },
                    &mut render_state,
                )?;
                redraw = false;
            }
            let should_capture = self.mouse && !selection_mode;
            if should_capture != mouse_capture {
                if should_capture {
                    terminal_state::enable_mouse_capture()?;
                } else {
                    terminal_state::disable_mouse_capture()?;
                }
                mouse_capture = should_capture;
            }

            match event::read()? {
                Event::Key(key)
                    if matches!(key.kind, KeyEventKind::Press | KeyEventKind::Repeat) =>
                {
                    if selection_mode {
                        if key.code == KeyCode::Esc {
                            selection_mode = false;
                            render_state.toolbar_dirty = true;
                            redraw = true;
                        }
                        continue;
                    }
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
                            terminal_state::disable_mouse_capture()?;
                        }
                        finish_line(prompt, &buffer, anchor_row, self.color)?;
                        if let EditorSignal::Success(line) = &signal {
                            self.save_history(line);
                        }
                        return Ok(signal);
                    }
                    redraw = true;
                }
                Event::Paste(value) => {
                    buffer.insert_str(cursor, &value);
                    cursor += value.len();
                    refresh_command_menu(&buffer, cursor, &mut menu);
                    redraw = true;
                }
                Event::Mouse(mouse_event) if mouse_capture => match mouse_event.kind {
                    MouseEventKind::Down(MouseButton::Left) => {
                        let action = (mouse_event.row == layout.toolbar_row)
                            .then(|| {
                                layout
                                    .toolbar_regions
                                    .iter()
                                    .find(|region| {
                                        (region.start..region.end).contains(&mouse_event.column)
                                    })
                                    .map(|region| region.action)
                            })
                            .flatten();
                        if let Some(action) = action {
                            if action == EditorAction::SelectText {
                                terminal_state::disable_mouse_capture()?;
                                mouse_capture = false;
                                selection_mode = true;
                                render_state.toolbar_dirty = true;
                                redraw = true;
                                continue;
                            }
                            terminal_state::disable_mouse_capture()?;
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
                                redraw = true;
                            }
                        } else if mouse_event.row == layout.button_row {
                            if mouse_event.column < layout.select_end {
                                if let Some(active) = menu.as_mut() {
                                    apply_selected(&mut buffer, &mut cursor, active)?;
                                }
                                menu = None;
                                redraw = true;
                            } else if (layout.cancel_start..layout.cancel_end)
                                .contains(&mouse_event.column)
                            {
                                menu = None;
                                redraw = true;
                            }
                        }
                    }
                    MouseEventKind::ScrollUp if menu.is_some() => {
                        activate_selection(menu.as_mut(), -1);
                        redraw = true;
                    }
                    MouseEventKind::ScrollDown if menu.is_some() => {
                        activate_selection(menu.as_mut(), 1);
                        redraw = true;
                    }
                    MouseEventKind::ScrollUp => {
                        self.scroll_offset = self
                            .scroll_offset
                            .saturating_add(3)
                            .min(self.output_lines.len().saturating_sub(1));
                        redraw = true;
                    }
                    MouseEventKind::ScrollDown => {
                        self.scroll_offset = self.scroll_offset.saturating_sub(3);
                        redraw = true;
                    }
                    _ => {}
                },
                Event::Resize(_, _) => {
                    render_state.toolbar_dirty = true;
                    redraw = true;
                }
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
                let has_arguments = buffer[..(*cursor).min(buffer.len())]
                    .chars()
                    .any(char::is_whitespace);
                if active.dimmed && has_arguments && active.candidates.len() > 1 {
                    active.dimmed = false;
                } else {
                    apply_selected(buffer, cursor, active)?;
                    *menu = None;
                }
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
    if before_cursor.starts_with('/') {
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

fn render_workspace(
    prompt: &str,
    buffer: &str,
    cursor: usize,
    menu: Option<&mut CompletionMenu>,
    anchor_row: &mut u16,
    options: RenderOptions<'_>,
    state: &mut RenderState,
) -> AppResult<Layout> {
    let (width, height) = terminal::size().unwrap_or((80, 24));
    let width = width.max(1);
    let height = height.max(1);
    let show_toolbar = options.show_toolbar && height >= 4;
    let footer_rows = if show_toolbar { 2 } else { 1 };
    let header_rows = terminal_state::WORKSPACE_HEADER_ROWS
        .min(height.saturating_sub(footer_rows).saturating_sub(1));
    let prompt_row = height - 1;
    let content_start = header_rows;
    let content_end = height.saturating_sub(footer_rows).max(content_start);
    *anchor_row = prompt_row;

    let size_changed = state.frame_size != Some((width, height));
    if size_changed {
        terminal_state::end_workspace_output()?;
        state.frame_size = Some((width, height));
        state.toolbar_dirty = true;
    }

    let header_signature = hash_value(&(
        width,
        height,
        options.context.source.as_str(),
        options.context.language.as_str(),
        options.context.mode.as_str(),
        options.context.mouse,
    ));
    let menu_signature = menu.as_ref().map(|active| {
        let candidates = active
            .candidates
            .iter()
            .map(|candidate| candidate.value.as_str())
            .collect::<Vec<_>>();
        (active.selected, active.offset, active.dimmed, candidates)
    });
    let content_signature = hash_value(&(
        width,
        height,
        buffer,
        options.output_lines.len(),
        options.scroll_offset,
        menu_signature,
    ));

    let mut stderr = io::stderr();
    if size_changed || state.header_signature != header_signature {
        draw_workspace_header(
            &mut stderr,
            width,
            header_rows,
            options.context,
            options.color,
        )?;
        state.header_signature = header_signature;
    }

    let mut layout = Layout::default();
    if size_changed || state.content_signature != content_signature || menu.is_some() {
        draw_workspace_content(
            &mut stderr,
            width,
            content_start,
            content_end,
            options.output_lines,
            options.scroll_offset,
            menu,
            options.color,
            &mut layout,
        )?;
        state.content_signature = content_signature;
    } else if let Some(toolbar) = state.toolbar.as_ref() {
        copy_toolbar(&mut layout, toolbar);
    }

    if show_toolbar {
        let toolbar_row = height - 2;
        let toolbar_changed = state.toolbar_dirty
            || state.toolbar_size != Some((width, height))
            || state.toolbar.is_none();
        if toolbar_changed {
            draw_toolbar(
                &mut stderr,
                width,
                toolbar_row,
                options.color,
                options.case_ids,
                options.selection_mode,
                &mut layout,
            )?;
            state.toolbar = Some(toolbar_only(&layout));
            state.toolbar_size = Some((width, height));
            state.toolbar_dirty = false;
        } else if let Some(toolbar) = state.toolbar.as_ref() {
            copy_toolbar(&mut layout, toolbar);
        }
    } else {
        state.toolbar = None;
        state.toolbar_size = None;
    }

    queue!(stderr, MoveTo(0, prompt_row), Clear(ClearType::CurrentLine))?;
    let prefix_width = if options.selection_mode {
        draw_selection_prompt(&mut stderr, options.color)?;
        UnicodeWidthStr::width(" Select text · Ctrl-Shift-C copy · Esc return")
    } else {
        draw_prompt(&mut stderr, prompt, options.color)?;
        queue!(stderr, Print(buffer), Clear(ClearType::UntilNewLine))?;
        UnicodeWidthStr::width(prompt) + UnicodeWidthStr::width(&buffer[..cursor.min(buffer.len())])
    };
    let cursor_column = (prefix_width % width as usize) as u16;
    queue!(stderr, MoveTo(cursor_column, prompt_row), Show)?;
    stderr.flush()?;
    Ok(layout)
}

fn draw_selection_prompt(output: &mut impl Write, color: bool) -> AppResult<()> {
    if color {
        queue!(output, SetForegroundColor(theme::PRIMARY_SOFT))?;
    }
    queue!(
        output,
        SetAttribute(Attribute::Bold),
        Print(" Select text"),
        SetAttribute(Attribute::Reset),
        ResetColor,
        Print(" · Ctrl-Shift-C copy · Esc return"),
        Clear(ClearType::UntilNewLine)
    )?;
    Ok(())
}

fn draw_workspace_header(
    output: &mut impl Write,
    width: u16,
    rows: u16,
    context: &EditorContext,
    color: bool,
) -> AppResult<()> {
    for row in 0..rows {
        queue!(output, MoveTo(0, row), Clear(ClearType::CurrentLine))?;
        let text = match row {
            0 => format!(
                " run-cli  v{}  ·  {}",
                env!("CARGO_PKG_VERSION"),
                context.source
            ),
            1 => format!(
                " {}  ·  {}  ·  mouse {}",
                context.language,
                context.mode,
                if context.mouse { "on" } else { "off" }
            ),
            _ => "─".repeat(width as usize),
        };
        if color && row == 0 {
            queue!(
                output,
                SetForegroundColor(theme::PRIMARY),
                SetAttribute(Attribute::Bold)
            )?;
        } else if color && row == 1 {
            queue!(output, SetForegroundColor(theme::MUTED))?;
        } else if row == 0 {
            queue!(output, SetAttribute(Attribute::Bold))?;
        } else {
            queue!(output, SetAttribute(Attribute::Dim))?;
        }
        queue!(
            output,
            Print(truncate_width(&text, width as usize)),
            SetAttribute(Attribute::Reset),
            ResetColor,
            Clear(ClearType::UntilNewLine)
        )?;
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn draw_workspace_content(
    output: &mut impl Write,
    width: u16,
    start: u16,
    end: u16,
    output_lines: &[String],
    scroll_offset: usize,
    menu: Option<&mut CompletionMenu>,
    color: bool,
    layout: &mut Layout,
) -> AppResult<()> {
    let available = end.saturating_sub(start) as usize;
    let visible_candidates = menu
        .as_ref()
        .map(|active| {
            active
                .candidates
                .len()
                .min(available.saturating_sub(2))
                .min(10)
        })
        .unwrap_or(0);
    let menu_rows = if visible_candidates > 0 {
        visible_candidates + 2
    } else {
        0
    };
    let output_rows = available.saturating_sub(menu_rows);
    let max_offset = output_lines.len().saturating_sub(output_rows);
    let offset = scroll_offset.min(max_offset);
    let line_end = output_lines.len().saturating_sub(offset);
    let line_start = line_end.saturating_sub(output_rows);
    let latest_command = output_lines
        .iter()
        .rposition(|line| line.starts_with("› "))
        .unwrap_or(0);

    for index in 0..output_rows {
        let row = start + index as u16;
        queue!(output, MoveTo(0, row), Clear(ClearType::CurrentLine))?;
        let line_index = line_start + index;
        if let Some(line) = output_lines.get(line_index) {
            draw_output_line(
                output,
                line,
                width.saturating_sub(1),
                color,
                line_index < latest_command,
            )?;
        }
    }
    if offset > 0 && output_rows > 0 {
        let label = format!(" ↑ {offset} ");
        let label_width = UnicodeWidthStr::width(label.as_str()) as u16;
        if label_width < width {
            queue!(
                output,
                MoveTo(width - label_width, start),
                SetAttribute(Attribute::Reverse),
                Print(label),
                SetAttribute(Attribute::Reset)
            )?;
        }
    }

    if let Some(active) = menu {
        if visible_candidates == 0 {
            return Ok(());
        }
        if active.selected < active.offset {
            active.offset = active.selected;
        } else if active.selected >= active.offset + visible_candidates {
            active.offset = active.selected + 1 - visible_candidates;
        }
        let header_row = start + output_rows as u16;
        queue!(output, MoveTo(0, header_row), Clear(ClearType::CurrentLine))?;
        if color {
            queue!(
                output,
                SetForegroundColor(if active.dimmed {
                    theme::MUTED
                } else {
                    theme::PRIMARY
                })
            )?;
        }
        queue!(
            output,
            SetAttribute(if active.dimmed {
                Attribute::Dim
            } else {
                Attribute::Bold
            }),
            Print(if active.dimmed {
                "  Options"
            } else {
                "  Suggestions"
            }),
            SetAttribute(Attribute::Reset),
            ResetColor,
            Print(format!("  ·  {} matches", active.candidates.len())),
            Clear(ClearType::UntilNewLine)
        )?;
        layout.candidate_row = header_row + 1;
        layout.visible_candidates = visible_candidates;
        for visible_index in 0..visible_candidates {
            let candidate_index = active.offset + visible_index;
            let candidate = &active.candidates[candidate_index];
            let row = layout.candidate_row + visible_index as u16;
            queue!(output, MoveTo(0, row), Clear(ClearType::CurrentLine))?;
            let selected = candidate_index == active.selected;
            if active.dimmed {
                queue!(output, SetAttribute(Attribute::Dim))?;
                if color {
                    queue!(output, SetForegroundColor(theme::MUTED))?;
                }
            } else if selected && color {
                queue!(
                    output,
                    SetForegroundColor(theme::ON_ACCENT),
                    SetBackgroundColor(theme::PRIMARY),
                    SetAttribute(Attribute::Bold)
                )?;
            } else if selected {
                queue!(output, SetAttribute(Attribute::Reverse))?;
            }
            let marker = if active.dimmed {
                "·"
            } else if selected {
                "›"
            } else {
                " "
            };
            let label = truncate_width(
                &format!(
                    " {marker} {}  ·  {}",
                    candidate.value, candidate.description
                ),
                width.saturating_sub(1) as usize,
            );
            queue!(
                output,
                Print(if selected && !active.dimmed {
                    pad_width(&label, width.saturating_sub(1) as usize)
                } else {
                    label
                }),
                SetAttribute(Attribute::Reset),
                ResetColor,
                Clear(ClearType::UntilNewLine)
            )?;
        }
        layout.button_row = layout.candidate_row + visible_candidates as u16;
        queue!(
            output,
            MoveTo(0, layout.button_row),
            Clear(ClearType::CurrentLine)
        )?;
        if active.dimmed {
            queue!(
                output,
                SetAttribute(Attribute::Dim),
                Print("  Tab choose · ↑↓ navigate · Esc close"),
                SetAttribute(Attribute::Reset)
            )?;
        } else if width >= 22 {
            draw_button(output, "Select", true)?;
            queue!(output, Print("  "))?;
            draw_button(output, "Close", false)?;
            layout.select_end = 10;
            layout.cancel_start = 12;
            layout.cancel_end = 21;
        } else {
            draw_compact_button(output, "OK", true)?;
            queue!(output, Print(" "))?;
            draw_compact_button(output, "X", false)?;
            layout.select_end = 4;
            layout.cancel_start = 5;
            layout.cancel_end = 8;
        }
    }
    Ok(())
}

fn draw_output_line(
    output: &mut impl Write,
    line: &str,
    width: u16,
    color: bool,
    stale: bool,
) -> AppResult<()> {
    let trimmed = line.trim_start();
    if stale {
        queue!(output, SetAttribute(Attribute::Dim))?;
        if color {
            queue!(output, SetForegroundColor(theme::MUTED))?;
        }
    } else if color {
        if line.starts_with("› ") {
            queue!(
                output,
                SetForegroundColor(theme::PRIMARY),
                SetAttribute(Attribute::Bold)
            )?;
        } else if trimmed.starts_with('✓') || trimmed.starts_with("Success (exit") {
            queue!(output, SetForegroundColor(theme::SUCCESS))?;
        } else if trimmed.starts_with('✗') || trimmed.starts_with("Failed (exit") {
            queue!(output, SetForegroundColor(theme::DANGER))?;
        } else if trimmed.starts_with('!') || trimmed.contains("warning:") {
            queue!(output, SetForegroundColor(theme::WARNING))?;
        } else if trimmed == "Input" || trimmed == "Output" {
            queue!(
                output,
                SetForegroundColor(theme::PRIMARY_SOFT),
                SetAttribute(Attribute::Bold)
            )?;
        } else if line.starts_with("━━ ") {
            queue!(
                output,
                SetForegroundColor(theme::PRIMARY_SOFT),
                SetAttribute(Attribute::Bold)
            )?;
        }
    } else if line.starts_with("› ") || line.starts_with("━━ ") {
        queue!(output, SetAttribute(Attribute::Bold))?;
    }
    queue!(
        output,
        Print(truncate_width(line, width as usize)),
        SetAttribute(Attribute::Reset),
        ResetColor
    )?;
    Ok(())
}

fn hash_value(value: &impl Hash) -> u64 {
    let mut hasher = DefaultHasher::new();
    value.hash(&mut hasher);
    hasher.finish()
}

fn finish_line(_prompt: &str, _buffer: &str, anchor_row: u16, _color: bool) -> AppResult<()> {
    execute!(
        io::stderr(),
        MoveTo(0, anchor_row),
        Clear(ClearType::CurrentLine)
    )?;
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

    let Some(rest) = prompt.strip_prefix(" run-cli:") else {
        queue!(
            output,
            SetForegroundColor(theme::PRIMARY),
            SetAttribute(Attribute::Bold),
            Print(prompt),
            SetAttribute(Attribute::Reset),
            ResetColor
        )?;
        return Ok(());
    };
    let source = rest.strip_suffix(" › ").unwrap_or(rest);
    queue!(
        output,
        Print(" "),
        SetForegroundColor(theme::PRIMARY),
        SetAttribute(Attribute::Bold),
        Print("run-cli"),
        SetAttribute(Attribute::Reset),
        SetForegroundColor(theme::MUTED),
        Print(":"),
        SetForegroundColor(theme::PRIMARY_SOFT),
        SetAttribute(Attribute::Bold),
        Print(source),
        SetAttribute(Attribute::Reset),
        SetForegroundColor(theme::PRIMARY),
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
    case_ids: &[u64],
    selection_mode: bool,
    layout: &mut Layout,
) -> AppResult<()> {
    layout.toolbar_row = row;
    queue!(output, MoveTo(0, row), Clear(ClearType::CurrentLine))?;
    layout.toolbar_regions.clear();
    let compact = width < 60;
    let mut column = 0_u16;

    if width < 18 {
        draw_toolbar_action(
            output,
            "I",
            EditorAction::RunInteractive,
            color,
            theme::PRIMARY,
            &mut column,
            layout,
        )?;
        draw_toolbar_action(
            output,
            "C",
            EditorAction::RunClipboard,
            color,
            theme::PRIMARY,
            &mut column,
            layout,
        )?;
        draw_toolbar_action(
            output,
            "B",
            EditorAction::Build,
            color,
            theme::SURFACE,
            &mut column,
            layout,
        )?;
        draw_toolbar_action(
            output,
            "S",
            EditorAction::SelectText,
            color,
            if selection_mode {
                theme::PRIMARY
            } else {
                theme::SURFACE
            },
            &mut column,
            layout,
        )?;
        draw_toolbar_text(output, "T", color, &mut column)?;
        draw_test_actions(
            output,
            width,
            case_ids,
            ToolbarDensity::Tiny,
            color,
            &mut column,
            layout,
        )?;
    } else if compact {
        draw_toolbar_text(output, " R", color, &mut column)?;
        draw_toolbar_action(
            output,
            "[I]",
            EditorAction::RunInteractive,
            color,
            theme::PRIMARY,
            &mut column,
            layout,
        )?;
        draw_toolbar_action(
            output,
            "[C]",
            EditorAction::RunClipboard,
            color,
            theme::PRIMARY,
            &mut column,
            layout,
        )?;
        draw_toolbar_action(
            output,
            "[B]",
            EditorAction::Build,
            color,
            theme::SURFACE,
            &mut column,
            layout,
        )?;
        draw_toolbar_action(
            output,
            "[S]",
            EditorAction::SelectText,
            color,
            if selection_mode {
                theme::PRIMARY
            } else {
                theme::SURFACE
            },
            &mut column,
            layout,
        )?;
        draw_toolbar_text(output, " T", color, &mut column)?;
        draw_test_actions(
            output,
            width,
            case_ids,
            ToolbarDensity::Compact,
            color,
            &mut column,
            layout,
        )?;
    } else {
        draw_toolbar_text(output, " ", color, &mut column)?;
        draw_toolbar_action(
            output,
            "[ Run ]",
            EditorAction::RunInteractive,
            color,
            theme::PRIMARY,
            &mut column,
            layout,
        )?;
        draw_toolbar_text(output, "  ", color, &mut column)?;
        draw_toolbar_action(
            output,
            "[ Clipboard ]",
            EditorAction::RunClipboard,
            color,
            theme::PRIMARY,
            &mut column,
            layout,
        )?;
        draw_toolbar_text(output, "  ", color, &mut column)?;
        draw_toolbar_action(
            output,
            "[ Build ]",
            EditorAction::Build,
            color,
            theme::SURFACE,
            &mut column,
            layout,
        )?;
        draw_toolbar_text(output, "  ", color, &mut column)?;
        draw_toolbar_action(
            output,
            "[ Select ]",
            EditorAction::SelectText,
            color,
            if selection_mode {
                theme::PRIMARY
            } else {
                theme::SURFACE
            },
            &mut column,
            layout,
        )?;
        draw_toolbar_text(output, "   Test ", color, &mut column)?;
        draw_test_actions(
            output,
            width,
            case_ids,
            ToolbarDensity::Full,
            color,
            &mut column,
            layout,
        )?;
    }
    Ok(())
}

fn draw_test_actions(
    output: &mut impl Write,
    width: u16,
    case_ids: &[u64],
    density: ToolbarDensity,
    color: bool,
    column: &mut u16,
    layout: &mut Layout,
) -> AppResult<()> {
    let all_label = match density {
        ToolbarDensity::Tiny => "A",
        ToolbarDensity::Compact => "[A]",
        ToolbarDensity::Full => "[ All ]",
    };
    let all_width = UnicodeWidthStr::width(all_label) as u16;
    let mut omitted = false;
    for id in case_ids {
        let label = match density {
            ToolbarDensity::Tiny => id.to_string(),
            ToolbarDensity::Compact => format!("[{id}]"),
            ToolbarDensity::Full => format!("[ {id} ]"),
        };
        let needed = UnicodeWidthStr::width(label.as_str()) as u16 + 1;
        if column.saturating_add(needed).saturating_add(all_width) > width {
            omitted = true;
            break;
        }
        draw_toolbar_action(
            output,
            &label,
            EditorAction::TestCase(*id),
            color,
            theme::PRIMARY,
            column,
            layout,
        )?;
        draw_toolbar_text(output, " ", color, column)?;
    }
    if omitted && column.saturating_add(2).saturating_add(all_width) <= width {
        draw_toolbar_text(output, "… ", color, column)?;
    }
    if column.saturating_add(all_width) <= width {
        draw_toolbar_action(
            output,
            all_label,
            EditorAction::TestAll,
            color,
            theme::PRIMARY,
            column,
            layout,
        )?;
    }
    Ok(())
}

#[derive(Clone, Copy)]
enum ToolbarDensity {
    Tiny,
    Compact,
    Full,
}

fn draw_toolbar_text(
    output: &mut impl Write,
    text: &str,
    color: bool,
    column: &mut u16,
) -> AppResult<()> {
    if color {
        queue!(output, SetForegroundColor(theme::MUTED))?;
    } else {
        queue!(output, SetAttribute(Attribute::Dim))?;
    }
    queue!(
        output,
        Print(text),
        SetAttribute(Attribute::Reset),
        ResetColor
    )?;
    *column = column.saturating_add(UnicodeWidthStr::width(text) as u16);
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn draw_toolbar_action(
    output: &mut impl Write,
    label: &str,
    action: EditorAction,
    color: bool,
    background: Color,
    column: &mut u16,
    layout: &mut Layout,
) -> AppResult<()> {
    let start = *column;
    draw_toolbar_button(output, label, color, background)?;
    *column = column.saturating_add(UnicodeWidthStr::width(label) as u16);
    layout.toolbar_regions.push(ToolbarRegion {
        start,
        end: *column,
        action,
    });
    Ok(())
}

fn toolbar_only(layout: &Layout) -> Layout {
    Layout {
        toolbar_row: layout.toolbar_row,
        toolbar_regions: layout.toolbar_regions.clone(),
        ..Layout::default()
    }
}

fn copy_toolbar(target: &mut Layout, toolbar: &Layout) {
    target.toolbar_row = toolbar.toolbar_row;
    target.toolbar_regions.clone_from(&toolbar.toolbar_regions);
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
            SetForegroundColor(theme::ON_ACCENT),
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

fn strip_terminal_sequences(value: &str) -> String {
    let mut result = String::with_capacity(value.len());
    let mut characters = value.chars().peekable();
    while let Some(character) = characters.next() {
        if character == '\x1b' {
            match characters.next() {
                Some('[') => {
                    for part in characters.by_ref() {
                        if ('@'..='~').contains(&part) {
                            break;
                        }
                    }
                }
                Some(']') => {
                    let mut escaped = false;
                    for part in characters.by_ref() {
                        if part == '\x07' || (escaped && part == '\\') {
                            break;
                        }
                        escaped = part == '\x1b';
                    }
                }
                Some(_) | None => {}
            }
        } else if character == '\r' {
            if characters.peek() != Some(&'\n') {
                result.push('\n');
            }
        } else if character == '\n' || character == '\t' || !character.is_control() {
            result.push(character);
        }
    }
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
    fn captured_output_discards_terminal_control_sequences() {
        assert_eq!(
            strip_terminal_sequences("\x1b[31merror\x1b[0m\rprogress\n"),
            "error\nprogress\n"
        );
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
    fn run_options_open_as_a_passive_menu() {
        let mut menu = None;
        refresh_command_menu("/run ", 5, &mut menu);
        let menu = menu.expect("run options should open");
        assert!(menu.dimmed);
        assert!(menu
            .candidates
            .iter()
            .any(|candidate| candidate.value == "clipboard"));
    }
}
