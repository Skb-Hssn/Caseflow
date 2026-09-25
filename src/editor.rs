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
use std::collections::HashMap;
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

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum WorkspaceStyle {
    #[default]
    Classic,
    Split,
}

impl WorkspaceStyle {
    pub fn label(self) -> &'static str {
        match self {
            Self::Classic => "Classic toolbar",
            Self::Split => "Split columns",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EditorAction {
    RunInteractive,
    RunClipboard,
    RepeatLast,
    AddCase,
    ToggleDebug,
    SelectText,
    More,
    ChooseCase,
    TestCase(u64),
    TestAll,
}

pub struct LineEditor {
    history: Vec<String>,
    history_path: Option<PathBuf>,
    mouse: bool,
    color: bool,
    case_ids: Vec<u64>,
    repeat_available: bool,
    context: EditorContext,
    output_lines: Vec<String>,
    scroll_offset: usize,
    warning: Option<String>,
    workspace_style: WorkspaceStyle,
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
    row: u16,
    start: u16,
    end: u16,
    action: EditorAction,
}

#[derive(Default, Clone)]
struct Layout {
    candidate_row: u16,
    candidate_start: u16,
    candidate_end: u16,
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
    repeat_available: bool,
    context: &'a EditorContext,
    output_lines: &'a [String],
    scroll_offset: usize,
    workspace_style: WorkspaceStyle,
}

struct ToolbarOptions<'a> {
    case_ids: &'a [u64],
    repeat_available: bool,
    debug: bool,
    selection_mode: bool,
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
            repeat_available: false,
            context: EditorContext::default(),
            output_lines: Vec::new(),
            scroll_offset: 0,
            warning,
            workspace_style: WorkspaceStyle::default(),
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

    pub fn set_workspace_style(&mut self, style: WorkspaceStyle) {
        self.workspace_style = style;
    }

    pub fn set_case_ids(&mut self, case_ids: Vec<u64>) {
        self.case_ids = case_ids;
    }

    pub fn set_repeat_available(&mut self, available: bool) {
        self.repeat_available = available;
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

    pub fn clear_output(&mut self) {
        self.output_lines.clear();
        self.scroll_offset = 0;
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
            let mut input_lines = vec!["Input".to_string()];
            input_lines.extend(transcript.lines().map(str::to_owned));
            input_lines.push("Output".to_string());
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
                        repeat_available: self.repeat_available,
                        context: &self.context,
                        output_lines: &self.output_lines,
                        scroll_offset: self.scroll_offset,
                        workspace_style: self.workspace_style,
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
                        match key.code {
                            KeyCode::Esc => {
                                selection_mode = false;
                                render_state.toolbar_dirty = true;
                                redraw = true;
                            }
                            KeyCode::Up | KeyCode::PageUp => {
                                let amount = if key.code == KeyCode::PageUp { 10 } else { 3 };
                                self.scroll_offset = self
                                    .scroll_offset
                                    .saturating_add(amount)
                                    .min(self.output_lines.len().saturating_sub(1));
                                redraw = true;
                            }
                            KeyCode::Down | KeyCode::PageDown => {
                                let amount = if key.code == KeyCode::PageDown { 10 } else { 3 };
                                self.scroll_offset = self.scroll_offset.saturating_sub(amount);
                                redraw = true;
                            }
                            KeyCode::Home => {
                                self.scroll_offset = self.output_lines.len().saturating_sub(1);
                                redraw = true;
                            }
                            KeyCode::End => {
                                self.scroll_offset = 0;
                                redraw = true;
                            }
                            _ => {}
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
                        // Treat the lower frame rule as part of the toolbar's
                        // hit target. Besides being easier to click, this keeps
                        // controls usable in terminals that report border-row
                        // clicks one cell below the painted label.
                        let action = layout
                            .toolbar_regions
                            .iter()
                            .find(|region| {
                                let toolbar_border = region.row == layout.toolbar_row
                                    && mouse_event.row == region.row.saturating_add(1);
                                (mouse_event.row == region.row || toolbar_border)
                                    && (region.start..region.end).contains(&mouse_event.column)
                            })
                            .map(|region| region.action);
                        if let Some(action) = action {
                            if action == EditorAction::SelectText {
                                terminal_state::enable_native_selection()?;
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
                            && (layout.candidate_start..layout.candidate_end)
                                .contains(&mouse_event.column)
                        {
                            let visible = (mouse_event.row - layout.candidate_row) as usize;
                            if let Some(active) = menu.as_mut() {
                                active.selected = active.offset + visible;
                                apply_selected(&mut buffer, &mut cursor, active)?;
                                menu = None;
                                redraw = true;
                            }
                        } else if mouse_event.row == layout.button_row {
                            if mouse_event.column >= layout.candidate_start
                                && mouse_event.column < layout.select_end
                            {
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
                Event::Mouse(mouse_event) if selection_mode => match mouse_event.kind {
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
    let split_layout = terminal_state::split_workspace_active(
        options.workspace_style == WorkspaceStyle::Split,
        width,
        height,
    );
    // The split layout trades a little horizontal room for a permanent action
    // rail. On smaller terminals it deliberately falls back to the classic
    // footer rather than squeezing either pane past usability.
    let show_toolbar = !split_layout && options.show_toolbar && height >= 6 && width >= 8;
    let footer_rows = if split_layout {
        2
    } else if show_toolbar {
        terminal_state::WORKSPACE_MOUSE_FOOTER_ROWS
    } else {
        1
    };
    let header_rows = terminal_state::WORKSPACE_HEADER_ROWS
        .min(height.saturating_sub(footer_rows).saturating_sub(1));
    let prompt_row = height - 1;
    let content_start = header_rows;
    let content_end = height.saturating_sub(footer_rows).max(content_start);
    let rail_width = if split_layout {
        terminal_state::workspace_rail_width(width)
    } else {
        0
    };
    let content_x = if split_layout {
        rail_width.saturating_add(2)
    } else {
        0
    };
    let content_width = width.saturating_sub(content_x).max(1);
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
        options.workspace_style,
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
        options.workspace_style,
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
            content_x,
            content_width,
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

    if split_layout {
        let controls_changed = state.toolbar_dirty
            || state.toolbar_size != Some((width, height))
            || state.toolbar.is_none();
        if controls_changed {
            draw_action_rail(
                &mut stderr,
                rail_width,
                content_start,
                content_end,
                options.color,
                ToolbarOptions {
                    case_ids: options.case_ids,
                    repeat_available: options.repeat_available,
                    debug: options.context.mode == "debug",
                    selection_mode: options.selection_mode,
                },
                &mut layout,
            )?;
            draw_split_footer_rule(&mut stderr, width, prompt_row, options.color)?;
            state.toolbar = Some(toolbar_only(&layout));
            state.toolbar_size = Some((width, height));
            state.toolbar_dirty = false;
        } else if let Some(toolbar) = state.toolbar.as_ref() {
            copy_toolbar(&mut layout, toolbar);
        }
    } else if show_toolbar {
        let toolbar_row = height - 3;
        let toolbar_changed = state.toolbar_dirty
            || state.toolbar_size != Some((width, height))
            || state.toolbar.is_none();
        if toolbar_changed {
            draw_toolbar(
                &mut stderr,
                width,
                toolbar_row,
                options.color,
                ToolbarOptions {
                    case_ids: options.case_ids,
                    repeat_available: options.repeat_available,
                    debug: options.context.mode == "debug",
                    selection_mode: options.selection_mode,
                },
                &mut layout,
            )?;
            draw_footer_rules(&mut stderr, width, toolbar_row, options.color)?;
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
        UnicodeWidthStr::width(" Select text · wheel/↑↓ scroll · Ctrl-Shift-C copy · Esc return")
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
        Print(" · wheel/↑↓ scroll · Ctrl-Shift-C copy · Esc return"),
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
        if row == 0 {
            draw_header_status(output, width, context, color)?;
        } else {
            if color {
                queue!(output, SetForegroundColor(theme::MUTED))?;
            }
            queue!(
                output,
                SetAttribute(Attribute::Dim),
                Print("─".repeat(width as usize)),
                SetAttribute(Attribute::Reset),
                ResetColor
            )?;
        }
    }
    Ok(())
}

fn draw_header_status(
    output: &mut impl Write,
    width: u16,
    context: &EditorContext,
    color: bool,
) -> AppResult<()> {
    let source = std::path::Path::new(&context.source)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or(&context.source);
    let left_plain = format!(" Caseflow  v{}  ·  {source}", env!("CARGO_PKG_VERSION"));
    let right = format!(
        "{}  ·  {}  ·  mouse {} ",
        context.language,
        context.mode,
        if context.mouse { "on" } else { "off" }
    );
    let available_gap = (width as usize)
        .saturating_sub(UnicodeWidthStr::width(left_plain.as_str()))
        .saturating_sub(UnicodeWidthStr::width(right.as_str()));
    if available_gap < 2 {
        let compact = truncate_width(&format!("{left_plain}  ·  {right}"), width as usize);
        if color {
            queue!(output, SetForegroundColor(theme::PRIMARY))?;
        }
        queue!(
            output,
            SetAttribute(Attribute::Bold),
            Print(compact),
            SetAttribute(Attribute::Reset),
            ResetColor,
            Clear(ClearType::UntilNewLine)
        )?;
        return Ok(());
    }
    let gap = available_gap;

    if color {
        queue!(
            output,
            Print(" "),
            SetForegroundColor(theme::PRIMARY),
            SetAttribute(Attribute::Bold),
            Print("Caseflow"),
            SetAttribute(Attribute::Reset),
            SetForegroundColor(theme::MUTED),
            Print(format!("  v{}  ·  ", env!("CARGO_PKG_VERSION"))),
            SetForegroundColor(theme::ON_ACCENT),
            SetAttribute(Attribute::Bold),
            Print(source),
            SetAttribute(Attribute::Reset),
            ResetColor,
            Print(" ".repeat(gap)),
            SetForegroundColor(theme::ON_ACCENT),
            SetAttribute(Attribute::Bold),
            Print(&context.language),
            SetAttribute(Attribute::Reset),
            SetForegroundColor(theme::MUTED),
            Print("  ·  "),
            SetForegroundColor(theme::ON_ACCENT),
            SetAttribute(Attribute::Bold),
            Print(&context.mode),
            SetAttribute(Attribute::Reset),
            SetForegroundColor(theme::MUTED),
            Print(format!(
                "  ·  mouse {} ",
                if context.mouse { "on" } else { "off" }
            )),
            ResetColor,
            Clear(ClearType::UntilNewLine)
        )?;
    } else {
        queue!(
            output,
            SetAttribute(Attribute::Bold),
            Print(truncate_width(
                &format!("{left_plain}{}{right}", " ".repeat(gap)),
                width as usize
            )),
            SetAttribute(Attribute::Reset),
            Clear(ClearType::UntilNewLine)
        )?;
    }
    Ok(())
}

fn draw_footer_rules(
    output: &mut impl Write,
    width: u16,
    toolbar_row: u16,
    color: bool,
) -> AppResult<()> {
    if toolbar_row == 0 {
        return Ok(());
    }
    queue!(output, MoveTo(0, toolbar_row - 1))?;
    if color {
        queue!(output, SetForegroundColor(theme::MUTED))?;
    }
    queue!(
        output,
        SetAttribute(Attribute::Dim),
        Print("─".repeat(width as usize)),
        MoveTo(0, toolbar_row + 1),
        Print("─".repeat(width as usize)),
        SetAttribute(Attribute::Reset),
        ResetColor
    )?;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn draw_workspace_content(
    output: &mut impl Write,
    x: u16,
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
    let case_badge_statuses = collect_case_badge_statuses(&output_lines[latest_command..line_end]);

    for index in 0..output_rows {
        let row = start + index as u16;
        clear_pane_row(output, x, row, width)?;
        let line_index = line_start + index;
        if let Some(line) = output_lines.get(line_index) {
            draw_output_line(
                output,
                line,
                width.saturating_sub(1),
                color,
                line_index < latest_command,
                &case_badge_statuses,
            )?;
        }
    }
    if offset > 0 && output_rows > 0 {
        let label = format!(" ↑ {offset} ");
        let label_width = UnicodeWidthStr::width(label.as_str()) as u16;
        if label_width < width {
            queue!(
                output,
                MoveTo(x + width - label_width, start),
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
        clear_pane_row(output, x, header_row, width)?;
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
        layout.candidate_start = x;
        layout.candidate_end = x.saturating_add(width);
        layout.visible_candidates = visible_candidates;
        for visible_index in 0..visible_candidates {
            let candidate_index = active.offset + visible_index;
            let candidate = &active.candidates[candidate_index];
            let row = layout.candidate_row + visible_index as u16;
            clear_pane_row(output, x, row, width)?;
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
        clear_pane_row(output, x, layout.button_row, width)?;
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
            layout.select_end = x + 10;
            layout.cancel_start = x + 12;
            layout.cancel_end = x + 21;
        } else {
            draw_compact_button(output, "OK", true)?;
            queue!(output, Print(" "))?;
            draw_compact_button(output, "X", false)?;
            layout.select_end = x + 4;
            layout.cancel_start = x + 5;
            layout.cancel_end = x + 8;
        }
    }
    Ok(())
}

fn clear_pane_row(output: &mut impl Write, x: u16, row: u16, width: u16) -> AppResult<()> {
    queue!(
        output,
        MoveTo(x, row),
        Print(" ".repeat(width as usize)),
        MoveTo(x, row)
    )?;
    Ok(())
}

fn draw_output_line(
    output: &mut impl Write,
    line: &str,
    width: u16,
    color: bool,
    stale: bool,
    case_badge_statuses: &HashMap<u64, CaseBadgeStatus>,
) -> AppResult<()> {
    let trimmed = line.trim_start();
    if color && !stale && trimmed.starts_with("Test summary") {
        return draw_test_summary_line(output, line, width, case_badge_statuses);
    }
    if color && !stale && trimmed.starts_with("System status") {
        return draw_system_status_line(output, line, width);
    }
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
            queue!(
                output,
                SetForegroundColor(theme::SUCCESS),
                SetAttribute(Attribute::Bold)
            )?;
        } else if trimmed.starts_with('✗') || trimmed.starts_with("Failed (exit") {
            queue!(
                output,
                SetForegroundColor(theme::DANGER),
                SetAttribute(Attribute::Bold)
            )?;
        } else if trimmed.starts_with('!') || trimmed.contains("warning:") {
            queue!(output, SetForegroundColor(theme::WARNING))?;
        } else if matches!(trimmed, "Input" | "Output" | "Expected Output") {
            queue!(
                output,
                SetForegroundColor(theme::ACCENT_ORANGE),
                SetAttribute(Attribute::Bold)
            )?;
        } else if trimmed.starts_with("━━ File") {
            queue!(
                output,
                SetForegroundColor(theme::PRIMARY),
                SetAttribute(Attribute::Bold)
            )?;
        } else if !trimmed.is_empty() && trimmed.chars().all(|character| character == '─') {
            queue!(
                output,
                SetForegroundColor(theme::MUTED),
                SetAttribute(Attribute::Dim)
            )?;
        } else if trimmed.starts_with("━━ ") {
            queue!(
                output,
                SetForegroundColor(theme::PRIMARY_SOFT),
                SetAttribute(Attribute::Bold)
            )?;
        }
    } else if line.starts_with("› ") || trimmed.starts_with("━━ ") {
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

fn draw_system_status_line(output: &mut impl Write, line: &str, width: u16) -> AppResult<()> {
    let visible = truncate_width(line, width as usize);
    let leading = &visible[..visible.len() - visible.trim_start().len()];
    let trimmed = visible.trim_start();
    let Some(result) = trimmed.strip_prefix("System status  ·  ") else {
        queue!(output, Print(visible))?;
        return Ok(());
    };
    let (status, metrics) = result
        .split_once("  ·  ")
        .map_or((result, ""), |(status, metrics)| (status, metrics));
    let failed = status.starts_with('✗');
    queue!(
        output,
        Print(leading),
        SetForegroundColor(theme::MUTED),
        Print("System status  ·  "),
        SetForegroundColor(if failed {
            theme::DANGER
        } else {
            theme::SUCCESS
        }),
        SetAttribute(Attribute::Bold),
        Print(status),
        SetAttribute(Attribute::Reset)
    )?;
    if !metrics.is_empty() {
        queue!(
            output,
            SetForegroundColor(theme::MUTED),
            Print("  ·  "),
            Print(metrics)
        )?;
    }
    queue!(output, SetAttribute(Attribute::Reset), ResetColor)?;
    Ok(())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CaseBadgeStatus {
    Passed,
    Failed,
}

fn collect_case_badge_statuses(lines: &[String]) -> HashMap<u64, CaseBadgeStatus> {
    lines
        .iter()
        .filter_map(|line| {
            let (_, verdict) = line.trim_start().split_once("Case #")?;
            let (id, result) = verdict.split_once(" verdict: ")?;
            let id = id.parse::<u64>().ok()?;
            let status = if result.starts_with("PASS") {
                CaseBadgeStatus::Passed
            } else if result.starts_with("FAIL") {
                CaseBadgeStatus::Failed
            } else {
                return None;
            };
            Some((id, status))
        })
        .collect()
}

fn draw_test_summary_line(
    output: &mut impl Write,
    line: &str,
    width: u16,
    statuses: &HashMap<u64, CaseBadgeStatus>,
) -> AppResult<()> {
    let visible = truncate_width(line, width as usize);
    let Some((prefix, badges)) = visible.rsplit_once("  ·  ") else {
        queue!(
            output,
            SetForegroundColor(theme::ON_ACCENT),
            SetAttribute(Attribute::Bold),
            Print(visible),
            SetAttribute(Attribute::Reset),
            ResetColor
        )?;
        return Ok(());
    };
    queue!(
        output,
        SetForegroundColor(theme::ON_ACCENT),
        SetAttribute(Attribute::Bold),
        Print(prefix),
        SetAttribute(Attribute::Reset),
        SetForegroundColor(theme::MUTED),
        Print("  ·  "),
        SetAttribute(Attribute::Reset)
    )?;
    for (index, badge) in badges.split_whitespace().enumerate() {
        if index > 0 {
            queue!(output, Print(" "))?;
        }
        let id = badge
            .strip_prefix('[')
            .and_then(|value| value.strip_suffix(']'))
            .and_then(|value| value.parse::<u64>().ok());
        let (color, dim) = match id.and_then(|id| statuses.get(&id)) {
            Some(CaseBadgeStatus::Passed) => (theme::SUCCESS, false),
            Some(CaseBadgeStatus::Failed) => (theme::DANGER, false),
            None => (theme::MUTED, true),
        };
        queue!(output, SetForegroundColor(color))?;
        if dim {
            queue!(output, SetAttribute(Attribute::Dim))?;
        }
        queue!(
            output,
            Print(badge),
            SetAttribute(Attribute::Reset),
            ResetColor
        )?;
    }
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

    let Some(rest) = prompt.strip_prefix(" caseflow:") else {
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
        Print("caseflow"),
        SetAttribute(Attribute::Reset),
        SetForegroundColor(theme::MUTED),
        Print(":"),
        Print(source),
        SetAttribute(Attribute::Reset),
        SetForegroundColor(theme::MUTED),
        Print(" › "),
        SetAttribute(Attribute::Reset),
        ResetColor
    )?;
    Ok(())
}

fn draw_split_footer_rule(
    output: &mut impl Write,
    width: u16,
    prompt_row: u16,
    color: bool,
) -> AppResult<()> {
    if prompt_row == 0 {
        return Ok(());
    }
    queue!(output, MoveTo(0, prompt_row - 1))?;
    if color {
        queue!(output, SetForegroundColor(theme::MUTED))?;
    }
    queue!(
        output,
        SetAttribute(Attribute::Dim),
        Print("─".repeat(width as usize)),
        SetAttribute(Attribute::Reset),
        ResetColor
    )?;
    Ok(())
}

fn draw_action_rail(
    output: &mut impl Write,
    width: u16,
    start: u16,
    end: u16,
    color: bool,
    options: ToolbarOptions<'_>,
    layout: &mut Layout,
) -> AppResult<()> {
    layout.toolbar_row = u16::MAX;
    layout.toolbar_regions.clear();
    for row in start..end {
        queue!(
            output,
            MoveTo(0, row),
            Print(" ".repeat(width.saturating_add(2) as usize))
        )?;
    }

    let run_top = start;
    let run_bottom = start + 5;
    draw_rail_box(
        output,
        width,
        run_top,
        run_bottom,
        color,
        theme::SECTION_RUN,
    )?;
    draw_rail_group_header(
        output,
        width,
        run_top,
        "▶",
        "RUN",
        Some("SPLIT"),
        color,
        theme::SECTION_RUN,
    )?;
    draw_rail_box_divider(output, width, run_top + 1, color, theme::SECTION_RUN)?;
    draw_rail_action(
        output,
        width,
        start + 2,
        "▶",
        "[Int]",
        EditorAction::RunInteractive,
        color,
        theme::SECTION_RUN,
        layout,
    )?;
    draw_rail_action(
        output,
        width,
        start + 3,
        "▣",
        "[Clip]",
        EditorAction::RunClipboard,
        color,
        theme::SECTION_RUN,
        layout,
    )?;
    draw_rail_action(
        output,
        width,
        start + 4,
        "↻",
        "[Again]",
        EditorAction::RepeatLast,
        color,
        if options.repeat_available {
            theme::PRIMARY
        } else {
            theme::SURFACE
        },
        layout,
    )?;

    let session_top = end.saturating_sub(6);
    let cases_top = start + 7;
    let cases_bottom = session_top.saturating_sub(2);
    draw_rail_box(
        output,
        width,
        cases_top,
        cases_bottom,
        color,
        theme::SECTION_CASES,
    )?;
    draw_rail_cases_header(
        output,
        width,
        cases_top,
        options.case_ids.len(),
        color,
        theme::SECTION_CASES,
        layout,
    )?;
    draw_rail_box_divider(output, width, cases_top + 1, color, theme::SECTION_CASES)?;
    draw_rail_case_grid(
        output,
        width,
        cases_top + 2,
        cases_bottom,
        options.case_ids,
        color,
        theme::SECTION_CASES,
        layout,
    )?;

    draw_rail_box(
        output,
        width,
        session_top,
        end - 1,
        color,
        theme::SECTION_SESSION,
    )?;
    draw_rail_group_header(
        output,
        width,
        session_top,
        "⚙",
        "SESSION",
        None,
        color,
        theme::SECTION_SESSION,
    )?;
    draw_rail_box_divider(
        output,
        width,
        session_top + 1,
        color,
        theme::SECTION_SESSION,
    )?;
    draw_rail_action(
        output,
        width,
        session_top + 2,
        "⚙",
        if options.debug { "[On]" } else { "[Off]" },
        EditorAction::ToggleDebug,
        color,
        if options.debug {
            theme::SUCCESS
        } else {
            theme::SURFACE
        },
        layout,
    )?;
    draw_rail_action(
        output,
        width,
        session_top + 3,
        "▤",
        "[Select]",
        EditorAction::SelectText,
        color,
        theme::SECTION_SESSION,
        layout,
    )?;
    draw_rail_action(
        output,
        width,
        session_top + 4,
        "•••",
        "[More]",
        EditorAction::More,
        color,
        theme::SECTION_SESSION,
        layout,
    )?;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn draw_rail_group_header(
    output: &mut impl Write,
    width: u16,
    row: u16,
    icon: &str,
    title: &str,
    meta: Option<&str>,
    color: bool,
    accent: Color,
) -> AppResult<()> {
    queue!(output, MoveTo(2, row), Print(" "))?;
    if color {
        queue!(output, SetForegroundColor(accent))?;
    }
    queue!(
        output,
        SetAttribute(Attribute::Bold),
        Print(icon),
        Print("  "),
        Print(title),
        Print(" "),
        SetAttribute(Attribute::Reset),
        ResetColor
    )?;
    if let Some(meta) = meta {
        let column = width.saturating_sub(UnicodeWidthStr::width(meta) as u16 + 3);
        queue!(output, MoveTo(column.saturating_sub(1), row), Print(" "))?;
        draw_rail_muted(output, column, row, meta, color)?;
        queue!(output, Print(" "))?;
    }
    Ok(())
}

fn draw_rail_cases_header(
    output: &mut impl Write,
    width: u16,
    row: u16,
    case_count: usize,
    color: bool,
    accent: Color,
    layout: &mut Layout,
) -> AppResult<()> {
    draw_rail_group_header(output, width, row, "▤", "CASES", None, color, accent)?;
    let mut column = 15_u16;
    queue!(
        output,
        MoveTo(column - 1, row),
        Print(" "),
        MoveTo(column, row)
    )?;
    let add_start = column;
    draw_rail_button(output, "[+Case]", color, accent, true)?;
    column += UnicodeWidthStr::width("[+Case]") as u16;
    layout.toolbar_regions.push(ToolbarRegion {
        row,
        start: add_start,
        end: column,
        action: EditorAction::AddCase,
    });
    queue!(output, Print("  "))?;
    column += 2;
    queue!(output, MoveTo(column, row))?;
    let all_start = column;
    draw_rail_button(output, "[All]", color, accent, false)?;
    column += UnicodeWidthStr::width("[All]") as u16;
    queue!(output, Print(" "))?;
    layout.toolbar_regions.push(ToolbarRegion {
        row,
        start: all_start,
        end: column,
        action: EditorAction::TestAll,
    });

    let meta = if case_count == 1 {
        "1 case".to_string()
    } else {
        format!("{case_count} cases")
    };
    let meta_column = width.saturating_sub(UnicodeWidthStr::width(meta.as_str()) as u16 + 3);
    if meta_column > column + 1 {
        queue!(
            output,
            MoveTo(meta_column.saturating_sub(1), row),
            Print(" ")
        )?;
        draw_rail_muted(output, meta_column, row, &meta, color)?;
        queue!(output, Print(" "))?;
    }
    Ok(())
}

fn draw_rail_muted(
    output: &mut impl Write,
    column: u16,
    row: u16,
    text: &str,
    color: bool,
) -> AppResult<()> {
    queue!(output, MoveTo(column, row))?;
    if color {
        queue!(output, SetForegroundColor(theme::MUTED))?;
    }
    queue!(
        output,
        SetAttribute(Attribute::Dim),
        Print(text),
        SetAttribute(Attribute::Reset),
        ResetColor
    )?;
    Ok(())
}

fn draw_rail_box(
    output: &mut impl Write,
    width: u16,
    top: u16,
    bottom: u16,
    color: bool,
    accent: Color,
) -> AppResult<()> {
    if bottom <= top {
        return Ok(());
    }
    if color {
        queue!(output, SetForegroundColor(accent))?;
    } else {
        queue!(output, SetAttribute(Attribute::Dim))?;
    }
    let left = 1_u16;
    let right = width.saturating_sub(1);
    queue!(
        output,
        MoveTo(left, top),
        Print("╭"),
        Print("─".repeat(right.saturating_sub(left + 1) as usize)),
        Print("╮"),
        MoveTo(left, bottom),
        Print("╰"),
        Print("─".repeat(right.saturating_sub(left + 1) as usize)),
        Print("╯")
    )?;
    for row in top + 1..bottom {
        queue!(
            output,
            MoveTo(left, row),
            Print("│"),
            MoveTo(right, row),
            Print("│")
        )?;
    }
    queue!(output, SetAttribute(Attribute::Reset), ResetColor)?;
    Ok(())
}

fn draw_rail_box_divider(
    output: &mut impl Write,
    width: u16,
    row: u16,
    color: bool,
    accent: Color,
) -> AppResult<()> {
    let left = 1_u16;
    let right = width.saturating_sub(1);
    if color {
        queue!(output, SetForegroundColor(accent))?;
    } else {
        queue!(output, SetAttribute(Attribute::Dim))?;
    }
    queue!(
        output,
        MoveTo(left, row),
        Print("├"),
        Print("─".repeat(right.saturating_sub(left + 1) as usize)),
        Print("┤"),
        SetAttribute(Attribute::Reset),
        ResetColor
    )?;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn draw_rail_action(
    output: &mut impl Write,
    width: u16,
    row: u16,
    icon: &str,
    label: &str,
    action: EditorAction,
    color: bool,
    tint: Color,
    layout: &mut Layout,
) -> AppResult<()> {
    let strong = matches!(
        action,
        EditorAction::RunInteractive | EditorAction::More | EditorAction::ChooseCase
    ) || tint == theme::SUCCESS;
    queue!(output, MoveTo(3, row))?;
    if color {
        queue!(
            output,
            SetForegroundColor(if tint == theme::SURFACE {
                theme::MUTED
            } else {
                tint
            })
        )?;
    }
    queue!(
        output,
        SetAttribute(if strong {
            Attribute::Bold
        } else {
            Attribute::Dim
        }),
        Print(icon),
        SetAttribute(Attribute::Reset),
        ResetColor
    )?;
    queue!(output, MoveTo(8, row))?;
    draw_rail_button(output, label, color, tint, strong)?;
    layout.toolbar_regions.push(ToolbarRegion {
        row,
        start: 0,
        end: width,
        action,
    });
    Ok(())
}

fn draw_rail_button(
    output: &mut impl Write,
    label: &str,
    color: bool,
    tint: Color,
    strong: bool,
) -> AppResult<()> {
    let disabled = tint == theme::SURFACE;
    if color {
        queue!(
            output,
            SetForegroundColor(if disabled { theme::MUTED } else { tint })
        )?;
    }
    if strong {
        queue!(output, SetAttribute(Attribute::Bold))?;
    } else if disabled {
        queue!(output, SetAttribute(Attribute::Dim))?;
    }
    queue!(
        output,
        Print(label),
        SetAttribute(Attribute::Reset),
        ResetColor
    )?;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn draw_rail_case_grid(
    output: &mut impl Write,
    width: u16,
    start: u16,
    end: u16,
    case_ids: &[u64],
    color: bool,
    accent: Color,
    layout: &mut Layout,
) -> AppResult<()> {
    let available_rows = end.saturating_sub(start) as usize;
    if available_rows == 0 {
        return Ok(());
    }
    if case_ids.is_empty() {
        draw_rail_muted(output, 3, start, "No saved cases", color)?;
        return Ok(());
    }
    let buttons = case_ids
        .iter()
        .map(|id| (format!("[{id}]"), EditorAction::TestCase(*id)))
        .collect::<Vec<_>>();

    let right_edge = width.saturating_sub(2);
    let required_rows = rail_grid_row_count(&buttons, right_edge);
    let has_overflow = required_rows > available_rows;
    let button_rows = if has_overflow {
        available_rows.saturating_sub(1)
    } else {
        available_rows
    };
    let mut row = start;
    let mut column = 3_u16;
    for (label, action) in &buttons {
        let label_width = UnicodeWidthStr::width(label.as_str()) as u16;
        if column > 3 && column.saturating_add(label_width) > right_edge {
            row = row.saturating_add(1);
            column = 3;
        }
        if row >= start.saturating_add(button_rows as u16) {
            break;
        }
        queue!(output, MoveTo(column, row))?;
        let button_start = column;
        draw_rail_button(output, label, color, accent, false)?;
        column += label_width;
        layout.toolbar_regions.push(ToolbarRegion {
            row,
            start: button_start,
            end: column,
            action: *action,
        });
        column += 1;
    }
    if has_overflow {
        draw_rail_action(
            output,
            width,
            end - 1,
            "…",
            "[Cases]",
            EditorAction::ChooseCase,
            color,
            accent,
            layout,
        )?;
    }
    Ok(())
}

fn rail_grid_row_count(buttons: &[(String, EditorAction)], right_edge: u16) -> usize {
    if buttons.is_empty() {
        return 0;
    }
    let mut rows = 1_usize;
    let mut column = 3_u16;
    for (label, _) in buttons {
        let label_width = UnicodeWidthStr::width(label.as_str()) as u16;
        if column > 3 && column.saturating_add(label_width) > right_edge {
            rows += 1;
            column = 3;
        }
        column = column.saturating_add(label_width + 1);
    }
    rows
}

fn draw_toolbar(
    output: &mut impl Write,
    width: u16,
    row: u16,
    color: bool,
    options: ToolbarOptions<'_>,
    layout: &mut Layout,
) -> AppResult<()> {
    layout.toolbar_row = row;
    queue!(output, MoveTo(0, row), Clear(ClearType::CurrentLine))?;
    layout.toolbar_regions.clear();
    let compact = width < 83;
    let mut column = 0_u16;

    if width < 33 {
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
            "R",
            EditorAction::RepeatLast,
            color,
            if options.repeat_available {
                theme::PRIMARY
            } else {
                theme::SURFACE
            },
            &mut column,
            layout,
        )?;
        draw_toolbar_action(
            output,
            "+",
            EditorAction::AddCase,
            color,
            theme::PRIMARY_SOFT,
            &mut column,
            layout,
        )?;
        draw_toolbar_text(output, "T", color, &mut column)?;
        draw_test_actions(
            output,
            width.saturating_sub(3),
            options.case_ids,
            ToolbarDensity::Tiny,
            color,
            theme::PRIMARY_SOFT,
            &mut column,
            layout,
        )?;
        draw_toolbar_action(
            output,
            "D",
            EditorAction::ToggleDebug,
            color,
            if options.debug {
                theme::SUCCESS
            } else {
                theme::SURFACE
            },
            &mut column,
            layout,
        )?;
        draw_toolbar_action(
            output,
            "S",
            EditorAction::SelectText,
            color,
            if options.selection_mode {
                theme::PRIMARY_SOFT
            } else {
                theme::SURFACE
            },
            &mut column,
            layout,
        )?;
        draw_toolbar_action(
            output,
            "M",
            EditorAction::More,
            color,
            theme::SURFACE,
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
            "[R]",
            EditorAction::RepeatLast,
            color,
            if options.repeat_available {
                theme::PRIMARY
            } else {
                theme::SURFACE
            },
            &mut column,
            layout,
        )?;
        draw_toolbar_text(output, " T", color, &mut column)?;
        draw_toolbar_action(
            output,
            "[+]",
            EditorAction::AddCase,
            color,
            theme::PRIMARY_SOFT,
            &mut column,
            layout,
        )?;
        draw_test_actions(
            output,
            width.saturating_sub(14),
            options.case_ids,
            ToolbarDensity::Compact,
            color,
            theme::PRIMARY_SOFT,
            &mut column,
            layout,
        )?;
        draw_toolbar_text(output, " D", color, &mut column)?;
        draw_toolbar_action(
            output,
            if options.debug { "[on]" } else { "[off]" },
            EditorAction::ToggleDebug,
            color,
            if options.debug {
                theme::SUCCESS
            } else {
                theme::SURFACE
            },
            &mut column,
            layout,
        )?;
        draw_toolbar_text(output, " ", color, &mut column)?;
        draw_toolbar_action(
            output,
            "[S]",
            EditorAction::SelectText,
            color,
            if options.selection_mode {
                theme::PRIMARY_SOFT
            } else {
                theme::SURFACE
            },
            &mut column,
            layout,
        )?;
        draw_toolbar_action(
            output,
            "[…]",
            EditorAction::More,
            color,
            theme::SURFACE,
            &mut column,
            layout,
        )?;
    } else {
        draw_toolbar_label(output, " Run  ", color, &mut column)?;
        draw_toolbar_action(
            output,
            "[Int]",
            EditorAction::RunInteractive,
            color,
            theme::PRIMARY,
            &mut column,
            layout,
        )?;
        draw_toolbar_text(output, " ", color, &mut column)?;
        draw_toolbar_action(
            output,
            "[Clip]",
            EditorAction::RunClipboard,
            color,
            theme::PRIMARY,
            &mut column,
            layout,
        )?;
        draw_toolbar_text(output, " ", color, &mut column)?;
        draw_toolbar_action(
            output,
            "[Again]",
            EditorAction::RepeatLast,
            color,
            if options.repeat_available {
                theme::PRIMARY
            } else {
                theme::SURFACE
            },
            &mut column,
            layout,
        )?;
        let wide = width >= 120;
        if wide {
            pad_toolbar_to(output, width.saturating_mul(27) / 100, &mut column)?;
            draw_toolbar_separator(output, color, &mut column)?;
            draw_toolbar_label(output, " Test  ", color, &mut column)?;
        } else {
            draw_toolbar_text(output, "   Test  ", color, &mut column)?;
        }
        draw_toolbar_action(
            output,
            "[+Case]",
            EditorAction::AddCase,
            color,
            theme::PRIMARY_SOFT,
            &mut column,
            layout,
        )?;
        draw_toolbar_text(output, " ", color, &mut column)?;
        draw_test_actions(
            output,
            if wide {
                width.saturating_mul(63) / 100
            } else {
                width.saturating_sub(32)
            },
            options.case_ids,
            ToolbarDensity::Full,
            color,
            theme::PRIMARY_SOFT,
            &mut column,
            layout,
        )?;
        if wide {
            pad_toolbar_to(output, width.saturating_mul(63) / 100, &mut column)?;
            draw_toolbar_separator(output, color, &mut column)?;
            draw_toolbar_label(output, " Debug ", color, &mut column)?;
        } else {
            draw_toolbar_text(output, "   Debug ", color, &mut column)?;
        }
        draw_toolbar_action(
            output,
            if options.debug { "[On]" } else { "[Off]" },
            EditorAction::ToggleDebug,
            color,
            if options.debug {
                theme::SUCCESS
            } else {
                theme::SURFACE
            },
            &mut column,
            layout,
        )?;
        if wide {
            pad_toolbar_to(output, width.saturating_sub(32), &mut column)?;
            draw_toolbar_separator(output, color, &mut column)?;
            draw_toolbar_text(output, " ", color, &mut column)?;
        } else {
            draw_toolbar_text(output, "   ", color, &mut column)?;
        }
        draw_toolbar_action(
            output,
            "[Select]",
            EditorAction::SelectText,
            color,
            if options.selection_mode {
                theme::PRIMARY_SOFT
            } else {
                theme::SURFACE
            },
            &mut column,
            layout,
        )?;
        if wide {
            pad_toolbar_to(output, width.saturating_sub(12), &mut column)?;
        } else {
            draw_toolbar_text(output, " ", color, &mut column)?;
        }
        draw_toolbar_action(
            output,
            "[More]",
            EditorAction::More,
            color,
            theme::PRIMARY,
            &mut column,
            layout,
        )?;
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn draw_test_actions(
    output: &mut impl Write,
    width: u16,
    case_ids: &[u64],
    density: ToolbarDensity,
    color: bool,
    background: Color,
    column: &mut u16,
    layout: &mut Layout,
) -> AppResult<()> {
    let all_label = match density {
        ToolbarDensity::Tiny => "A",
        ToolbarDensity::Compact => "[A]",
        ToolbarDensity::Full => "[All]",
    };
    let all_width = UnicodeWidthStr::width(all_label) as u16;
    let picker_label = match density {
        ToolbarDensity::Tiny => "…",
        ToolbarDensity::Compact | ToolbarDensity::Full => "[…]",
    };
    let picker_width = UnicodeWidthStr::width(picker_label) as u16;
    let mut omitted = false;
    for (index, id) in case_ids.iter().enumerate() {
        let label = match density {
            ToolbarDensity::Tiny => id.to_string(),
            ToolbarDensity::Compact => format!("[{id}]"),
            ToolbarDensity::Full => format!("[{id}]"),
        };
        let needed = UnicodeWidthStr::width(label.as_str()) as u16 + 1;
        let remaining_indicator = if index + 1 < case_ids.len() {
            picker_width + 1
        } else {
            0
        };
        if column
            .saturating_add(needed)
            .saturating_add(remaining_indicator)
            .saturating_add(all_width)
            > width
        {
            omitted = true;
            break;
        }
        draw_toolbar_action(
            output,
            &label,
            EditorAction::TestCase(*id),
            color,
            theme::SURFACE,
            column,
            layout,
        )?;
        draw_toolbar_text(output, " ", color, column)?;
    }
    if omitted
        && column
            .saturating_add(picker_width)
            .saturating_add(1)
            .saturating_add(all_width)
            <= width
    {
        draw_toolbar_action(
            output,
            picker_label,
            EditorAction::ChooseCase,
            color,
            theme::PRIMARY,
            column,
            layout,
        )?;
        draw_toolbar_text(output, " ", color, column)?;
    }
    if column.saturating_add(all_width) <= width {
        draw_toolbar_action(
            output,
            all_label,
            EditorAction::TestAll,
            color,
            if color { theme::ON_ACCENT } else { background },
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

fn draw_toolbar_label(
    output: &mut impl Write,
    text: &str,
    color: bool,
    column: &mut u16,
) -> AppResult<()> {
    if color {
        queue!(output, SetForegroundColor(theme::ON_ACCENT))?;
    }
    queue!(output, Print(text), ResetColor)?;
    *column = column.saturating_add(UnicodeWidthStr::width(text) as u16);
    Ok(())
}

fn draw_toolbar_separator(output: &mut impl Write, color: bool, column: &mut u16) -> AppResult<()> {
    draw_toolbar_text(output, "│", color, column)
}

fn pad_toolbar_to(output: &mut impl Write, target: u16, column: &mut u16) -> AppResult<()> {
    if *column >= target {
        return Ok(());
    }
    let padding = target - *column;
    queue!(output, Print(" ".repeat(padding as usize)))?;
    *column = target;
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
        row: layout.toolbar_row,
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
    tint: Color,
) -> AppResult<()> {
    if color {
        let disabled = tint == theme::SURFACE;
        queue!(
            output,
            SetForegroundColor(if disabled { theme::MUTED } else { tint })
        )?;
        if !disabled {
            queue!(output, SetAttribute(Attribute::Bold))?;
        }
        queue!(
            output,
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
    fn case_badge_colors_are_derived_from_case_verdicts() {
        let lines = vec![
            "  ✓ Case #1 verdict: PASS · matches main.out1".to_string(),
            "  ✗ Case #2 verdict: FAIL · output differs".to_string(),
        ];
        let statuses = collect_case_badge_statuses(&lines);
        assert_eq!(statuses.get(&1), Some(&CaseBadgeStatus::Passed));
        assert_eq!(statuses.get(&2), Some(&CaseBadgeStatus::Failed));
        assert_eq!(statuses.get(&3), None);
    }

    #[test]
    fn split_case_grid_wraps_instead_of_dropping_buttons() {
        let buttons = (1..=8)
            .map(|id| (format!("[{id}]"), EditorAction::TestCase(id)))
            .collect::<Vec<_>>();
        assert_eq!(rail_grid_row_count(&buttons, 41), 1);
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
