use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
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
