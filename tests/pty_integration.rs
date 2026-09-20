#![cfg(unix)]

use portable_pty::{native_pty_system, Child, CommandBuilder, MasterPty, PtySize};
use std::fs;
use std::io::{Read, Write};
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};
use tempfile::TempDir;

fn binary() -> &'static str {
    env!("CARGO_BIN_EXE_run-cli")
}

struct PtySession {
    master: Box<dyn MasterPty + Send>,
    child: Box<dyn Child + Send + Sync>,
    writer: Arc<Mutex<Box<dyn Write + Send>>>,
    output: Arc<Mutex<Vec<u8>>>,
}

impl PtySession {
    fn spawn(mut command: CommandBuilder) -> Self {
        command.env("TERM", "xterm-256color");
        let pair = native_pty_system()
            .openpty(PtySize {
                rows: 24,
                cols: 100,
                pixel_width: 0,
                pixel_height: 0,
            })
            .unwrap();
        let child = pair.slave.spawn_command(command).unwrap();
        drop(pair.slave);
        let mut reader = pair.master.try_clone_reader().unwrap();
        let writer = Arc::new(Mutex::new(pair.master.take_writer().unwrap()));
        let output = Arc::new(Mutex::new(Vec::new()));
        let thread_writer = Arc::clone(&writer);
        let thread_output = Arc::clone(&output);
        thread::spawn(move || {
            let mut buffer = [0_u8; 4096];
            let mut pending = Vec::new();
            loop {
                let Ok(count) = reader.read(&mut buffer) else {
                    break;
                };
                if count == 0 {
                    break;
                }
                thread_output
                    .lock()
                    .unwrap()
                    .extend_from_slice(&buffer[..count]);
                pending.extend_from_slice(&buffer[..count]);
                if pending.windows(4).any(|window| window == b"\x1b[6n") {
                    let mut writer = thread_writer.lock().unwrap();
                    writer.write_all(b"\x1b[1;1R").unwrap();
                    writer.flush().unwrap();
                    pending.clear();
                } else if pending.len() > 16 {
                    pending.drain(..pending.len() - 16);
                }
            }
        });
        Self {
            master: pair.master,
            child,
            writer,
            output,
        }
    }

    fn send(&self, bytes: &[u8]) {
        let mut writer = self.writer.lock().unwrap();
        writer.write_all(bytes).unwrap();
        writer.flush().unwrap();
    }

