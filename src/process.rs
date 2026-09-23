use crate::error::{AppError, AppResult};
use crate::model::{OutputTarget, RunReport, RunRequest};
use crate::terminal;
use crate::ui::Ui;
use std::fs::{self, File, OpenOptions};
use std::io::{self, IsTerminal, Read, Write};
use std::os::fd::{AsRawFd, FromRawFd};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicU64, Ordering};
use std::time::{Duration, Instant};

static ACTIVE_PROCESS_GROUP: AtomicI32 = AtomicI32::new(0);
static INTERRUPT_REQUESTED: AtomicBool = AtomicBool::new(false);
static INTERRUPT_SCOPE_ACTIVE: AtomicBool = AtomicBool::new(false);
static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

extern "C" fn forward_signal(signal: libc::c_int) {
    if signal == libc::SIGINT {
        INTERRUPT_REQUESTED.store(true, Ordering::Relaxed);
    }
    let process_group = ACTIVE_PROCESS_GROUP.load(Ordering::Relaxed);
    if process_group > 0 {
        unsafe {
            libc::kill(-process_group, signal);
        }
    }
}

struct SignalGuard {
    old_int: libc::sighandler_t,
    old_term: libc::sighandler_t,
}

impl SignalGuard {
    fn install(process_group: i32) -> Self {
        if !INTERRUPT_SCOPE_ACTIVE.load(Ordering::SeqCst) {
            INTERRUPT_REQUESTED.store(false, Ordering::SeqCst);
        }
        ACTIVE_PROCESS_GROUP.store(process_group, Ordering::SeqCst);
        if INTERRUPT_REQUESTED.load(Ordering::SeqCst) {
            unsafe {
                libc::kill(-process_group, libc::SIGINT);
            }
        }
        unsafe {
            Self {
                old_int: libc::signal(libc::SIGINT, forward_signal as libc::sighandler_t),
                old_term: libc::signal(libc::SIGTERM, forward_signal as libc::sighandler_t),
            }
        }
    }
}

impl Drop for SignalGuard {
    fn drop(&mut self) {
        ACTIVE_PROCESS_GROUP.store(0, Ordering::SeqCst);
        if !INTERRUPT_SCOPE_ACTIVE.load(Ordering::SeqCst) {
            INTERRUPT_REQUESTED.store(false, Ordering::SeqCst);
        }
        unsafe {
            libc::signal(libc::SIGINT, self.old_int);
            libc::signal(libc::SIGTERM, self.old_term);
        }
    }
}

pub struct InterruptScope {
    old_int: libc::sighandler_t,
}

impl InterruptScope {
    pub fn install() -> Self {
        INTERRUPT_REQUESTED.store(false, Ordering::SeqCst);
        INTERRUPT_SCOPE_ACTIVE.store(true, Ordering::SeqCst);
        let old_int = unsafe { libc::signal(libc::SIGINT, forward_signal as libc::sighandler_t) };
        Self { old_int }
    }

    pub fn requested(&self) -> bool {
        INTERRUPT_REQUESTED.load(Ordering::SeqCst)
    }
}

impl Drop for InterruptScope {
    fn drop(&mut self) {
        ACTIVE_PROCESS_GROUP.store(0, Ordering::SeqCst);
        INTERRUPT_SCOPE_ACTIVE.store(false, Ordering::SeqCst);
        INTERRUPT_REQUESTED.store(false, Ordering::SeqCst);
        unsafe {
            libc::signal(libc::SIGINT, self.old_int);
        }
    }
}

struct ForegroundGuard {
    terminal_fd: i32,
    parent_group: i32,
    old_ttou: libc::sighandler_t,
    active: bool,
}

impl ForegroundGuard {
    fn give_to(process_group: i32) -> Self {
        let terminal_fd = libc::STDIN_FILENO;
        let parent_group = unsafe { libc::getpgrp() };
        let old_ttou = unsafe { libc::signal(libc::SIGTTOU, libc::SIG_IGN) };
        let active = unsafe { libc::tcsetpgrp(terminal_fd, process_group) == 0 };
        Self {
            terminal_fd,
            parent_group,
            old_ttou,
            active,
        }
    }
}

impl Drop for ForegroundGuard {
    fn drop(&mut self) {
        unsafe {
            if self.active {
                libc::tcsetpgrp(self.terminal_fd, self.parent_group);
            }
            libc::signal(libc::SIGTTOU, self.old_ttou);
        }
    }
}

