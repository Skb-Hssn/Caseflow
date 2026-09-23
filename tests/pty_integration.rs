#![cfg(unix)]

use portable_pty::{native_pty_system, Child, CommandBuilder, MasterPty, PtySize};
use std::fs;
use std::io::{Read, Write};
use std::net::TcpListener;
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
    child: Option<Box<dyn Child + Send + Sync>>,
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
            child: Some(child),
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

    fn output_len(&self) -> usize {
        self.output.lock().unwrap().len()
    }

    fn text_since(&self, offset: usize) -> String {
        let output = self.output.lock().unwrap();
        String::from_utf8_lossy(&output[offset.min(output.len())..]).into_owned()
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

    fn wait_for_count(&self, needle: &str, expected: usize) {
        let deadline = Instant::now() + Duration::from_secs(10);
        while Instant::now() < deadline {
            if self.text().matches(needle).count() >= expected {
                return;
            }
            thread::sleep(Duration::from_millis(20));
        }
        panic!(
            "PTY output did not contain {expected} copies of {needle:?}:\n{}",
            self.text()
        );
    }

    fn wait_for_sequence_since(&self, offset: usize, needles: &[&str]) {
        let deadline = Instant::now() + Duration::from_secs(10);
        while Instant::now() < deadline {
            let text = self.text_since(offset);
            let mut remaining = text.as_str();
            let mut matched = true;
            for needle in needles {
                let Some(index) = remaining.find(needle) else {
                    matched = false;
                    break;
                };
                remaining = &remaining[index + needle.len()..];
            }
            if matched {
                return;
            }
            thread::sleep(Duration::from_millis(20));
        }
        panic!(
            "PTY output after byte {offset} did not contain sequence {needles:?}:\n{}",
            self.text_since(offset)
        );
    }

    fn wait(mut self) -> portable_pty::ExitStatus {
        self.child.take().unwrap().wait().unwrap()
    }
}

