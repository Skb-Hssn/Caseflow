use std::fs;
use std::io::{Read, Write};
use std::net::{Shutdown, TcpListener, TcpStream};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Mutex, MutexGuard};
use std::thread;
use std::time::{Duration, Instant};
use tempfile::TempDir;

static LOOPBACK_TEST_LOCK: Mutex<()> = Mutex::new(());

fn binary() -> &'static str {
    env!("CARGO_BIN_EXE_run-cli")
}

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join(name)
}

fn command(directory: &TempDir) -> Command {
    let mut command = Command::new(binary());
    command
        .arg("--color")
        .arg("never")
        .env("XDG_CACHE_HOME", directory.path().join("cache"))
        .env("XDG_CONFIG_HOME", directory.path().join("config"))
        .env("XDG_STATE_HOME", directory.path().join("state"));
    command
}

fn tool_exists(tool: &str) -> bool {
    let exists = std::env::var_os("PATH")
        .map(|paths| std::env::split_paths(&paths).any(|path| path.join(tool).is_file()))
        .unwrap_or(false);
    let version_args: &[&str] = match tool {
        "go" => &["version"],
        "kotlinc" => &["-version"],
        _ => &["--version"],
    };
    exists
        && Command::new(tool)
            .args(version_args)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map(|status| status.success())
            .unwrap_or(false)
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

fn loopback_test_guard() -> MutexGuard<'static, ()> {
    LOOPBACK_TEST_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn connect_with_retry(port: u16) -> TcpStream {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        match TcpStream::connect(("127.0.0.1", port)) {
            Ok(stream) => return stream,
            Err(error) if Instant::now() < deadline => {
                let _ = error;
                thread::sleep(Duration::from_millis(20));
            }
            Err(error) => panic!("could not connect to companion receiver: {error}"),
        }
    }
}

#[test]
fn canonical_run_and_legacy_exec_are_equivalent() {
    let directory = TempDir::new().unwrap();
    let source = directory.path().join("sum.py");
    let input = directory.path().join("numbers.in");
    let canonical_output = directory.path().join("canonical.out");
    let legacy_output = directory.path().join("legacy.out");
    fs::copy(fixture("sum.py"), &source).unwrap();
    fs::write(&input, "20 22\n").unwrap();

    for (subcommand, output) in [("run", &canonical_output), ("exec", &legacy_output)] {
        let result = command(&directory)
            .arg(subcommand)
            .arg(&source)
            .arg("--input")
            .arg(&input)
            .arg("--output")
            .arg(output)
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{subcommand} failed: {}",
            String::from_utf8_lossy(&result.stderr)
        );
    }

    assert_eq!(fs::read(&canonical_output).unwrap(), b"42\n");
    assert_eq!(
        fs::read(&canonical_output).unwrap(),
        fs::read(&legacy_output).unwrap()
    );
}