pub fn run(request: &RunRequest, ui: &Ui) -> AppResult<RunReport> {
    let mut input_capture = match &request.capture_input {
        Some(path) => Some(File::create(path).map_err(|error| {
            AppError::new(format!(
                "cannot capture input to '{}': {error}",
                path.display()
            ))
        })?),
        None => None,
    };
    let mut command = Command::new(&request.command.program);
    command.args(&request.command.args);
    let retain_interactive_input = terminal::output_capture_active()
        && request.input.is_none()
        && request.capture_input.is_none()
        && io::stdin().is_terminal();
    let proxy_input = request.capture_input.is_some() || retain_interactive_input;
    let mut retained_input = Vec::new();

    if let Some(input) = &request.input {
        let file = File::open(input).map_err(|error| {
            AppError::new(format!("cannot open input '{}': {error}", input.display()))
        })?;
        command.stdin(Stdio::from(file));
    } else if proxy_input {
        command.stdin(Stdio::piped());
    } else {
        command.stdin(Stdio::inherit());
    }

    let mut atomic_output = None;
    let mut output_reader = None;
    let mut tee_output = None;
    match &request.output {
        OutputTarget::Inherit => {
            if terminal::output_capture_active() {
                let (master, slave) = open_output_pty()?;
                let stderr = slave.try_clone().map_err(|error| {
                    AppError::new(format!("cannot duplicate output terminal: {error}"))
                })?;
                command.stdout(Stdio::from(slave));
                command.stderr(Stdio::from(stderr));
                output_reader = Some(master);
            } else {
                command.stdout(Stdio::inherit());
                command.stderr(Stdio::inherit());
            }
        }
        OutputTarget::TeeFile(path) => {
            let file = File::create(path).map_err(|error| {
                AppError::new(format!(
                    "cannot create captured output '{}': {error}",
                    path.display()
                ))
            })?;
            if terminal::output_capture_active() {
                let (master, slave) = open_output_pty()?;
                command.stdout(Stdio::from(slave));
                command.stderr(Stdio::inherit());
                output_reader = Some(master);
                tee_output = Some(file);
            } else {
                command.stdout(Stdio::piped());
                command.stderr(Stdio::inherit());
                tee_output = Some(file);
            }
        }
        OutputTarget::File(path) => {
            let file = File::create(path).map_err(|error| {
                AppError::new(format!(
                    "cannot create output '{}': {error}",
                    path.display()
                ))
            })?;
            command.stdout(Stdio::from(file));
            command.stderr(Stdio::inherit());
        }
        OutputTarget::AtomicFile(path) => {
            let (temporary, file) = create_atomic_temp(path)?;
            command.stdout(Stdio::from(file));
            command.stderr(Stdio::inherit());
            atomic_output = Some((temporary, path.clone()));
        }
    }
    let owns_terminal = request.input.is_none() && !proxy_input && io::stdin().is_terminal();

    unsafe {
        command.pre_exec(move || {
            libc::signal(libc::SIGINT, libc::SIG_DFL);
            libc::signal(libc::SIGTERM, libc::SIG_DFL);
            if libc::setpgid(0, 0) != 0 {
                return Err(io::Error::last_os_error());
            }
            if owns_terminal {
                let old_ttou = libc::signal(libc::SIGTTOU, libc::SIG_IGN);
                let foreground_status = libc::tcsetpgrp(libc::STDIN_FILENO, libc::getpid());
                libc::signal(libc::SIGTTOU, old_ttou);
                if foreground_status != 0 {
                    return Err(io::Error::last_os_error());
                }
            }
            Ok(())
        });
    }

    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(error) => {
            if let Some((temporary, _)) = &atomic_output {
                let _ = fs::remove_file(temporary);
            }
            return Err(AppError::new(format!(
                "could not start '{}': {error}",
                request.command.program.to_string_lossy()
            )));
        }
    };
    drop(command);
    let pid = child.id() as i32;
    let _signal_guard = SignalGuard::install(pid);
    let _foreground_guard = owns_terminal.then(|| ForegroundGuard::give_to(pid));
    let output_reader = if let Some(reader) = output_reader {
        let tee = tee_output.take();
        Some(std::thread::spawn(move || drain_output(reader, tee)))
    } else if let Some(tee) = tee_output.take() {
        let reader = child
            .stdout
            .take()
            .expect("tee output must configure piped stdout");
        Some(std::thread::spawn(move || drain_output(reader, Some(tee))))
    } else {
        None
    };

    let mut child_stdin = child.stdin.take();
    let started = Instant::now();
    let mut termination_started = None;
    let mut interrupt_started = None;
    let mut interrupt_term_sent = false;
    let mut interrupt_kill_sent = false;
    let mut timed_out = false;
    let mut raw_status = 0;
    let mut usage: libc::rusage = unsafe { std::mem::zeroed() };

    loop {
        let waited = unsafe { libc::wait4(pid, &mut raw_status, libc::WNOHANG, &mut usage) };
        if waited == pid {
            break;
        }
        if waited < 0 {
            let error = io::Error::last_os_error();
            if error.kind() != io::ErrorKind::Interrupted {
                cleanup_atomic(&atomic_output);
                return Err(AppError::new(format!(
                    "could not wait for child process: {error}"
                )));
            }
        }

        if let Some(stdin) = child_stdin.as_mut() {
            let keep_open = if let Some(capture) = input_capture.as_mut() {
                pump_stdin(stdin, capture)?
            } else if retain_interactive_input {
                pump_stdin(stdin, &mut retained_input)?
            } else {
                true
            };
            if !keep_open {
                child_stdin = None;
                input_capture = None;
            }
        } else {
            std::thread::sleep(Duration::from_millis(10));
        }

        if let Some(limit) = request.timeout {
            if !timed_out && started.elapsed() >= limit {
                timed_out = true;
                termination_started = Some(Instant::now());
                unsafe {
                    libc::kill(-pid, libc::SIGTERM);
                }
            }
        }
        if INTERRUPT_REQUESTED.load(Ordering::Relaxed) && interrupt_started.is_none() {
            interrupt_started = Some(Instant::now());
        }
        if let Some(interrupt) = interrupt_started {
            let elapsed = interrupt.elapsed();
            if !interrupt_term_sent && elapsed >= Duration::from_millis(350) {
                unsafe {
                    libc::kill(-pid, libc::SIGTERM);
                }
                interrupt_term_sent = true;
            }
            if !interrupt_kill_sent && elapsed >= Duration::from_secs(1) {
                unsafe {
                    libc::kill(-pid, libc::SIGKILL);
                }
                interrupt_kill_sent = true;
            }
        }
        if let Some(termination) = termination_started {
            if termination.elapsed() >= Duration::from_secs(1) {
                unsafe {
                    libc::kill(-pid, libc::SIGKILL);
                }
                termination_started = None;
            }
        }
    }

    if let Some(reader) = output_reader {
        reader
            .join()
            .map_err(|_| AppError::new("output reader thread panicked"))??;
    }
    drop(child_stdin);
    drop(input_capture);
    if retain_interactive_input {
        terminal::record_interactive_input(&retained_input);
    }
    let interrupted = interrupt_started.is_some()
        || INTERRUPT_REQUESTED.load(Ordering::SeqCst)
        || (libc::WIFSIGNALED(raw_status) && libc::WTERMSIG(raw_status) == libc::SIGINT);
    let exit_code = if interrupted {
        130
    } else if timed_out {
        124
    } else {
        decode_wait_status(raw_status)
    };
    let report = RunReport {
        exit_code,
        interrupted,
        wall_time: started.elapsed(),
        peak_memory_kib: usage.ru_maxrss,
        timed_out,
    };

    if let Some((temporary, destination)) = atomic_output {
        if report.success() {
            fs::rename(&temporary, &destination).map_err(|error| {
                let _ = fs::remove_file(&temporary);
                AppError::new(format!(
                    "cannot save output '{}': {error}",
                    destination.display()
                ))
            })?;
            ui.success(format!("Saved output  {}", destination.display()));
        } else {
            let _ = fs::remove_file(temporary);
        }
    }
    if request.show_report {
        ui.report(&report);
    }
    Ok(report)
}