impl Drop for PtySession {
    fn drop(&mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
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
    if !mouse {
        command.arg("--no-mouse");
    }
    command.arg(source);
    command.env("XDG_CACHE_HOME", directory.path().join("cache"));
    command.env("XDG_CONFIG_HOME", directory.path().join("config"));
    command.env("XDG_STATE_HOME", directory.path().join("state"));
    command
}

fn unused_loopback_port() -> Option<u16> {
    match TcpListener::bind(("127.0.0.1", 0)) {
        Ok(listener) => Some(listener.local_addr().unwrap().port()),
        Err(error) if error.kind() == std::io::ErrorKind::PermissionDenied => {
            eprintln!("skipping loopback test: {error}");
            None
        }
        Err(error) => panic!("could not reserve a loopback port: {error}"),
    }
}

#[test]
fn ctrl_c_cancels_companion_wait_and_returns_to_repl() {
    let directory = TempDir::new().unwrap();
    let source = source(directory.path(), "print('ready')\n");
    let Some(port) = unused_loopback_port() else {
        return;
    };
    let session = PtySession::spawn(repl_command(&directory, &source, false));
    session.wait_for("caseflow:main.py");
    let prompt_count = session.text().matches("caseflow:main.py").count();

    session.send(format!("/companion --port {port} --wait 30\r").as_bytes());
    session.wait_for("Listening for Competitive Companion");
    session.send(b"\x03");
    session.wait_for("Competitive Companion import cancelled");
    session.wait_for_count("caseflow:main.py", prompt_count + 1);

    session.send(b"/exit\r");
    assert!(session.wait().success());
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
    session.wait_for("caseflow:main.py");
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
    let session = PtySession::spawn(repl_command(&directory, &source, true));
    session.wait_for("caseflow:main.py");
    session.send(b"/run\r");
    session.wait_for("child-started");
    session.send(b"\x03");
    session.wait_for("Failed (exit 130)");
    session.send(b"/quit\r");
    assert!(session.wait().success());
}

#[test]
fn ctrl_c_stops_the_entire_saved_case_batch() {
    let directory = TempDir::new().unwrap();
    let source = source(
        directory.path(),
        concat!(
            "import sys, time\n",
            "value = sys.stdin.read().strip()\n",
            "print(f'batch-case-start:{value}', flush=True)\n",
            "if value == 'hang':\n",
            "    time.sleep(30)\n",
        ),
    );
    fs::write(directory.path().join("main.in1"), "hang\n").unwrap();
    fs::write(
        directory.path().join("main.in2"),
        "second-case-must-not-run\n",
    )
    .unwrap();

    let session = PtySession::spawn(repl_command(&directory, &source, true));
    session.wait_for("caseflow:main.py");
    let run_offset = session.output_len();
    session.send(b"/test all\r");
    session.wait_for_sequence_since(run_offset, &["batch-case-start:hang"]);
    session.send(b"\x03");
    session.wait_for_sequence_since(
        run_offset,
        &[
            "Failed (exit 130)",
            "Test run stopped by Ctrl-C after case #1",
            "1 remaining case(s) skipped",
            "caseflow:main.py",
        ],
    );
    assert!(
        !session
            .text_since(run_offset)
            .contains("batch-case-start:second-case-must-not-run"),
        "the second saved case ran after Ctrl-C"
    );
    session.send(b"/exit\r");
    assert!(session.wait().success());
}

#[test]
fn ctrl_c_force_stops_a_signal_ignoring_loop() {
    let directory = TempDir::new().unwrap();
    let source = source(
        directory.path(),
        concat!(
            "import signal\n",
            "signal.signal(signal.SIGINT, signal.SIG_IGN)\n",
            "signal.signal(signal.SIGTERM, signal.SIG_IGN)\n",
            "print('stubborn-loop-started', flush=True)\n",
            "while True:\n",
            "    pass\n",
        ),
    );
    let session = PtySession::spawn(repl_command(&directory, &source, false));
    session.wait_for("caseflow:main.py");
    session.send(b"/run\r");
    session.wait_for("stubborn-loop-started");
    session.send(b"\x03");
    session.wait_for("Failed (exit 130)");
    session.wait_for("caseflow:main.py");
    session.send(b"/exit\r");
    assert!(session.wait().success());
}

#[test]
fn one_ctrl_c_stops_an_infinite_output_loop_without_killing_repl() {
    let directory = TempDir::new().unwrap();
    let source = source(
        directory.path(),
        "while True:\n    print('flood-output', flush=True)\n",
    );
    let session = PtySession::spawn(repl_command(&directory, &source, true));
    session.wait_for("caseflow:main.py");
    let run_offset = session.output_len();
    session.send(b"/run\r");
    session.wait_for_sequence_since(
        run_offset,
        &[
            "flood-output",
            "Output limit reached; further live output is suppressed",
        ],
    );
    let interrupt_offset = session.output_len();
    session.send(b"\x03");
    session.wait_for_sequence_since(interrupt_offset, &["Failed (exit 130)", "caseflow:main.py"]);
    session.send(b"/status\r");
    session.wait_for_sequence_since(interrupt_offset, &["Language", "caseflow:main.py"]);
    session.send(b"/exit\r");
    assert!(session.wait().success());
}

#[test]
fn ordinary_output_larger_than_the_viewport_is_not_suppressed() {
    let directory = TempDir::new().unwrap();
    let source = source(
        directory.path(),
        "for index in range(40):\n    print(f'ordinary-{index:02}', flush=True)\n",
    );
    let session = PtySession::spawn(repl_command(&directory, &source, true));
    session.wait_for("caseflow:main.py");
    let run_offset = session.output_len();
    session.send(b"/run\r");
    session.wait_for_sequence_since(
        run_offset,
        &["ordinary-39", "Success (exit 0)", "caseflow:main.py"],
    );
    let output = session.text_since(run_offset);
    assert!(
        !output.contains("Output limit reached"),
        "ordinary output should remain live without a suppression warning"
    );
    assert!(!output.contains("... final output ..."));
    session.send(b"/exit\r");
    assert!(session.wait().success());
}

#[test]
fn saved_case_with_expected_output_shows_a_pass_verdict() {
    let directory = TempDir::new().unwrap();
    let source = source(directory.path(), "print(input().strip().upper())\n");
    fs::write(directory.path().join("main.in1"), "accepted\n").unwrap();
    fs::write(directory.path().join("main.out1"), "ACCEPTED\n").unwrap();
    let session = PtySession::spawn(repl_command(&directory, &source, true));
    session.wait_for("caseflow:main.py");
    let run_offset = session.output_len();
    session.send(b"/test 1\r");
    session.wait_for_sequence_since(
        run_offset,
        &[
            "ACCEPTED",
            "Case #1 verdict: PASS",
            "Verdict: PASS · 1/1 judged case(s) passed",
            "caseflow:main.py",
        ],
    );
    session.send(b"/exit\r");
    assert!(session.wait().success());
}

#[test]
fn failed_case_shows_expected_output_and_batch_summary() {
    let directory = TempDir::new().unwrap();
    let source = source(directory.path(), "print(input().strip().upper())\n");
    fs::write(directory.path().join("main.in1"), "accepted\n").unwrap();
    fs::write(directory.path().join("main.out1"), "ACCEPTED\n").unwrap();
    fs::write(directory.path().join("main.in2"), "actual two\n").unwrap();
    fs::write(directory.path().join("main.out2"), "EXPECTED TWO\n").unwrap();
    fs::write(directory.path().join("main.in3"), "run only\n").unwrap();

    let session = PtySession::spawn(repl_command(&directory, &source, true));
    session.wait_for("caseflow:main.py");
    let run_offset = session.output_len();
    session.send(b"/test all\r");
    session.wait_for_sequence_since(
        run_offset,
        &[
            "Expected Output",
            "EXPECTED TWO",
            "Case #2 verdict: FAIL",
            "System status",
            "main.in3",
            "Test summary",
            "[1]",
            "[2]",
            "[3]",
            "caseflow:main.py",
        ],
    );
    session.send(b"/exit\r");
    assert!(session.wait().success());
}

#[test]
fn unflushed_cpp_output_is_visible_while_the_program_is_running() {
    let directory = TempDir::new().unwrap();
    let source = directory.path().join("main.cpp");
    fs::write(
        &source,
        "#include <cstdio>\nint main() { std::puts(\"unflushed-live-output\"); for (;;) {} }\n",
    )
    .unwrap();
    let session = PtySession::spawn(repl_command(&directory, &source, true));
    session.wait_for("caseflow:main.cpp");
    let run_offset = session.output_len();
    session.send(b"/run\r");
    session.wait_for_sequence_since(run_offset, &["unflushed-live-output"]);
    let interrupt_offset = session.output_len();
    session.send(b"\x03");
    session.wait_for_sequence_since(
        interrupt_offset,
        &["Failed (exit 130)", "caseflow:main.cpp"],
    );
    session.send(b"/exit\r");
    assert!(session.wait().success());
}

#[test]
fn saved_case_output_is_visible_before_the_program_exits() {
    let directory = TempDir::new().unwrap();
    let source = source(
        directory.path(),
        "print('saved-case-live-output', flush=True)\nwhile True:\n    pass\n",
    );
    fs::write(directory.path().join("main.in1"), "1\n").unwrap();
    let session = PtySession::spawn(repl_command(&directory, &source, true));
    session.wait_for("caseflow:main.py");
    let run_offset = session.output_len();
    session.send(b"/test 1\r");
    session.wait_for_sequence_since(
        run_offset,
        &["File", "Input", "Output", "saved-case-live-output"],
    );
    let interrupt_offset = session.output_len();
    session.send(b"\x03");
    session.wait_for_sequence_since(interrupt_offset, &["Failed (exit 130)", "caseflow:main.py"]);
    session.send(b"/exit\r");
    assert!(session.wait().success());
}

#[test]
fn ctrl_c_stops_stress_testing_without_saving_a_false_mismatch() {
    let directory = TempDir::new().unwrap();
    let source = source(
        directory.path(),
        "from pathlib import Path\nPath('candidate-ready').write_text('ready')\nwhile True:\n    pass\n",
    );
    fs::write(directory.path().join("brute.py"), "print(int(input()))\n").unwrap();
    fs::write(
        directory.path().join("generator.py"),
        "import sys\nprint(sys.argv[1])\n",
    )
    .unwrap();
    let session = PtySession::spawn(repl_command(&directory, &source, true));
    session.wait_for("caseflow:main.py");
    let run_offset = session.output_len();
    session.send(b"/stress brute.py generator.py --runs 100 --timeout 60\r");
    let ready = directory.path().join("candidate-ready");
    let deadline = Instant::now() + Duration::from_secs(10);
    while !ready.exists() && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(20));
    }
    assert!(ready.exists(), "candidate did not start during stress test");
    session.send(b"\x03");
    session.wait_for_sequence_since(
        run_offset,
        &[
            "Stress testing stopped during program on case 1",
            "caseflow:main.py",
        ],
    );
    assert!(!directory.path().join("main.in1").exists());
    session.send(b"/exit\r");
    assert!(session.wait().success());
}