#[test]
fn canonical_test_selectors_cover_all_ids_commas_and_last() {
    let directory = TempDir::new().unwrap();
    let source = directory.path().join("echo.py");
    fs::write(&source, "print(input().strip())\n").unwrap();
    for (id, value) in [(1, "one\n"), (2, "two\n"), (3, "three\n")] {
        fs::write(directory.path().join(format!("echo.in{id}")), value).unwrap();
    }

    for (selectors, expected) in [
        (Vec::<&str>::new(), "one\ntwo\nthree\n"),
        (vec!["1", "3"], "one\nthree\n"),
        (vec!["1,2"], "one\ntwo\n"),
        (vec!["last"], "three\n"),
    ] {
        let mut run = command(&directory);
        run.arg("test").arg(&source).args(&selectors);
        let output = run.output().unwrap();
        assert!(
            output.status.success(),
            "selectors {selectors:?} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(String::from_utf8_lossy(&output.stdout), expected);
    }
}

#[test]
fn test_automatically_judges_matching_output_files() {
    let directory = TempDir::new().unwrap();
    let source = directory.path().join("judge.py");
    fs::write(
        &source,
        "import sys\nsys.stdout.write(sys.stdin.read().strip().upper() + '\\n')\n",
    )
    .unwrap();
    for (id, input) in [(1, "accepted\n"), (2, "wrong\n"), (3, "run only\n")] {
        fs::write(directory.path().join(format!("judge.in{id}")), input).unwrap();
    }
    fs::write(directory.path().join("judge.out1"), "ACCEPTED\n").unwrap();
    fs::write(directory.path().join("judge.out2"), "something else\n").unwrap();

    let failed = command(&directory)
        .arg("test")
        .arg(&source)
        .output()
        .unwrap();
    assert_eq!(failed.status.code(), Some(1));
    assert_eq!(
        String::from_utf8_lossy(&failed.stdout),
        "ACCEPTED\nWRONG\nRUN ONLY\n"
    );
    let diagnostics = String::from_utf8_lossy(&failed.stderr);
    assert!(
        diagnostics.contains("Case #1 verdict: PASS"),
        "{diagnostics}"
    );
    assert!(
        diagnostics.contains("Case #2 verdict: FAIL"),
        "{diagnostics}"
    );
    assert!(diagnostics.contains("Expected Output"), "{diagnostics}");
    assert!(diagnostics.contains("something else"), "{diagnostics}");
    assert!(
        diagnostics.contains("Verdict: FAIL · 1/2 judged case(s) passed"),
        "{diagnostics}"
    );
    assert!(
        diagnostics.contains("Test summary  ·  1/2 passed  ·  [1] [2] [3]"),
        "{diagnostics}"
    );
    assert!(!diagnostics.contains("Case #3 verdict"), "{diagnostics}");

    let passed = command(&directory)
        .arg("test")
        .arg(&source)
        .arg("1")
        .output()
        .unwrap();
    assert!(passed.status.success());
    let diagnostics = String::from_utf8_lossy(&passed.stderr);
    assert!(
        diagnostics.contains("Verdict: PASS · 1/1 judged case(s) passed"),
        "{diagnostics}"
    );
}

#[test]
fn canonical_compare_and_legacy_diff_are_equivalent() {
    let directory = TempDir::new().unwrap();
    let source = directory.path().join("main.py");
    let expected = directory.path().join("expected.out");
    fs::write(&source, "print(input().strip())\n").unwrap();
    fs::write(directory.path().join("main.in1"), "same\n").unwrap();
    fs::write(&expected, "same\n").unwrap();

    for subcommand in ["compare", "diff"] {
        let output = command(&directory)
            .arg(subcommand)
            .arg(&source)
            .arg("1")
            .arg(&expected)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{subcommand} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

#[test]
fn canonical_and_legacy_stress_syntax_both_work() {
    let directory = TempDir::new().unwrap();
    let source = directory.path().join("main.py");
    let brute = directory.path().join("brute.py");
    let generator = directory.path().join("generator.py");
    fs::write(&source, "print(int(input()))\n").unwrap();
    fs::write(&brute, "print(int(input()))\n").unwrap();
    fs::write(&generator, "import sys\nprint(sys.argv[1])\n").unwrap();

    let canonical = command(&directory)
        .arg("stress")
        .arg(&source)
        .arg(&brute)
        .arg(&generator)
        .arg("--runs")
        .arg("2")
        .arg("--timeout")
        .arg("1")
        .output()
        .unwrap();
    assert!(
        canonical.status.success(),
        "canonical stress failed: {}",
        String::from_utf8_lossy(&canonical.stderr)
    );

    let legacy = command(&directory)
        .arg("stress")
        .arg(&source)
        .arg("--brute")
        .arg(&brute)
        .arg("--generator")
        .arg(&generator)
        .arg("--limit")
        .arg("2")
        .arg("--timeout")
        .arg("1")
        .output()
        .unwrap();
    assert!(
        legacy.status.success(),
        "legacy stress failed: {}",
        String::from_utf8_lossy(&legacy.stderr)
    );
}

#[test]
fn ctrl_c_stops_generator_and_brute_stages_without_saving_cases() {
    for interrupted_stage in ["generator", "brute"] {
        let directory = TempDir::new().unwrap();
        let source = directory.path().join("main.py");
        let brute = directory.path().join("brute.py");
        let generator = directory.path().join("generator.py");
        let ready = directory.path().join(format!("{interrupted_stage}-ready"));
        fs::write(&source, "print(int(input()))\n").unwrap();
        fs::write(
            &brute,
            if interrupted_stage == "brute" {
                "from pathlib import Path\nPath('brute-ready').write_text('ready')\nwhile True:\n    pass\n"
            } else {
                "print(int(input()))\n"
            },
        )
        .unwrap();
        fs::write(
            &generator,
            if interrupted_stage == "generator" {
                "from pathlib import Path\nPath('generator-ready').write_text('ready')\nwhile True:\n    pass\n"
            } else {
                "print(1)\n"
            },
        )
        .unwrap();

        let child = command(&directory)
            .current_dir(directory.path())
            .arg("stress")
            .arg(&source)
            .arg(&brute)
            .arg(&generator)
            .arg("--runs")
            .arg("100")
            .arg("--timeout")
            .arg("60")
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(10);
        while !ready.exists() && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(20));
        }
        assert!(ready.exists(), "{interrupted_stage} stage did not start");
        unsafe {
            libc::kill(child.id() as i32, libc::SIGINT);
        }
        let output = child.wait_with_output().unwrap();
        assert_eq!(output.status.code(), Some(130));
        assert!(
            String::from_utf8_lossy(&output.stderr).contains(&format!(
                "Stress testing stopped during {interrupted_stage} on case 1"
            )),
            "unexpected stderr: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(!directory.path().join("main.in1").exists());
    }
}

#[test]
fn clipboard_run_and_bare_case_paste_append_cases() {
    let directory = TempDir::new().unwrap();
    let source = directory.path().join("sum.py");
    fs::copy(fixture("sum.py"), &source).unwrap();
    let bin = directory.path().join("bin");
    fs::create_dir(&bin).unwrap();
    let clipboard = bin.join("wl-paste");
    fs::write(&clipboard, "#!/bin/sh\nprintf '2 3\\n'\n").unwrap();
    fs::set_permissions(&clipboard, fs::Permissions::from_mode(0o755)).unwrap();
    let inherited_path = std::env::var_os("PATH").unwrap_or_default();
    let path =
        std::env::join_paths(std::iter::once(bin).chain(std::env::split_paths(&inherited_path)))
            .unwrap();

    let run = command(&directory)
        .env("PATH", &path)
        .arg("run")
        .arg(&source)
        .arg("--clipboard")
        .output()
        .unwrap();
    assert!(
        run.status.success(),
        "clipboard run failed: {}",
        String::from_utf8_lossy(&run.stderr)
    );
    assert_eq!(run.stdout, b"5\n");

    let paste = command(&directory)
        .env("PATH", &path)
        .arg("case")
        .arg("paste")
        .arg(&source)
        .output()
        .unwrap();
    assert!(
        paste.status.success(),
        "bare paste failed: {}",
        String::from_utf8_lossy(&paste.stderr)
    );
    assert_eq!(
        fs::read_to_string(directory.path().join("sum.in1")).unwrap(),
        "2 3\n"
    );
    assert_eq!(
        fs::read_to_string(directory.path().join("sum.in2")).unwrap(),
        "2 3\n"
    );
}

#[test]
fn executes_every_available_language() {
    let directory = TempDir::new().unwrap();
    let input = directory.path().join("numbers.in");
    fs::write(&input, "2 3\n").unwrap();
    let cases = [
        ("sum.cpp", "g++"),
        ("sum.c", "gcc"),
        ("sum.py", "python3"),
        ("Main.java", "javac"),
        ("sum.rs", "rustc"),
        ("sum.go", "go"),
        ("sum.kt", "kotlinc"),
    ];
    let mut executed = 0;
    for (index, (source, tool)) in cases.iter().enumerate() {
        if !tool_exists(tool) {
            continue;
        }
        let output = directory.path().join(format!("answer-{index}.out"));
        let result = command(&directory)
            .arg("exec")
            .arg(fixture(source))
            .arg("--input")
            .arg(&input)
            .arg("--output")
            .arg(&output)
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{source} failed:\n{}",
            String::from_utf8_lossy(&result.stderr)
        );
        assert_eq!(fs::read_to_string(output).unwrap(), "5\n", "{source}");
        executed += 1;
    }
    assert!(
        executed >= 2,
        "expected at least two available language toolchains"
    );
}

#[test]
fn existing_output_survives_a_failed_program() {
    let directory = TempDir::new().unwrap();
    let source = directory.path().join("fail.py");
    let input = directory.path().join("input.txt");
    let output = directory.path().join("answer.out");
    fs::write(&source, "print('partial')\nraise SystemExit(3)\n").unwrap();
    fs::write(&input, "ignored\n").unwrap();
    fs::write(&output, "keep\n").unwrap();
    let status = command(&directory)
        .arg("exec")
        .arg(&source)
        .arg("--input")
        .arg(&input)
        .arg("--output")
        .arg(&output)
        .status()
        .unwrap();
    assert_eq!(status.code(), Some(3));
    assert_eq!(fs::read_to_string(output).unwrap(), "keep\n");
}

#[test]
fn saved_cases_remain_compatible() {
    let directory = TempDir::new().unwrap();
    let source = directory.path().join("sum.py");
    fs::copy(fixture("sum.py"), &source).unwrap();
    fs::write(directory.path().join("sum.in1"), "1 2\n").unwrap();
    fs::write(directory.path().join("sum.in3"), "3 4\n").unwrap();
    let output = command(&directory)
        .arg("test")
        .arg(&source)
        .arg("--all")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&output.stdout), "3\n7\n");
}

