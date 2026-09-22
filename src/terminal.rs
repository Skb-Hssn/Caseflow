use crate::error::{AppError, AppResult};
use crate::theme;
use crossterm::cursor::{Hide, MoveTo, MoveToColumn, Show};
use crossterm::event::{self, Event, KeyCode, KeyEventKind, MouseButton, MouseEventKind};
use crossterm::style::{
    Attribute, Color, Print, ResetColor, SetAttribute, SetBackgroundColor, SetForegroundColor,
};
use crossterm::terminal::{
    self, Clear, ClearType, EnterAlternateScreen, LeaveAlternateScreen, ScrollUp,
};
use crossterm::{execute, queue};
use std::io::{self, IsTerminal, Write};
use std::os::fd::{FromRawFd, RawFd};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::thread;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

struct TerminalGuard;

pub struct ScreenGuard;

/// Temporarily returns an interactive session to the terminal's primary
/// screen. Dropping the guard restores a fresh alternate screen for the next
/// workspace render.
#[must_use = "keep the suspension alive while the child owns the terminal"]
pub struct ScreenSuspension {
    resume: bool,
}

static ALTERNATE_SCREEN_ACTIVE: AtomicBool = AtomicBool::new(false);
static OUTPUT_CAPTURE_ACTIVE: AtomicBool = AtomicBool::new(false);
static CAPTURED_STDERR_IS_TERMINAL: AtomicBool = AtomicBool::new(false);
static CAPTURED_INPUT: Mutex<Vec<u8>> = Mutex::new(Vec::new());

// Normal tracking reports clicks, releases, and wheel events. Crossterm's
// EnableMouseCapture also enables all-motion tracking (1003), which floods an
// interactive editor with events whenever the pointer moves.
const ENABLE_MOUSE_CLICKS: &str = "\x1b[?1007l\x1b[?1000h\x1b[?1006h";
const ENABLE_NATIVE_SELECTION: &str = "\x1b[?1006l\x1b[?1003l\x1b[?1002l\x1b[?1000l\x1b[?1007h";
const DISABLE_MOUSE_TRACKING: &str = "\x1b[?1007l\x1b[?1006l\x1b[?1003l\x1b[?1002l\x1b[?1000l";
const LIVE_OUTPUT_BYTE_LIMIT: usize = 16 * 1024;
// A short terminal should not make ordinary program output look excessive.
// This remains low enough to stop tight short-line print loops promptly.
const LIVE_OUTPUT_LINE_LIMIT: usize = 256;
const FINAL_OUTPUT_LIMIT: usize = 8 * 1024;
const OUTPUT_SUPPRESSED_NOTICE: &[u8] =
    b"\n  ! Output limit reached; further live output is suppressed. Press Ctrl-C to stop.\n";
const FINAL_OUTPUT_NOTICE: &[u8] = b"\n  ... final output ...\n";
pub const WORKSPACE_HEADER_ROWS: u16 = 3;

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

impl ScreenGuard {
    pub fn enter() -> AppResult<Self> {
        execute!(
            io::stderr(),
            EnterAlternateScreen,
            Clear(ClearType::All),
            MoveTo(0, 0),
            Show
        )?;
        ALTERNATE_SCREEN_ACTIVE.store(true, Ordering::Release);
        Ok(Self)
    }
}

impl Drop for ScreenGuard {
    fn drop(&mut self) {
        restore_screen();
    }
}

impl Drop for ScreenSuspension {
    fn drop(&mut self) {
        if !self.resume || std::thread::panicking() {
            return;
        }
        let mut stderr = io::stderr();
        if execute!(stderr, EnterAlternateScreen).is_ok() {
            ALTERNATE_SCREEN_ACTIVE.store(true, Ordering::Release);
            let _ = execute!(stderr, Clear(ClearType::All), MoveTo(0, 0), Show);
        }
    }
}