fn open_output_pty() -> AppResult<(File, File)> {
    let mut master = -1;
    let mut slave = -1;
    let (columns, rows) = crossterm::terminal::size().unwrap_or((80, 24));
    let size = libc::winsize {
        ws_row: rows,
        ws_col: columns,
        ws_xpixel: 0,
        ws_ypixel: 0,
    };
    let result = unsafe {
        libc::openpty(
            &mut master,
            &mut slave,
            std::ptr::null_mut(),
            std::ptr::null(),
            &size,
        )
    };
    if result != 0 {
        return Err(AppError::new(format!(
            "cannot create output terminal: {}",
            io::Error::last_os_error()
        )));
    }
    let master = unsafe { File::from_raw_fd(master) };
    let slave = unsafe { File::from_raw_fd(slave) };
    let mut settings: libc::termios = unsafe { std::mem::zeroed() };
    if unsafe { libc::tcgetattr(slave.as_raw_fd(), &mut settings) } != 0 {
        return Err(AppError::new(format!(
            "cannot inspect output terminal: {}",
            io::Error::last_os_error()
        )));
    }
    settings.c_oflag &= !(libc::OPOST as libc::tcflag_t);
    if unsafe { libc::tcsetattr(slave.as_raw_fd(), libc::TCSANOW, &settings) } != 0 {
        return Err(AppError::new(format!(
            "cannot configure output terminal: {}",
            io::Error::last_os_error()
        )));
    }
    Ok((master, slave))
}