#[test]
fn interactive_input_is_retained_after_enter() {
    let directory = TempDir::new().unwrap();
    let source = source(
        directory.path(),
        "value = input()\nprint(f'answer:{value}')\n",
    );
    let session = PtySession::spawn(repl_command(&directory, &source, false));
    session.wait_for("caseflow:main.py");
    session.send(b"/run\r");
    session.wait_for("interactive input");
    session.send(b"hello-world\r");
    session.wait_for("answer:hello-world");
    session.wait_for_count("hello-world", 3);
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
    session.wait_for("caseflow:main.py");
    session.send(b"/run --input input-\t");
    session.wait_for("[ Select ]");
    // Suggestions occupy the bottom of the viewport; the first item is row 20.
    session.send(b"\x1b[<0;2;20M");
    session.send(b"\r");
    session.wait_for("Success (exit 0)");
    session.send(b"/quit\r");
    assert!(session.wait().success());
}

#[test]
fn slash_opens_dimmed_commands_and_tab_accepts_one() {
    let directory = TempDir::new().unwrap();
    let source = source(directory.path(), "print('ready')\n");
    let session = PtySession::spawn(repl_command(&directory, &source, false));
    session.wait_for("caseflow:main.py");
    session.send(b"/");
    session.wait_for("Options");
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
    session.wait_for("[Int]");
    // The toolbar is fixed above the command row; this lands inside Int.
    session.send(b"\x1b[<0;10;23M");
    session.wait_for("toolbar-run");
    session.wait_for("Success (exit 0)");
    session.send(b"/quit\r");
    assert!(session.wait().success());
}