/// Leave run-cli's alternate screen while a terminal-owning child such as an
/// editor is active. Outside an interactive session this is intentionally a
/// no-op, allowing the same child-launching code to serve one-shot commands.
pub fn suspend_screen() -> AppResult<ScreenSuspension> {
    if !ALTERNATE_SCREEN_ACTIVE.load(Ordering::Acquire) {
        return Ok(ScreenSuspension { resume: false });
    }

    restore_terminal();
    execute!(
        io::stderr(),
        Print("\x1b[r"),
        Show,
        SetAttribute(Attribute::Reset),
        ResetColor,
        LeaveAlternateScreen
    )?;
    ALTERNATE_SCREEN_ACTIVE.store(false, Ordering::Release);
    Ok(ScreenSuspension { resume: true })
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

pub fn restore_terminal_and_screen() {
    restore_terminal();
    restore_screen();
}

fn restore_screen() {
    if ALTERNATE_SCREEN_ACTIVE.swap(false, Ordering::AcqRel) {
        let _ = execute!(io::stderr(), Print("\x1b[r"), LeaveAlternateScreen, Show);
    }
}

pub fn enable_mouse_capture() -> AppResult<()> {
    execute!(io::stderr(), Print(ENABLE_MOUSE_CLICKS))?;
    Ok(())
}

pub fn disable_mouse_capture() -> AppResult<()> {
    execute!(io::stderr(), Print(DISABLE_MOUSE_TRACKING))?;
    Ok(())
}

pub fn enable_native_selection() -> AppResult<()> {
    execute!(io::stderr(), Print(ENABLE_NATIVE_SELECTION))?;
    Ok(())
}

pub fn stderr_is_terminal() -> bool {
    if OUTPUT_CAPTURE_ACTIVE.load(Ordering::Acquire) {
        CAPTURED_STDERR_IS_TERMINAL.load(Ordering::Acquire)
    } else {
        io::stderr().is_terminal()
    }
}

pub fn output_capture_active() -> bool {
    OUTPUT_CAPTURE_ACTIVE.load(Ordering::Acquire)
}

pub fn record_interactive_input(input: &[u8]) {
    if output_capture_active() {
        if let Ok(mut captured) = CAPTURED_INPUT.lock() {
            captured.extend_from_slice(input);
        }
    }
}

pub fn begin_workspace_output(mouse: bool) -> AppResult<()> {
    let (width, height) = terminal::size().unwrap_or((80, 24));
    let footer_rows = if mouse && height >= 4 && width >= 8 {
        2
    } else {
        1
    };
    let header_rows =
        WORKSPACE_HEADER_ROWS.min(height.saturating_sub(footer_rows).saturating_sub(1));
    let top = header_rows.saturating_add(1);
    let bottom = height.saturating_sub(footer_rows).max(top).min(height);
    execute!(
        io::stderr(),
        Print(format!("\x1b[{top};{bottom}r")),
        MoveTo(0, bottom.saturating_sub(1)),
        Clear(ClearType::CurrentLine),
        Show
    )?;
    Ok(())
}

pub fn end_workspace_output() -> AppResult<()> {
    execute!(io::stderr(), Print("\x1b[r"))?;
    Ok(())
}

pub struct CapturedOutput {
    pub output: Vec<u8>,
    pub input: Vec<u8>,
}

pub fn capture_output<T>(
    mouse: bool,
    action: impl FnOnce() -> T,
) -> AppResult<(T, CapturedOutput)> {
    let _capture_state = CaptureStateGuard::enter(io::stderr().is_terminal());
    let mut pipe_fds = [0; 2];
    if unsafe { libc::pipe(pipe_fds.as_mut_ptr()) } != 0 {
        return Err(io::Error::last_os_error().into());
    }
    let saved_stdout = duplicate_fd(libc::STDOUT_FILENO)?;
    let saved_stderr = match duplicate_fd(libc::STDERR_FILENO) {
        Ok(fd) => fd,
        Err(error) => {
            unsafe {
                libc::close(pipe_fds[0]);
                libc::close(pipe_fds[1]);
                libc::close(saved_stdout);
            }
            return Err(error);
        }
    };
    let forward_fd = match duplicate_fd(libc::STDERR_FILENO) {
        Ok(fd) => fd,
        Err(error) => {
            unsafe {
                libc::close(pipe_fds[0]);
                libc::close(pipe_fds[1]);
                libc::close(saved_stdout);
                libc::close(saved_stderr);
            }
            return Err(error);
        }
    };
    let redirect = OutputRedirect::install(pipe_fds[1], saved_stdout, saved_stderr)?;
    let reader_fd = pipe_fds[0];
    let (width, height) = terminal::size().unwrap_or((80, 24));
    let footer_rows = if mouse && height >= 4 && width >= 8 {
        2
    } else {
        1
    };
    let header_rows =
        WORKSPACE_HEADER_ROWS.min(height.saturating_sub(footer_rows).saturating_sub(1));
    let visible_line_limit = height
        .saturating_sub(footer_rows)
        .saturating_sub(header_rows)
        .saturating_sub(1)
        .max(1) as usize;
    let live_line_limit = visible_line_limit.max(LIVE_OUTPUT_LINE_LIMIT);
    let reader = thread::spawn(move || capture_and_forward(reader_fd, forward_fd, live_line_limit));
    let result = action();
    let _ = io::stdout().flush();
    let _ = io::stderr().flush();
    drop(redirect);
    let output = reader
        .join()
        .map_err(|_| AppError::new("output capture thread panicked"))??;
    let input = CAPTURED_INPUT
        .lock()
        .map(|mut captured| std::mem::take(&mut *captured))
        .unwrap_or_default();
    Ok((result, CapturedOutput { output, input }))
}

struct CaptureStateGuard {
    previous_active: bool,
    previous_terminal: bool,
}

impl CaptureStateGuard {
    fn enter(stderr_is_terminal: bool) -> Self {
        let previous_terminal =
            CAPTURED_STDERR_IS_TERMINAL.swap(stderr_is_terminal, Ordering::AcqRel);
        let previous_active = OUTPUT_CAPTURE_ACTIVE.swap(true, Ordering::AcqRel);
        if !previous_active {
            if let Ok(mut captured) = CAPTURED_INPUT.lock() {
                captured.clear();
            }
        }
        Self {
            previous_active,
            previous_terminal,
        }
    }
}

impl Drop for CaptureStateGuard {
    fn drop(&mut self) {
        CAPTURED_STDERR_IS_TERMINAL.store(self.previous_terminal, Ordering::Release);
        OUTPUT_CAPTURE_ACTIVE.store(self.previous_active, Ordering::Release);
    }
}

struct OutputRedirect {
    saved_stdout: RawFd,
    saved_stderr: RawFd,
}

impl OutputRedirect {
    fn install(write_fd: RawFd, saved_stdout: RawFd, saved_stderr: RawFd) -> AppResult<Self> {
        if unsafe { libc::dup2(write_fd, libc::STDOUT_FILENO) } < 0 {
            let error = io::Error::last_os_error();
            unsafe {
                libc::close(write_fd);
                libc::close(saved_stdout);
                libc::close(saved_stderr);
            }
            return Err(error.into());
        }
        if unsafe { libc::dup2(write_fd, libc::STDERR_FILENO) } < 0 {
            let error = io::Error::last_os_error();
            unsafe {
                libc::dup2(saved_stdout, libc::STDOUT_FILENO);
                libc::close(write_fd);
                libc::close(saved_stdout);
                libc::close(saved_stderr);
            }
            return Err(error.into());
        }
        unsafe {
            libc::close(write_fd);
        }
        Ok(Self {
            saved_stdout,
            saved_stderr,
        })
    }
}

impl Drop for OutputRedirect {
    fn drop(&mut self) {
        unsafe {
            libc::dup2(self.saved_stdout, libc::STDOUT_FILENO);
            libc::dup2(self.saved_stderr, libc::STDERR_FILENO);
            libc::close(self.saved_stdout);
            libc::close(self.saved_stderr);
        }
    }
}

fn duplicate_fd(fd: RawFd) -> AppResult<RawFd> {
    let duplicated = unsafe { libc::dup(fd) };
    if duplicated < 0 {
        Err(io::Error::last_os_error().into())
    } else {
        Ok(duplicated)
    }
}

fn capture_and_forward(
    reader_fd: RawFd,
    forward_fd: RawFd,
    live_line_limit: usize,
) -> AppResult<Vec<u8>> {
    let mut reader = unsafe { std::fs::File::from_raw_fd(reader_fd) };
    let mut forward = unsafe { std::fs::File::from_raw_fd(forward_fd) };
    let mut captured = Vec::new();
    let mut final_output = Vec::new();
    let mut output_suppressed = false;
    let mut live_lines = 0_usize;
    let mut chunk = [0_u8; 8192];
    loop {
        let count = std::io::Read::read(&mut reader, &mut chunk)?;
        if count == 0 {
            break;
        }
        let chunk = &chunk[..count];
        if !output_suppressed {
            let visible = live_output_prefix_len(
                chunk,
                LIVE_OUTPUT_BYTE_LIMIT.saturating_sub(captured.len()),
                live_line_limit.saturating_sub(live_lines),
            );
            if visible > 0 {
                captured.extend_from_slice(&chunk[..visible]);
                forward.write_all(&chunk[..visible])?;
                forward.flush()?;
                live_lines += chunk[..visible]
                    .iter()
                    .filter(|byte| **byte == b'\n')
                    .count();
            }
            if visible < chunk.len() {
                output_suppressed = true;
                captured.extend_from_slice(OUTPUT_SUPPRESSED_NOTICE);
                forward.write_all(OUTPUT_SUPPRESSED_NOTICE)?;
                forward.flush()?;
                append_output_tail(&mut final_output, &chunk[visible..]);
            }
        } else {
            append_output_tail(&mut final_output, chunk);
        }
    }
    if output_suppressed && !final_output.is_empty() {
        trim_partial_line(&mut final_output);
        captured.extend_from_slice(FINAL_OUTPUT_NOTICE);
        captured.extend_from_slice(&final_output);
        forward.write_all(FINAL_OUTPUT_NOTICE)?;
        forward.write_all(&final_output)?;
        forward.flush()?;
    }
    Ok(captured)
}

fn live_output_prefix_len(chunk: &[u8], byte_limit: usize, line_limit: usize) -> usize {
    if byte_limit == 0 || line_limit == 0 {
        return 0;
    }
    let maximum = chunk.len().min(byte_limit);
    let mut lines = 0_usize;
    for (index, byte) in chunk[..maximum].iter().enumerate() {
        if *byte == b'\n' {
            lines += 1;
            if lines == line_limit {
                return index + 1;
            }
        }
    }
    maximum
}

fn append_output_tail(tail: &mut Vec<u8>, chunk: &[u8]) {
    if chunk.len() >= FINAL_OUTPUT_LIMIT {
        tail.clear();
        tail.extend_from_slice(&chunk[chunk.len() - FINAL_OUTPUT_LIMIT..]);
        return;
    }
    let overflow = tail
        .len()
        .saturating_add(chunk.len())
        .saturating_sub(FINAL_OUTPUT_LIMIT);
    if overflow > 0 {
        tail.drain(..overflow);
    }
    tail.extend_from_slice(chunk);
}

fn trim_partial_line(output: &mut Vec<u8>) {
    if let Some(newline) = output.iter().position(|byte| *byte == b'\n') {
        output.drain(..=newline);
    }
}

pub fn select_source(sources: &[PathBuf], mouse: bool, color: bool) -> AppResult<Option<PathBuf>> {
    if sources.is_empty() {
        return Ok(None);
    }
    if !io::stdin().is_terminal() || !stderr_is_terminal() {
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

/// Show a compact action menu inside the workspace output area.
///
/// The returned index refers to `items`. Escape, `q`, or the Close button
/// dismisses the menu without choosing an action.
pub fn select_menu(
    title: &str,
    items: &[(&str, &str)],
    mouse: bool,
    color: bool,
) -> AppResult<Option<usize>> {
    if items.is_empty() {
        return Ok(None);
    }
    if !io::stdin().is_terminal() || !stderr_is_terminal() {
        return Ok(None);
    }
    let _guard = TerminalGuard::enter(mouse)?;
    let mut selected = 0_usize;
    let mut offset = 0_usize;
    let mut redraw = true;
    let mut layout = ActionMenuLayout::default();
    loop {
        if redraw {
            layout = render_action_menu(title, items, selected, &mut offset, mouse, color)?;
            redraw = false;
        }
        match event::read()? {
            Event::Key(key) if matches!(key.kind, KeyEventKind::Press | KeyEventKind::Repeat) => {
                match key.code {
                    KeyCode::Up => {
                        selected = selected.saturating_sub(1);
                        redraw = true;
                    }
                    KeyCode::Down => {
                        selected = (selected + 1).min(items.len() - 1);
                        redraw = true;
                    }
                    KeyCode::Home => {
                        selected = 0;
                        redraw = true;
                    }
                    KeyCode::End => {
                        selected = items.len() - 1;
                        redraw = true;
                    }
                    KeyCode::Enter | KeyCode::Char(' ') => {
                        clear_action_menu(layout.start_row, layout.end_row)?;
                        return Ok(Some(selected));
                    }
                    KeyCode::Esc | KeyCode::Char('q') => {
                        clear_action_menu(layout.start_row, layout.end_row)?;
                        return Ok(None);
                    }
                    KeyCode::Char('c') if key.modifiers.contains(event::KeyModifiers::CONTROL) => {
                        clear_action_menu(layout.start_row, layout.end_row)?;
                        return Ok(None);
                    }
                    _ => {}
                }
            }
            Event::Mouse(mouse_event) if mouse => match mouse_event.kind {
                MouseEventKind::Down(MouseButton::Left) => {
                    if mouse_event.row >= layout.candidate_row
                        && mouse_event.row < layout.candidate_row + layout.visible_candidates as u16
                    {
                        let visible = (mouse_event.row - layout.candidate_row) as usize;
                        let chosen = offset + visible;
                        clear_action_menu(layout.start_row, layout.end_row)?;
                        return Ok(Some(chosen));
                    }
                    if mouse_event.row == layout.close_row
                        && (layout.close_start..layout.close_end).contains(&mouse_event.column)
                    {
                        clear_action_menu(layout.start_row, layout.end_row)?;
                        return Ok(None);
                    }
                }
                MouseEventKind::ScrollUp => {
                    selected = selected.saturating_sub(1);
                    redraw = true;
                }
                MouseEventKind::ScrollDown => {
                    selected = (selected + 1).min(items.len() - 1);
                    redraw = true;
                }
                _ => {}
            },
            Event::Resize(_, _) => redraw = true,
            _ => {}
        }
    }
}

#[derive(Default, Clone, Copy)]
struct ActionMenuLayout {
    start_row: u16,
    candidate_row: u16,
    visible_candidates: usize,
    close_row: u16,
    close_start: u16,
    close_end: u16,
    end_row: u16,
}

fn render_action_menu(
    title: &str,
    items: &[(&str, &str)],
    selected: usize,
    offset: &mut usize,
    mouse: bool,
    color: bool,
) -> AppResult<ActionMenuLayout> {
    let (width, height) = terminal::size().unwrap_or((80, 24));
    let width = width.max(1);
    let height = height.max(1);
    let footer_rows = if mouse && height >= 4 && width >= 8 {
        2
    } else {
        1
    };
    let start_row = WORKSPACE_HEADER_ROWS.min(height.saturating_sub(footer_rows).saturating_sub(2));
    let available = height.saturating_sub(footer_rows).saturating_sub(start_row) as usize;
    let show_close = available >= 3;
    let reserved_rows = 1 + usize::from(show_close);
    let visible = items
        .len()
        .min(available.saturating_sub(reserved_rows).max(1))
        .min(10);
    if selected < *offset {
        *offset = selected;
    } else if selected >= offset.saturating_add(visible) {
        *offset = selected + 1 - visible;
    }
    let candidate_row = start_row + 1;
    let candidate_end = candidate_row + visible.saturating_sub(1) as u16;
    let close_row = if show_close {
        candidate_end + 1
    } else {
        u16::MAX
    };
    let close_label = "[ Close ]";
    let close_end = if show_close {
        UnicodeWidthStr::width(close_label) as u16
    } else {
        0
    };
    let end_row = if show_close { close_row } else { candidate_end };
    let mut stderr = io::stderr();
    for row in start_row..=end_row {
        queue!(stderr, MoveTo(0, row), Clear(ClearType::CurrentLine))?;
    }
    queue!(stderr, MoveTo(0, start_row))?;
    let title = truncate_width(title, width as usize);
    let title_width = UnicodeWidthStr::width(title.as_str());
    let hint = truncate_width(
        "  ·  ↑↓ choose · Enter select · Esc close",
        width as usize - title_width,
    );
    if color {
        queue!(stderr, SetForegroundColor(theme::PRIMARY))?;
    }
    queue!(
        stderr,
        SetAttribute(Attribute::Bold),
        Print(title),
        SetAttribute(Attribute::Reset),
        ResetColor,
        SetAttribute(Attribute::Dim),
        Print(hint),
        SetAttribute(Attribute::Reset),
        Clear(ClearType::UntilNewLine)
    )?;
    for visible_index in 0..visible {
        let item_index = *offset + visible_index;
        let (label, description) = items[item_index];
        let row = candidate_row + visible_index as u16;
        let is_selected = item_index == selected;
        queue!(stderr, MoveTo(0, row))?;
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
        let text = truncate_width(
            &format!(" {marker} {label}  ·  {description}"),
            width.saturating_sub(1) as usize,
        );
        queue!(
            stderr,
            Print(if is_selected {
                pad_width(&text, width.saturating_sub(1) as usize)
            } else {
                text
            }),
            SetAttribute(Attribute::Reset),
            ResetColor,
            Clear(ClearType::UntilNewLine)
        )?;
    }
    if show_close {
        queue!(stderr, MoveTo(0, close_row))?;
        draw_tinted_button(&mut stderr, "Close", false, false, color, theme::SURFACE)?;
    }
    stderr.flush()?;
    Ok(ActionMenuLayout {
        start_row,
        candidate_row,
        visible_candidates: visible,
        close_row,
        close_start: 0,
        close_end,
        end_row,
    })
}

fn clear_action_menu(start_row: u16, end_row: u16) -> AppResult<()> {
    let mut stderr = io::stderr();
    for row in start_row..=end_row {
        queue!(stderr, MoveTo(0, row), Clear(ClearType::CurrentLine))?;
    }
    stderr.flush()?;
    Ok(())
}

pub fn confirm(prompt: &str, action: &str, mouse: bool, color: bool) -> AppResult<bool> {
    if !io::stdin().is_terminal() || !stderr_is_terminal() {
        return Ok(false);
    }
    let _guard = TerminalGuard::enter(mouse)?;
    let (_, row) = crossterm::cursor::position().unwrap_or((0, 0));
    // Reaching this dialog already requires an explicit destructive command.
    // Keep the requested action focused so Enter confirms it; Esc and `n`
    // remain immediate cancellation shortcuts.
    let mut selected = 0_usize;
    let mut redraw = true;
    let mut layout = ConfirmationLayout::default();
    loop {
        if redraw {
            layout = render_confirmation(prompt, action, selected, row, color)?;
            redraw = false;
        }
        match event::read()? {
            Event::Key(key) if key.kind == KeyEventKind::Press => match key.code {
                KeyCode::Tab | KeyCode::Left | KeyCode::Right => {
                    selected = 1 - selected;
                    redraw = true;
                }
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
                if mouse_event.row == layout.button_row {
                    if let MouseEventKind::Down(MouseButton::Left) = mouse_event.kind {
                        if (layout.action_start..layout.action_end).contains(&mouse_event.column) {
                            clear_from(row)?;
                            return Ok(true);
                        }
                        if (layout.cancel_start..layout.cancel_end).contains(&mouse_event.column) {
                            clear_from(row)?;
                            return Ok(false);
                        }
                    }
                }
            }
            Event::Resize(_, _) => redraw = true,
            _ => {}
        }
    }
}

#[derive(Default)]
struct ConfirmationLayout {
    button_row: u16,
    action_start: u16,
    action_end: u16,
    cancel_start: u16,
    cancel_end: u16,
}

fn render_confirmation(
    prompt: &str,
    action: &str,
    selected: usize,
    row: u16,
    color: bool,
) -> AppResult<ConfirmationLayout> {
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
    Ok(ConfirmationLayout {
        button_row,
        action_start,
        action_end,
        cancel_start,
        cancel_end,
    })
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

    #[test]
    fn live_output_is_limited_by_visible_lines_and_bytes() {
        assert_eq!(live_output_prefix_len(b"one\ntwo\nthree\n", 100, 2), 8);
        assert_eq!(live_output_prefix_len(b"abcdef", 3, 10), 3);
        assert_eq!(live_output_prefix_len(b"one\n", 100, 0), 0);
    }
}