#[test]
fn save_input_captures_piped_stdin() {
    let directory = TempDir::new().unwrap();
    let source = directory.path().join("sum.py");
    fs::copy(fixture("sum.py"), &source).unwrap();
    let mut child = command(&directory)
        .arg("exec")
        .arg(&source)
        .arg("--save-input")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.as_mut().unwrap().write_all(b"2 3\n").unwrap();
    drop(child.stdin.take());
    let output = child.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&output.stdout), "5\n");
    assert_eq!(
        fs::read_to_string(directory.path().join("sum.in1")).unwrap(),
        "2 3\n"
    );
}

#[test]
fn diff_and_stress_have_stable_exit_codes() {
    let directory = TempDir::new().unwrap();
    let source = directory.path().join("main.py");
    let brute = directory.path().join("brute.py");
    let generator = directory.path().join("generator.py");
    let expected = directory.path().join("expected.out");
    fs::write(&source, "print(int(input()))\n").unwrap();
    fs::write(&brute, "print(int(input()) + 1)\n").unwrap();
    fs::write(&generator, "import sys\nprint(sys.argv[1])\n").unwrap();
    fs::write(directory.path().join("main.in1"), "4\n").unwrap();
    fs::write(&expected, "4\n").unwrap();

    let diff = command(&directory)
        .arg("diff")
        .arg(&source)
        .arg("1")
        .arg(&expected)
        .status()
        .unwrap();
    assert!(diff.success());

    let stress = command(&directory)
        .arg("stress")
        .arg(&source)
        .arg("--brute")
        .arg(&brute)
        .arg("--generator")
        .arg(&generator)
        .arg("--limit")
        .arg("2")
        .arg("--timeout")
        .arg("1")
        .status()
        .unwrap();
    assert_eq!(stress.code(), Some(1));
    assert_eq!(
        fs::read_to_string(directory.path().join("main.in2")).unwrap(),
        "1\n"
    );
}