#[test]
fn mouse_toolbar_again_repeats_the_latest_run() {
    let directory = TempDir::new().unwrap();
    let source = source(directory.path(), "print('toolbar-again')\n");
    let session = PtySession::spawn(repl_command(&directory, &source, true));
    session.wait_for("[Again]");
    session.send(b"\x1b[<0;10;23M");
    session.wait_for_count("toolbar-again", 1);
    session.wait_for("Success (exit 0)");
    session.send(b"\x1b[<0;22;23M");
    session.wait_for_count("toolbar-again", 2);
    session.send(b"/quit\r");
    assert!(session.wait().success());
}

#[test]
fn mouse_toolbar_add_case_opens_the_external_editor() {
    let directory = TempDir::new().unwrap();
    let source = source(directory.path(), "print(input().strip())\n");
    let editor = directory.path().join("toolbar-editor.sh");
    fs::write(&editor, "#!/bin/sh\nprintf 'toolbar case\\n' > \"$1\"\n").unwrap();
    fs::set_permissions(&editor, fs::Permissions::from_mode(0o755)).unwrap();
    let mut command = repl_command(&directory, &source, true);
    command.env("VISUAL", &editor);
    let session = PtySession::spawn(command);
    session.wait_for("[+Case]");
    session.send(b"\x1b[<0;37;23M");
    session.wait_for("Added input #1");
    assert_eq!(
        fs::read_to_string(directory.path().join("main.in1")).unwrap(),
        "toolbar case\n"
    );
    session.send(b"/quit\r");
    assert!(session.wait().success());
}

#[test]
fn mouse_toolbar_more_menu_runs_the_selected_action() {
    let directory = TempDir::new().unwrap();
    let source = source(directory.path(), "print('ready')\n");
    let session = PtySession::spawn(repl_command(&directory, &source, true));
    session.wait_for("[More]");
    session.send(b"\x1b[<0;75;23M");
    session.wait_for("More actions");
    session.wait_for("Download contest");
    // Help is the ninth menu item and appears on terminal row 13.
    session.send(b"\x1b[<0;5;13M");
    session.wait_for("KEYS");
    session.wait_for("caseflow:main.py");
    session.send(b"/quit\r");
    assert!(session.wait().success());
}

