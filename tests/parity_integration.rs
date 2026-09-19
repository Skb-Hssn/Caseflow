#![cfg(unix)]

use std::fs;
use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::thread;
use std::time::{Duration, Instant};
use tempfile::TempDir;

fn binary() -> &'static str {
    env!("CARGO_BIN_EXE_run-cli")
}

fn legacy_runner() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("run.sh")
}

fn legacy_command() -> Command {
    let mut command = Command::new("bash");
    command.arg(legacy_runner());
    command
}

fn run_cli(directory: &Path) -> Command {
    let mut command = Command::new(binary());
    command
        .current_dir(directory)
        .arg("--color")
        .arg("never")
        .env("XDG_CACHE_HOME", directory.join("cache"))
        .env("XDG_CONFIG_HOME", directory.join("config"))
        .env("XDG_STATE_HOME", directory.join("state"));
    command
}

fn write_python(path: &Path) {
    fs::write(path, "a, b = map(int, input().split())\nprint(a + b)\n").unwrap();
}

fn run_with_input(mut command: Command, input: &[u8]) -> Output {
    let mut child = command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.as_mut().unwrap().write_all(input).unwrap();
    drop(child.stdin.take());
    child.wait_with_output().unwrap()
}

#[test]
fn batch_output_matches_run_sh_for_unusual_filenames() {
    let directory = TempDir::new().unwrap();
    let source = directory.path().join("answer [final] version.py");
    let input = directory.path().join("sample input [1].txt");
    let legacy_output = directory.path().join("legacy output [ok].txt");
    let modern_output = directory.path().join("modern output [ok].txt");
    write_python(&source);
    fs::write(&input, "19 23\n").unwrap();

    let legacy = legacy_command()
        .current_dir(directory.path())
        .env("NO_COLOR", "1")
        .arg(&source)
        .arg("--in")
        .arg(&input)
        .arg("--out")
        .arg(&legacy_output)
        .output()
        .unwrap();
    assert!(
        legacy.status.success(),
        "legacy runner failed: {}",
        String::from_utf8_lossy(&legacy.stderr)
    );

    let modern = run_cli(directory.path())
        .arg("exec")
        .arg(&source)
        .arg("--input")
        .arg(&input)
        .arg("--output")
        .arg(&modern_output)
        .output()
        .unwrap();
    assert!(
        modern.status.success(),
        "run-cli failed: {}",
        String::from_utf8_lossy(&modern.stderr)
    );
    assert_eq!(
        fs::read(&modern_output).unwrap(),
        fs::read(&legacy_output).unwrap()
    );
    assert_eq!(fs::read_to_string(modern_output).unwrap(), "42\n");
}

#[test]
fn concurrent_case_saves_are_unique_for_both_runners() {
    let directory = TempDir::new().unwrap();
    let modern_source = directory.path().join("modern.py");
    let legacy_source = directory.path().join("legacy.py");
    write_python(&modern_source);
    write_python(&legacy_source);

    let modern_threads = (0..4)
        .map(|index| {
            let directory = directory.path().to_path_buf();
            let source = modern_source.clone();
            thread::spawn(move || {
                let mut command = run_cli(&directory);
                command.arg("exec").arg(source).arg("--save-input");
                run_with_input(command, format!("{index} 1\n").as_bytes())
            })
        })
        .collect::<Vec<_>>();
    for handle in modern_threads {
        let output = handle.join().unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    let legacy_threads = (0..4)
        .map(|index| {
            let directory = directory.path().to_path_buf();
            let source = legacy_source.clone();
            thread::spawn(move || {
                let mut command = legacy_command();
                command
                    .current_dir(directory)
                    .env("NO_COLOR", "1")
                    .arg(source)
                    .arg("--save");
                run_with_input(command, format!("{index} 1\n").as_bytes())
            })
        })
        .collect::<Vec<_>>();
    for handle in legacy_threads {
        let output = handle.join().unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    for stem in ["modern", "legacy"] {
        let mut saved = (1..=4)
            .map(|id| fs::read_to_string(directory.path().join(format!("{stem}.in{id}"))).unwrap())
            .collect::<Vec<_>>();
        saved.sort();
        assert_eq!(saved, ["0 1\n", "1 1\n", "2 1\n", "3 1\n"]);
    }
}

#[test]
fn signal_is_forwarded_and_atomic_output_is_preserved() {
    let directory = TempDir::new().unwrap();
    let source = directory.path().join("slow.py");
    let output = directory.path().join("answer.out");
    let ready = directory.path().join("ready");
    fs::write(
        &source,
        format!(
            "import time\nopen({:?}, 'w').write('ready')\nprint('partial', flush=True)\ntime.sleep(30)\n",
            ready.to_string_lossy()
        ),
    )
    .unwrap();
    fs::write(&output, "keep\n").unwrap();
    let mut child = run_cli(directory.path())
        .arg("exec")
        .arg(&source)
        .arg("--output")
        .arg(&output)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    while !ready.exists() && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(10));
    }
    assert!(ready.exists(), "child did not start before signal test");
    unsafe {
        libc::kill(child.id() as i32, libc::SIGINT);
    }
    let status = child.wait().unwrap();
    assert_eq!(status.code(), Some(130));
    assert_eq!(fs::read_to_string(output).unwrap(), "keep\n");
}