    fn resize(&self, rows: u16, cols: u16) {
        self.master
            .resize(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .unwrap();
    }

    fn text(&self) -> String {
        String::from_utf8_lossy(&self.output.lock().unwrap()).into_owned()
    }

    fn wait_for(&self, needle: &str) {
        let deadline = Instant::now() + Duration::from_secs(10);
        while Instant::now() < deadline {
            if self.text().contains(needle) {
                return;
            }
            thread::sleep(Duration::from_millis(20));
        }
        panic!("PTY output did not contain {needle:?}:\n{}", self.text());
    }

    fn wait(mut self) -> portable_pty::ExitStatus {
        self.child.wait().unwrap()
    }
}

fn source(directory: &Path, body: &str) -> std::path::PathBuf {
    let path = directory.join("main.py");
    fs::write(&path, body).unwrap();
    path
}

fn repl_command(directory: &TempDir, source: &Path, mouse: bool) -> CommandBuilder {
    let mut command = CommandBuilder::new(binary());
    command.cwd(directory.path());
    command.arg("--color");
    command.arg("never");
    if mouse {
        command.arg("--mouse");
    }
    command.arg(source);
    command.env("XDG_CACHE_HOME", directory.path().join("cache"));
    command.env("XDG_CONFIG_HOME", directory.path().join("config"));
    command.env("XDG_STATE_HOME", directory.path().join("state"));
    command
}

#[test]
fn repl_handles_resize_and_restores_terminal() {
    let directory = TempDir::new().unwrap();
    let source = source(directory.path(), "print('ready')\n");
    let mut command = CommandBuilder::new("/bin/sh");
    command.cwd(directory.path());
    command.env("RUNCLI_BIN", binary());
    command.env("RUNCLI_SOURCE", &source);
    command.env("XDG_CACHE_HOME", directory.path().join("cache"));
    command.env("XDG_CONFIG_HOME", directory.path().join("config"));
    command.env("XDG_STATE_HOME", directory.path().join("state"));
    command.arg("-c");
    command.arg(
        "before=$(stty -g); \"$RUNCLI_BIN\" --color never \"$RUNCLI_SOURCE\"; code=$?; \
         after=$(stty -g); test \"$before\" = \"$after\" && echo RESTORED=yes; exit $code",
    );
    let session = PtySession::spawn(command);
    session.wait_for("run-cli:main.py");
    assert!(session.text().contains("\x1b[?1049h"));
    session.resize(8, 32);
    session.send(b"/status\r");
    session.wait_for("Language");
    session.resize(30, 120);
    session.send(b"/quit\r");
    session.wait_for("RESTORED=yes");
    let output = session.text();
    let leave_screen = output
        .find("\x1b[?1049l")
        .expect("alternate screen should be restored");
    let restored = output.find("RESTORED=yes").unwrap();
    assert!(leave_screen < restored);
    assert!(session.wait().success());
}

#[test]
fn ctrl_c_interrupts_child_and_returns_to_repl() {
    let directory = TempDir::new().unwrap();
    let source = source(
        directory.path(),
        "import time\nprint('child-started', flush=True)\ntime.sleep(30)\n",
    );
    let session = PtySession::spawn(repl_command(&directory, &source, false));
    session.wait_for("run-cli:main.py");
    session.send(b"/run\r");
    session.wait_for("child-started");
    session.send(b"\x03");
    session.wait_for("Failed (exit 130)");
    session.send(b"/quit\r");
    assert!(session.wait().success());
}

#[test]
fn mouse_click_selects_a_file_completion() {
    let directory = TempDir::new().unwrap();
    let source = source(
        directory.path(),
        "a, b = map(int, input().split())\nprint(a + b)\n",
    );
    fs::write(directory.path().join("input-one.txt"), "2 3\n").unwrap();
    fs::write(directory.path().join("input-two.txt"), "7 8\n").unwrap();
    let session = PtySession::spawn(repl_command(&directory, &source, true));
    session.wait_for("run-cli:main.py");
    session.send(b"/run --input input-\t");
    session.wait_for("[ Select ]");
    // The prompt and suggestion header occupy rows 1-2; the first item is row 3.
    session.send(b"\x1b[<0;2;3M");
    session.send(b"\r");
    session.wait_for("\r\n5\r\n");
    session.wait_for("Success (exit 0)");
    session.send(b"/quit\r");
    assert!(session.wait().success());
}

#[test]
fn slash_opens_dimmed_commands_and_tab_accepts_one() {
    let directory = TempDir::new().unwrap();
    let source = source(directory.path(), "print('ready')\n");
    let session = PtySession::spawn(repl_command(&directory, &source, false));
    session.wait_for("run-cli:main.py");
    session.send(b"/");
    session.wait_for("Commands");
    session.send(b"\t");
    session.wait_for("\x1b[0m/run \x1b");
    session.send(b"\x03");
    session.send(b"/quit\r");
    assert!(session.wait().success());
}

#[test]
fn persistent_mouse_toolbar_runs_the_active_source() {
    let directory = TempDir::new().unwrap();
    let source = source(directory.path(), "print('toolbar-run')\n");
    let session = PtySession::spawn(repl_command(&directory, &source, true));
    session.wait_for("[ Interactive ]");
    // The toolbar is fixed to row 24; this lands inside Interactive.
    session.send(b"\x1b[<0;13;24M");
    session.wait_for("toolbar-run");
    session.wait_for("Success (exit 0)");
    session.send(b"/quit\r");
    assert!(session.wait().success());
}

#[test]
fn mouse_toolbar_is_not_redrawn_while_typing() {
    let directory = TempDir::new().unwrap();
    let source = source(directory.path(), "print('ready')\n");
    let session = PtySession::spawn(repl_command(&directory, &source, true));
    session.wait_for("[ Interactive ]");
    assert!(
        !session.text().contains("\x1b[?1003h"),
        "mouse mode must not enable all-motion tracking"
    );
    assert_eq!(session.text().matches("run-cli:main.py").count(), 1);
    // An unsolicited pointer-motion report must not repaint any editor row.
    session.send(b"\x1b[<35;50;10M");
    thread::sleep(Duration::from_millis(100));
    assert_eq!(session.text().matches("run-cli:main.py").count(), 1);
    assert_eq!(session.text().matches("[ Interactive ]").count(), 1);
    session.send(b"/xyz");
    session.wait_for("/xyz");
    assert_eq!(
        session.text().matches("[ Interactive ]").count(),
        1,
        "toolbar should remain static during prompt redraws"
    );
    session.send(b"\x03");
    session.send(b"/quit\r");
    assert!(session.wait().success());
}

#[test]
fn clipboard_run_alias_saves_and_runs_the_next_case() {
    let directory = TempDir::new().unwrap();
    let source = source(
        directory.path(),
        "a, b = map(int, input().split())\nprint(a + b)\n",
    );
    let bin = directory.path().join("bin");
    fs::create_dir(&bin).unwrap();
    let clipboard = bin.join("wl-paste");
    fs::write(&clipboard, "#!/bin/sh\nprintf '2 3\\n'\n").unwrap();
    fs::set_permissions(&clipboard, fs::Permissions::from_mode(0o755)).unwrap();
    let mut command = repl_command(&directory, &source, false);
    let inherited_path = std::env::var_os("PATH").unwrap_or_default();
    command.env(
        "PATH",
        std::env::join_paths(
            std::iter::once(bin.clone()).chain(std::env::split_paths(&inherited_path)),
        )
        .unwrap(),
    );
    let session = PtySession::spawn(command);
    session.wait_for("run-cli:main.py");
    session.send(b"/run clipboard\r");
    session.wait_for("Saved input #1");
    session.wait_for("Success (exit 0)");
    session.send(b"/quit\r");
    assert!(session.wait().success());
    assert_eq!(
        fs::read_to_string(directory.path().join("main.in1")).unwrap(),
        "2 3\n"
    );
}

#[test]
fn mouse_toolbar_runs_a_specific_numbered_case() {
    let directory = TempDir::new().unwrap();
    let source = source(directory.path(), "print(input().strip())\n");
    fs::write(directory.path().join("main.in7"), "case-seven\n").unwrap();
    let session = PtySession::spawn(repl_command(&directory, &source, true));
    session.wait_for("[ 7 ]");
    session.wait_for("[ All ]");
    // The first numbered case begins at zero-based column 53 on the full bar.
    session.send(b"\x1b[<0;54;24M");
    session.wait_for("case-seven");
    session.wait_for("Success (exit 0)");
    session.send(b"/quit\r");
    assert!(session.wait().success());
}