#[test]
fn more_menu_case_picker_edits_and_deletes_a_saved_case() {
    let directory = TempDir::new().unwrap();
    let source = source(directory.path(), "print(input().strip())\n");
    let saved = directory.path().join("main.in3");
    fs::write(&saved, "before\n").unwrap();
    let editor = directory.path().join("case-picker-editor.sh");
    fs::write(&editor, "#!/bin/sh\nprintf 'after\\n' > \"$1\"\n").unwrap();
    fs::set_permissions(&editor, fs::Permissions::from_mode(0o755)).unwrap();
    let mut command = repl_command(&directory, &source, true);
    command.env("VISUAL", &editor);
    let session = PtySession::spawn(command);
    session.wait_for("[More]");

    // More → Edit case → Case #3.
    session.send(b"\x1b[<0;79;23M");
    session.wait_for("More actions");
    session.send(b"\x1b[<0;5;6M");
    session.wait_for("Edit saved case");
    session.send(b"\x1b[<0;5;5M");
    session.wait_for("Updated input #3");
    assert_eq!(fs::read_to_string(&saved).unwrap(), "after\n");

    // More → Delete case → Case #3; Enter confirms the existing safety dialog.
    session.send(b"\x1b[<0;79;23M");
    session.wait_for("More actions");
    session.send(b"\x1b[<0;5;11M");
    session.wait_for("Delete saved case");
    session.send(b"\x1b[<0;5;5M");
    session.wait_for("Delete saved input #3?");
    session.send(b"\r");
    session.wait_for("Deleted");
    assert!(!saved.exists());

    session.send(b"/quit\r");
    assert!(session.wait().success());
}

#[test]
fn mouse_toolbar_is_not_redrawn_while_typing() {
    let directory = TempDir::new().unwrap();
    let source = source(directory.path(), "print('ready')\n");
    let session = PtySession::spawn(repl_command(&directory, &source, true));
    session.wait_for("[Int]");
    assert!(
        !session.text().contains("\x1b[?1003h"),
        "mouse mode must not enable all-motion tracking"
    );
    assert_eq!(session.text().matches("caseflow:main.py").count(), 1);
    // An unsolicited pointer-motion report must not repaint any editor row.
    session.send(b"\x1b[<35;50;10M");
    thread::sleep(Duration::from_millis(100));
    assert_eq!(session.text().matches("caseflow:main.py").count(), 1);
    assert_eq!(session.text().matches("[Int]").count(), 1);
    session.send(b"/xyz");
    session.wait_for("/xyz");
    assert_eq!(
        session.text().matches("[Int]").count(),
        1,
        "toolbar should remain static during prompt redraws"
    );
    session.send(b"\x03");
    session.send(b"/quit\r");
    assert!(session.wait().success());
}

#[test]
fn debug_toolbar_button_toggles_build_mode() {
    let directory = TempDir::new().unwrap();
    let source = source(directory.path(), "print('ready')\n");
    let session = PtySession::spawn(repl_command(&directory, &source, true));
    session.wait_for("[Off]");
    assert!(!session.text().contains("[ Build ]"));
    session.send(b"\x1b[<0;58;23M");
    session.wait_for("Build mode: debug");
    session.wait_for("[On]");
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
    session.wait_for("caseflow:main.py");
    session.send(b"/run clipboard");
    session.wait_for("/run clipboard");
    let prompt_count = session.text().matches("caseflow:main.py").count();
    session.send(b"\r");
    session.wait_for("Saved input #1");
    session.wait_for("File ·");
    session.wait_for("System status  ·  ✓ Success (exit 0)");
    session.wait_for_count("caseflow:main.py", prompt_count + 1);
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
    session.wait_for("[7]");
    session.wait_for("[All]");
    // The first numbered case follows the Run and +Case controls.
    session.send(b"\x1b[<0;44;23M");
    session.wait_for("File ·");
    session.wait_for("Input");
    session.wait_for("Output");
    session.wait_for("System status");
    session.wait_for_count("case-seven", 2);
    session.wait_for("Success (exit 0)");
    session.send(b"/quit\r");
    assert!(session.wait().success());
}

#[test]
fn select_button_releases_mouse_for_native_copying() {
    let directory = TempDir::new().unwrap();
    let source = source(directory.path(), "print('selectable')\n");
    let session = PtySession::spawn(repl_command(&directory, &source, true));
    session.wait_for("[Select]");
    session.send(b"/help\r");
    session.wait_for("KEYS");
    session.wait_for("caseflow:main.py");
    session.send(b"\x1b[<0;66;23M");
    session.wait_for("wheel/↑↓ scroll");
    assert!(session.text().contains("\x1b[?1007h"));
    // Alternate-scroll mode translates the wheel to cursor keys while native
    // terminal selection owns the mouse.
    session.send(b"\x1b[A");
    session.wait_for("↑ 3");
    let prompt_count = session.text().matches("caseflow:main.py").count();
    session.send(b"\x1b");
    session.wait_for_count("caseflow:main.py", prompt_count + 1);
    session.send(b"/quit\r");
    assert!(session.wait().success());
}