#[test]
fn timeout_returns_124() {
    let directory = TempDir::new().unwrap();
    let source = directory.path().join("slow.py");
    fs::write(&source, "import time\ntime.sleep(2)\n").unwrap();
    let status = command(&directory)
        .arg("exec")
        .arg(&source)
        .arg("--timeout")
        .arg("0.03")
        .stdin(Stdio::null())
        .status()
        .unwrap();
    assert_eq!(status.code(), Some(124));
}

#[test]
fn dynamic_shell_completion_uses_ranked_file_suggestions() {
    let directory = TempDir::new().unwrap();
    fs::write(directory.path().join("answer.cpp"), "int main() {}\n").unwrap();
    fs::write(directory.path().join("answer.txt"), "not a source\n").unwrap();
    let line = "run-cli exec ans";
    let output = command(&directory)
        .current_dir(directory.path())
        .arg("__complete")
        .arg("--line")
        .arg(line)
        .arg("--cursor")
        .arg(line.len().to_string())
        .output()
        .unwrap();
    assert!(output.status.success());
    let suggestions = String::from_utf8(output.stdout).unwrap();
    assert!(suggestions.starts_with("answer.cpp\t"), "{suggestions}");
    assert!(!suggestions.contains("answer.txt"));

    for shell in ["bash", "zsh", "fish"] {
        let generated = command(&directory)
            .arg("completions")
            .arg(shell)
            .output()
            .unwrap();
        assert!(generated.status.success());
        assert!(String::from_utf8_lossy(&generated.stdout).contains("__complete"));
    }
}

#[test]
fn competitive_companion_imports_paired_samples_without_overwriting() {
    let _loopback_guard = loopback_test_guard();
    let directory = TempDir::new().unwrap();
    let source = directory.path().join("main.py");
    fs::write(&source, "print(input())\n").unwrap();
    fs::write(directory.path().join("main.in1"), "existing\n").unwrap();
    let Some(port) = unused_loopback_port() else {
        return;
    };
    let child = command(&directory)
        .arg("companion")
        .arg(&source)
        .arg("--port")
        .arg(port.to_string())
        .arg("--wait")
        .arg("5")
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();

    let body = br#"{"name":"Pair Sum","group":"Example Round","url":"https://example.test/problem","interactive":false,"memoryLimit":256,"timeLimit":1000,"tests":[{"input":"2 3\n","output":"5\n"},{"input":"10 20\n","output":"30\n"}],"testType":"single","input":{"type":"stdin"},"output":{"type":"stdout"},"languages":{},"batch":{"id":"single","size":1}}"#;
    let mut stream = connect_with_retry(port);
    write!(
        stream,
        "POST / HTTP/1.1\r\nHost: localhost:{port}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    )
    .unwrap();
    stream.write_all(body).unwrap();
    stream.shutdown(Shutdown::Write).unwrap();
    let mut response = String::new();
    stream.read_to_string(&mut response).unwrap();
    let output = child.wait_with_output().unwrap();

    assert!(response.starts_with("HTTP/1.1 200 OK"), "{response}");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        fs::read(directory.path().join("main.in1")).unwrap(),
        b"existing\n"
    );
    assert_eq!(
        fs::read(directory.path().join("main.in2")).unwrap(),
        b"2 3\n"
    );
    assert_eq!(
        fs::read(directory.path().join("main.out2")).unwrap(),
        b"5\n"
    );
    assert_eq!(
        fs::read(directory.path().join("main.in3")).unwrap(),
        b"10 20\n"
    );
    assert_eq!(
        fs::read(directory.path().join("main.out3")).unwrap(),
        b"30\n"
    );
    assert!(String::from_utf8_lossy(&output.stderr).contains("Imported 2 sample(s)"));
}