#[test]
fn configuration_precedence_is_user_then_project_then_environment() {
    let directory = TempDir::new().unwrap();
    let source = directory.path().join("main.py");
    let input = directory.path().join("input.txt");
    write_python(&source);
    fs::write(&input, "3 4\n").unwrap();
    let user_config = directory.path().join("config/run-cli/config.toml");
    fs::create_dir_all(user_config.parent().unwrap()).unwrap();
    fs::write(
        &user_config,
        "[toolchains]\npython = 'missing-user-python'\n",
    )
    .unwrap();
    fs::write(
        directory.path().join(".run-cli.toml"),
        "[toolchains]\npython = 'python3'\n",
    )
    .unwrap();

    let project_wins = run_cli(directory.path())
        .env_remove("PYTHON")
        .arg("exec")
        .arg(&source)
        .arg("--input")
        .arg(&input)
        .output()
        .unwrap();
    assert!(project_wins.status.success());

    fs::write(
        directory.path().join(".run-cli.toml"),
        "[toolchains]\npython = 'missing-project-python'\n",
    )
    .unwrap();
    let environment_wins = run_cli(directory.path())
        .env("PYTHON", "python3")
        .arg("exec")
        .arg(&source)
        .arg("--input")
        .arg(&input)
        .output()
        .unwrap();
    assert!(environment_wins.status.success());
}

#[test]
fn clipboard_failures_do_not_replace_or_append_cases() {
    let directory = TempDir::new().unwrap();
    let source = directory.path().join("main.py");
    write_python(&source);
    fs::write(directory.path().join("main.in1"), "original\n").unwrap();
    let tools = directory.path().join("tools");
    fs::create_dir(&tools).unwrap();
    let failing_clipboard = tools.join("wl-paste");
    fs::write(&failing_clipboard, "#!/bin/sh\nexit 9\n").unwrap();
    let mut permissions = fs::metadata(&failing_clipboard).unwrap().permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&failing_clipboard, permissions).unwrap();
    let path = format!(
        "{}:{}",
        tools.display(),
        std::env::var("PATH").unwrap_or_default()
    );

    let modern = run_cli(directory.path())
        .env("PATH", &path)
        .arg("case")
        .arg("paste")
        .arg(&source)
        .arg("--next")
        .output()
        .unwrap();
    assert!(!modern.status.success());
    assert_eq!(
        fs::read_to_string(directory.path().join("main.in1")).unwrap(),
        "original\n"
    );
    assert!(!directory.path().join("main.in2").exists());

    let legacy = legacy_command()
        .current_dir(directory.path())
        .env("NO_COLOR", "1")
        .env("PATH", &path)
        .arg(&source)
        .arg("-pa")
        .output()
        .unwrap();
    assert!(!legacy.status.success());
    assert_eq!(
        fs::read_to_string(directory.path().join("main.in1")).unwrap(),
        "original\n"
    );
    assert!(!directory.path().join("main.in2").exists());
}