#[test]
fn mouse_wheel_scrolls_only_the_output_viewport() {
    let directory = TempDir::new().unwrap();
    let source = source(
        directory.path(),
        "for index in range(40):\n    print(f'line-{index:02}')\n",
    );
    let session = PtySession::spawn(repl_command(&directory, &source, true));
    session.wait_for("[Int]");
    session.send(b"/run\r");
    session.wait_for("Success (exit 0)");
    session.wait_for("caseflow:main.py");
    let before = session.text();
    assert!(
        before.contains("\x1b[3;20r"),
        "live output must be confined between the fixed header and footer"
    );
    let before_len = before.len();
    let title = format!(" Caseflow  v{}  ·  ", env!("CARGO_PKG_VERSION"));
    let header_count = before.matches(&title).count();
    let toolbar_count = before.matches("[Int]").count();
    for _ in 0..8 {
        session.send(b"\x1b[<64;50;10M");
    }
    thread::sleep(Duration::from_millis(100));
    let after = session.text();
    let scrolled_render = &after[before_len..];
    assert!(scrolled_render.contains("line-10"));
    assert!(scrolled_render.contains('↑'));
    assert_eq!(after.matches(&title).count(), header_count);
    assert_eq!(after.matches("[Int]").count(), toolbar_count);
    session.send(b"/quit\r");
    assert!(session.wait().success());
}

#[test]
fn again_is_recoverable_and_keeps_repeating_the_latest_run() {
    let directory = TempDir::new().unwrap();
    let source = source(
        directory.path(),
        "from pathlib import Path\n\
         counter = Path('run-count.txt')\n\
         count = int(counter.read_text()) + 1 if counter.exists() else 1\n\
         counter.write_text(str(count))\n\
         print(f'repeat-run-{count}')\n",
    );
    let session = PtySession::spawn(repl_command(&directory, &source, false));
    session.wait_for("caseflow:main.py");

    let command_start = session.output_len();
    session.send(b"/again\r");
    session.wait_for_sequence_since(
        command_start,
        &[
            "nothing to repeat; run /run or /test first",
            "caseflow:main.py",
        ],
    );

    let command_start = session.output_len();
    session.send(b"/run\r");
    session.wait_for_sequence_since(
        command_start,
        &["repeat-run-1", "Success (exit 0)", "caseflow:main.py"],
    );

    // An unrelated command must not replace the repeatable command.
    let command_start = session.output_len();
    session.send(b"/status\r");
    session.wait_for_sequence_since(command_start, &["SESSION", "caseflow:main.py"]);
    let command_start = session.output_len();
    session.send(b"/again\r");
    session.wait_for_sequence_since(
        command_start,
        &["repeat-run-2", "Success (exit 0)", "caseflow:main.py"],
    );

    // `/again` must retain the expanded /run command instead of repeating itself.
    let command_start = session.output_len();
    session.send(b"/again\r");
    session.wait_for_sequence_since(
        command_start,
        &["repeat-run-3", "Success (exit 0)", "caseflow:main.py"],
    );
    assert_eq!(
        fs::read_to_string(directory.path().join("run-count.txt")).unwrap(),
        "3"
    );

    session.send(b"/exit\r");
    assert!(session.wait().success());
}

#[test]
fn clear_removes_retained_output_and_the_prompt_remains_usable() {
    const MARKER: &str = "RETAINED-OUTPUT-MARKER-48271";
    let directory = TempDir::new().unwrap();
    let source = source(directory.path(), &format!("print('{MARKER}')\n"));
    let session = PtySession::spawn(repl_command(&directory, &source, false));
    session.wait_for("caseflow:main.py");

    let command_start = session.output_len();
    session.send(b"/run\r");
    session.wait_for_sequence_since(
        command_start,
        &[MARKER, "Success (exit 0)", "caseflow:main.py"],
    );

    let clear_offset = session.output_len();
    session.send(b"/clear\r/help clear\r");
    let help = "Clear retained output from the viewport.";
    session.wait_for_sequence_since(clear_offset, &[help, "caseflow:main.py"]);
    // The first help line is emitted live. The following workspace render would
    // expose the old marker again if /clear had not removed retained lines.
    let after_clear = session.text_since(clear_offset);
    let after_live_help = &after_clear[after_clear.find(help).unwrap() + help.len()..];
    assert!(
        !after_live_help.contains(MARKER),
        "cleared program output was rendered again"
    );

    session.send(b"/exit\r");
    assert!(session.wait().success());
}