#[test]
fn competitive_companion_collects_a_complete_contest_batch() {
    let _loopback_guard = loopback_test_guard();
    let directory = TempDir::new().unwrap();
    let source = directory.path().join("A.cpp");
    fs::write(
        &source,
        "#include <iostream>\nint main(){std::cout << 1; }\n",
    )
    .unwrap();
    let Some(port) = unused_loopback_port() else {
        return;
    };
    let child = command(&directory)
        .arg("companion")
        .arg(&source)
        .arg("--contest")
        .arg("--port")
        .arg(port.to_string())
        .arg("--wait")
        .arg("5")
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();

    let send = |name: &str, input: &str, output: &str| {
        let body = format!(
            r#"{{"name":"{name}","group":"Contest","url":"https://example.test/{name}","interactive":false,"memoryLimit":256,"timeLimit":1000,"tests":[{{"input":"{input}","output":"{output}"}}],"batch":{{"id":"contest-1","size":2}}}}"#
        );
        let mut stream = connect_with_retry(port);
        write!(
            stream,
            "POST / HTTP/1.1\r\nHost: localhost:{port}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        )
        .unwrap();
        stream.write_all(body.as_bytes()).unwrap();
        stream.shutdown(Shutdown::Write).unwrap();
        let mut response = String::new();
        stream.read_to_string(&mut response).unwrap();
        assert!(response.starts_with("HTTP/1.1 200 OK"), "{response}");
    };
    send("A - First", r#"1\n"#, r#"one\n"#);
    send("B - Second", r#"2\n"#, r#"two\n"#);

    let output = child.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(fs::read(directory.path().join("A.in1")).unwrap(), b"1\n");
    assert_eq!(fs::read(directory.path().join("A.out1")).unwrap(), b"one\n");
    assert_eq!(fs::read(directory.path().join("B.in1")).unwrap(), b"2\n");
    assert_eq!(fs::read(directory.path().join("B.out1")).unwrap(), b"two\n");
    assert!(directory.path().join("B.cpp").is_file());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("Imported contest batch 'contest-1'"),
        "{stderr}"
    );
    assert!(stderr.contains("A.cpp"), "{stderr}");
    assert!(stderr.contains("B.cpp"), "{stderr}");
}

#[test]
fn competitive_companion_contest_without_source_creates_a_workspace() {
    let _loopback_guard = loopback_test_guard();
    let directory = TempDir::new().unwrap();
    let Some(port) = unused_loopback_port() else {
        return;
    };
    let mut command = command(&directory);
    command.current_dir(directory.path());
    let child = command
        .arg("companion")
        .arg("--contest")
        .arg("--port")
        .arg(port.to_string())
        .arg("--wait")
        .arg("5")
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();

    let body = br#"{"name":"Standalone","tests":[{"input":"7\n","output":"49\n"}],"batch":{"id":"contest-single","size":1}}"#;
    let mut stream = connect_with_retry(port);
    write!(
        stream,
        "POST / HTTP/1.1\r\nHost: localhost:{port}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    )
    .unwrap();
    stream.write_all(body).unwrap();
    stream.shutdown(Shutdown::Write).unwrap();
    let mut response = String::new();
    stream.read_to_string(&mut response).unwrap();
    let output = child.wait_with_output().unwrap();

    assert!(response.starts_with("HTTP/1.1 200 OK"), "{response}");
    assert!(output.status.success());
    assert!(directory.path().join("A.cpp").is_file());
    assert_eq!(fs::read(directory.path().join("A.in1")).unwrap(), b"7\n");
    assert_eq!(fs::read(directory.path().join("A.out1")).unwrap(), b"49\n");
}
