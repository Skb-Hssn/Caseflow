#![cfg(unix)]

use portable_pty::{native_pty_system, Child, CommandBuilder, MasterPty, PtySize};
use std::fs;
use std::io::{Read, Write};
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
    session.resize(8, 32);
    session.send(b"/status\r");
    session.wait_for("Language");
    session.resize(30, 120);
    session.send(b"/quit\r");
    session.wait_for("RESTORED=yes");
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
    // The prompt occupies row 1 and the first completion is rendered on row 2.
    session.send(b"\x1b[<0;2;2M");
    session.send(b"\r");
    session.wait_for("\r\n5\r\n");
    session.wait_for("Success (exit 0)");
    session.send(b"/quit\r");
    assert!(session.wait().success());
}