#[test]
fn canonical_session_controls_work_from_the_keyboard() {
    let directory = TempDir::new().unwrap();
    let source = source(directory.path(), "print('ready')\n");
    let session = PtySession::spawn(repl_command(&directory, &source, false));
    session.wait_for("caseflow:main.py");

    let command_start = session.output_len();
    session.send(b"/debug on\r");
    session.wait_for_sequence_since(command_start, &["Build mode: debug", "caseflow:main.py"]);
    let command_start = session.output_len();
    session.send(b"/debug off\r");
    session.wait_for_sequence_since(command_start, &["Build mode: standard", "caseflow:main.py"]);
    let command_start = session.output_len();
    session.send(b"/debug\r");
    session.wait_for_sequence_since(command_start, &["Build mode: debug", "caseflow:main.py"]);
    let command_start = session.output_len();
    session.send(b"/debug toggle\r");
    session.wait_for_sequence_since(command_start, &["Build mode: standard", "caseflow:main.py"]);

    let command_start = session.output_len();
    session.send(b"/mouse\r");
    session.wait_for_sequence_since(
        command_start,
        &["Mouse controls: on", "[Int]", "caseflow:main.py"],
    );
    let command_start = session.output_len();
    session.send(b"/mouse\r");
    session.wait_for_sequence_since(command_start, &["Mouse controls: off", "caseflow:main.py"]);

    let command_start = session.output_len();
    session.send(b"/help run\r");
    session.wait_for_sequence_since(
        command_start,
        &["/run clipboard [--timeout SEC]", "caseflow:main.py"],
    );
    session.send(b"/exit\r");
    assert!(session.wait().success());
}

#[test]
fn enter_confirms_case_deletion() {
    let directory = TempDir::new().unwrap();
    let source = source(directory.path(), "print(input().strip())\n");
    let saved = directory.path().join("main.in2");
    fs::write(&saved, "delete me\n").unwrap();
    let session = PtySession::spawn(repl_command(&directory, &source, false));
    session.wait_for("caseflow:main.py");

    let command_start = session.output_len();
    session.send(b"/case delete 2\r");
    session.wait_for_sequence_since(command_start, &["Delete saved input #2?", "[ Delete ]"]);
    let confirmation_count = session
        .text_since(command_start)
        .matches("Delete saved input #2?")
        .count();
    session.send(b"x");
    thread::sleep(Duration::from_millis(100));
    assert_eq!(
        session
            .text_since(command_start)
            .matches("Delete saved input #2?")
            .count(),
        confirmation_count,
        "ignored input must not redraw and duplicate the confirmation"
    );
    session.send(b"\r");
    session.wait_for_sequence_since(command_start, &["Deleted", "caseflow:main.py"]);
    assert!(
        !saved.exists(),
        "Enter should confirm the focused delete action"
    );

    session.send(b"/exit\r");
    assert!(session.wait().success());
}