fn drain_output(mut reader: impl Read, mut tee: Option<File>) -> AppResult<()> {
    let mut output = io::stdout();
    let mut chunk = [0_u8; 8192];
    loop {
        match reader.read(&mut chunk) {
            Ok(0) => break,
            Ok(count) => {
                if let Some(file) = tee.as_mut() {
                    file.write_all(&chunk[..count])?;
                }
                output.write_all(&chunk[..count])?;
                output.flush()?;
            }
            Err(error) if error.raw_os_error() == Some(libc::EIO) => break,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(error.into()),
        }
    }
    if let Some(file) = tee.as_mut() {
        file.flush()?;
    }
    Ok(())
}

fn pump_stdin(child: &mut impl Write, capture: &mut impl Write) -> AppResult<bool> {
    let mut poll_fd = libc::pollfd {
        fd: libc::STDIN_FILENO,
        events: libc::POLLIN,
        revents: 0,
    };
    let result = unsafe { libc::poll(&mut poll_fd, 1, 10) };
    if result < 0 {
        let error = io::Error::last_os_error();
        if error.kind() == io::ErrorKind::Interrupted {
            return Ok(true);
        }
        return Err(error.into());
    }
    if result == 0 {
        return Ok(true);
    }
    if poll_fd.revents & libc::POLLIN == 0 && poll_fd.revents & (libc::POLLHUP | libc::POLLERR) != 0
    {
        return Ok(false);
    }
    let mut buffer = [0_u8; 4096];
    let count = io::stdin().read(&mut buffer)?;
    if count == 0 {
        return Ok(false);
    }
    capture.write_all(&buffer[..count])?;
    capture.flush()?;
    match child.write_all(&buffer[..count]) {
        Ok(()) => {
            child.flush()?;
            Ok(true)
        }
        Err(error) if error.kind() == io::ErrorKind::BrokenPipe => Ok(false),
        Err(error) => Err(error.into()),
    }
}

fn create_atomic_temp(destination: &Path) -> AppResult<(PathBuf, File)> {
    let parent = destination.parent().unwrap_or_else(|| Path::new("."));
    let name = destination
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| AppError::new(format!("invalid output path '{}'", destination.display())))?;
    for _ in 0..100 {
        let count = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
        let temporary = parent.join(format!(
            ".{name}.run-cli.{}.{count}.tmp",
            std::process::id()
        ));
        match OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)
        {
            Ok(file) => return Ok((temporary, file)),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => {
                return Err(AppError::new(format!(
                    "cannot create temporary output beside '{}': {error}",
                    destination.display()
                )))
            }
        }
    }
    Err(AppError::new(format!(
        "cannot allocate temporary output beside '{}'",
        destination.display()
    )))
}

fn cleanup_atomic(output: &Option<(PathBuf, PathBuf)>) {
    if let Some((temporary, _)) = output {
        let _ = fs::remove_file(temporary);
    }
}

fn decode_wait_status(status: i32) -> i32 {
    if libc::WIFEXITED(status) {
        libc::WEXITSTATUS(status)
    } else if libc::WIFSIGNALED(status) {
        128 + libc::WTERMSIG(status)
    } else {
        1
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::CommandSpec;

    #[test]
    fn runs_a_successful_process() {
        let output = std::env::temp_dir().join(format!("run-cli-output-{}", std::process::id()));
        let request = RunRequest {
            command: CommandSpec::new("sh").args(["-c", "printf hello"]),
            input: None,
            output: OutputTarget::AtomicFile(output.clone()),
            timeout: None,
            capture_input: None,
            show_report: false,
        };
        let report = run(&request, &Ui::new(crate::config::ColorPolicy::Never)).unwrap();
        assert!(report.success());
        assert_eq!(fs::read_to_string(&output).unwrap(), "hello");
        let _ = fs::remove_file(output);
    }

    #[test]
    fn timeout_returns_124() {
        let request = RunRequest {
            command: CommandSpec::new("sh").args(["-c", "sleep 2"]),
            input: None,
            output: OutputTarget::File(PathBuf::from("/dev/null")),
            timeout: Some(Duration::from_millis(20)),
            capture_input: None,
            show_report: false,
        };
        let report = run(&request, &Ui::new(crate::config::ColorPolicy::Never)).unwrap();
        assert_eq!(report.exit_code, 124);
        assert!(report.timed_out);
    }
}
