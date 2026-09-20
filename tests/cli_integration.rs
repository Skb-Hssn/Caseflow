use std::fs;
use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};
use tempfile::TempDir;

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