#[test]
fn external_case_editor_owns_the_tty_and_rolls_back_failures() {
    let directory = TempDir::new().unwrap();
    let source = source(directory.path(), "print(input().strip())\n");
    let editor_directory = directory.path().join("fake editors");
    fs::create_dir(&editor_directory).unwrap();
    let editor = editor_directory.join("fake visual.sh");
    let editor_state = directory.path().join("editor state");
    fs::write(
        &editor,
        "#!/bin/sh\n\
         set -eu\n\
         test \"$1\" = --label\n\
         test \"$2\" = 'quoted argument'\n\
         label=$2\n\
         shift 2\n\
         test -t 0 && test -t 1 && test -t 2 || exit 91\n\
         count=0\n\
         test ! -f \"$EDITOR_STATE\" || count=$(cat \"$EDITOR_STATE\")\n\
         count=$((count + 1))\n\
         printf '%s\\n' \"$count\" > \"$EDITOR_STATE\"\n\
         printf 'EDITOR_TTY_OK:%s:%s\\n' \"$count\" \"$label\"\n\
         case \"$count\" in\n\
           1) printf 'added through editor\\n' > \"$1\" ;;\n\
           2) printf 'edited through editor\\n' > \"$1\" ;;\n\
           *) printf 'must not be committed\\n' > \"$1\"; exit 23 ;;\n\
         esac\n",
    )
    .unwrap();
    fs::set_permissions(&editor, fs::Permissions::from_mode(0o755)).unwrap();

    let mut command = repl_command(&directory, &source, false);
    command.env(
        "VISUAL",
        format!("\"{}\" --label \"quoted argument\"", editor.display()),
    );
    command.env("EDITOR", "/definitely/not/the/editor");
    command.env("EDITOR_STATE", &editor_state);
    let session = PtySession::spawn(command);
    session.wait_for("caseflow:main.py");
    let initial_prompt_count = session.text().matches("caseflow:main.py").count();

    session.send(b"/case add\r");
    session.wait_for("EDITOR_TTY_OK:1:quoted argument");
    session.wait_for("Added input #1");
    session.wait_for_count("caseflow:main.py", initial_prompt_count + 1);
    assert_eq!(
        fs::read_to_string(directory.path().join("main.in1")).unwrap(),
        "added through editor\n"
    );

    session.send(b"/case edit 1\r");
    session.wait_for("EDITOR_TTY_OK:2:quoted argument");
    session.wait_for("Updated input #1");
    session.wait_for_count("caseflow:main.py", initial_prompt_count + 2);
    assert_eq!(
        fs::read_to_string(directory.path().join("main.in1")).unwrap(),
        "edited through editor\n"
    );

    session.send(b"/case edit 1\r");
    session.wait_for("EDITOR_TTY_OK:3:quoted argument");
    session.wait_for("exited with status 23");
    session.wait_for_count("caseflow:main.py", initial_prompt_count + 3);
    assert_eq!(
        fs::read_to_string(directory.path().join("main.in1")).unwrap(),
        "edited through editor\n",
        "a failed editor must not replace the saved case"
    );
    let temporary_root = directory.path().join("cache/run-cli/tmp");
    assert!(
        fs::read_dir(&temporary_root).unwrap().all(|entry| !entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with("case-editor-")),
        "editor staging directories should be removed after success and failure"
    );

    let output = session.text();
    let initial_enter = output.find("\x1b[?1049h").unwrap();
    let first_leave = output[initial_enter + 1..]
        .find("\x1b[?1049l")
        .map(|offset| initial_enter + 1 + offset)
        .expect("run-cli should leave its alternate screen for the editor");
    let editor_marker = output[first_leave..]
        .find("EDITOR_TTY_OK:1:quoted argument")
        .map(|offset| first_leave + offset)
        .unwrap();
    let resumed = output[editor_marker..]
        .find("\x1b[?1049h")
        .map(|offset| editor_marker + offset)
        .expect("run-cli should restore its alternate screen after the editor");
    assert!(first_leave < editor_marker && editor_marker < resumed);
    assert_eq!(
        output.matches("\x1b[?1049l").count(),
        3,
        "each editor invocation should suspend the workspace once"
    );
    assert_eq!(
        output.matches("\x1b[?1049h").count(),
        4,
        "the initial workspace and all three resumptions should enter once"
    );

    session.send(b"/quit\r");
    assert!(session.wait().success());
}

#[test]
fn edit_command_creates_and_reopens_any_file() {
    let directory = TempDir::new().unwrap();
    let source = source(directory.path(), "print('ready')\n");
    let external_editor = directory.path().join("generic-editor.sh");
    fs::write(
        &external_editor,
        "#!/bin/sh\n\
         set -eu\n\
         test -t 0 && test -t 1 && test -t 2 || exit 91\n\
         test -f \"$1\" || exit 92\n\
         printf 'GENERIC_EDITOR:%s\\n' \"$1\"\n\
         printf 'edited\\n' >> \"$1\"\n",
    )
    .unwrap();
    fs::set_permissions(&external_editor, fs::Permissions::from_mode(0o755)).unwrap();

    let mut command = repl_command(&directory, &source, false);
    command.env("VISUAL", &external_editor);
    let session = PtySession::spawn(command);
    session.wait_for("caseflow:main.py");
    let target = directory.path().join("notes with spaces.txt");

    session.send(b"/edit 'notes with spaces.txt'\r");
    session.wait_for("GENERIC_EDITOR:notes with spaces.txt");
    session.wait_for("Created file  notes with spaces.txt");
    assert_eq!(fs::read_to_string(&target).unwrap(), "edited\n");

    session.send(b"/edit 'notes with spaces.txt'\r");
    session.wait_for_count("GENERIC_EDITOR:notes with spaces.txt", 2);
    session.wait_for("Edited file  notes with spaces.txt");
    assert_eq!(fs::read_to_string(&target).unwrap(), "edited\nedited\n");

    session.send(b"/exit\r");
    assert!(session.wait().success());
}
