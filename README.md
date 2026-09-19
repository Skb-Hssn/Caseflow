# run-cli

`run-cli` is a Linux-first competitive-programming runner with two interfaces:

- An inline REPL with slash commands, history, completion, file suggestions, source selection, and optional mouse controls.
- Structured one-shot commands for shell scripts, editor tasks, and CI.

It supports C++, Python, C, Java, Rust, Go, and Kotlin and reads the same `<stem>.in<ID>` saved cases as [`run.sh`](run.sh).

## Install

Download a prebuilt GNU or musl archive from the GitHub Releases page, or build
from source with Rust 1.74 or newer:

```sh
cargo install --path .
run-cli doctor
```

Build without installing:

```sh
cargo build --release
./target/release/run-cli --help
```

## Interactive use

Open a session for a source:

```sh
run-cli A.cpp
```

Run `run-cli` without a source to open a fuzzy source picker: type any
subsequence of the path to filter it, use arrows to move, and press Enter to
select. Inside a session:

```text
/run
/run --input sample.in --output answer.out
/run --save
/mode debug
/build
/test all
/test 1,3
/case list
/case paste next --run
/diff 1 expected.out
/stress brute.cpp generator.py --limit 500 --timeout 2
/source B.cpp
/status
/quit
```

Press Tab at command and path positions to open context-aware suggestions. Source positions prioritize supported language files, inputs prioritize saved cases and `.in`/`.txt` files, and expected output positions prioritize `.out`, `.ans`, and `.txt` files. Paths containing spaces are quoted automatically. Completion lists are scrollable, resize-aware, and capped to the available terminal height.

Source pickers, file-completion menus, and destructive confirmations have keyboard-accessible `[ Select ]`, `[ Delete ]`, and `[ Cancel ]` controls. Enable optional mouse clicks with `run-cli --mouse`, `/mouse on`, or `ui.mouse = true` in configuration. Mouse reporting is enabled only while an interactive control owns it, disabled while a submitted program owns the terminal, and cleaned up on every normal, error, panic, or handled-signal exit.

## One-shot commands

```sh
# Interactive or piped stdin
run-cli exec A.cpp
printf '2 3\n' | run-cli exec A.cpp

# Named input and atomic output
run-cli exec A.cpp --input sample.in --output answer.out

# Batch input/output pairs
run-cli exec A.cpp \
  --input first.in --output first.out \
  --input second.in --output second.out

# Build modes
run-cli build A.cpp
run-cli build A.cpp --debug
run-cli exec A.cpp --debug --timeout 2

# Saved cases
run-cli test A.cpp --all
run-cli test A.cpp --id 1,3
run-cli test A.cpp --last
run-cli case list A.cpp
run-cli case show A.cpp 1
run-cli case copy A.cpp 1
run-cli case paste A.cpp --next --run
run-cli case delete A.cpp 1 --force
run-cli case clear A.cpp --force

# Compare and stress
run-cli diff A.cpp 1 expected.out
run-cli stress A.cpp --brute brute.cpp --generator generator.py

# Shell completion
run-cli completions zsh > ~/.zfunc/_run-cli
run-cli completions bash > ~/.local/share/bash-completion/completions/run-cli
run-cli completions fish > ~/.config/fish/completions/run-cli.fish
```

Bash, Zsh, and Fish integrations call the same parser-aware filtering and
ranking engine used by the REPL, so source, input, output, and expected-file
suggestions remain consistent in both interfaces.

When stdout is redirected, `run-cli` leaves it exclusively for program output. Build messages, errors, and resource measurements go to stderr. An explicit output file is replaced atomically only after a successful run.

## Saved cases

Cases remain beside the source stem and are shared across languages:

```text
A.cpp
A.py
A.in1
A.in2
```

- IDs are positive integers.
- A new case uses `max(existing ID) + 1`; gaps are not renumbered or reused.
- `--last` selects the newest modification time, breaking ties with the highest ID.
- Concurrent writers use a shared file lock.
- Clipboard access prefers `wl-copy`/`wl-paste`, then `xclip`.

Interactive deletion asks for confirmation. One-shot deletion and clearing require `--force`.

## Configuration

Configuration is loaded in this order, from lowest to highest precedence:

1. Built-in defaults
2. `$XDG_CONFIG_HOME/run-cli/config.toml` (or `~/.config/run-cli/config.toml`)
3. The nearest `.run-cli.toml` above the active source
4. Environment variables
5. Command-line flags

Copy [`.run-cli.toml.example`](.run-cli.toml.example) to start a project configuration. Supported environment overrides are `CXX`, `CC`, `PYTHON`, `JAVAC`, `JAVA`, `RUSTC`, `GO`, `KOTLINC`, `NO_COLOR`, `STRESS_LIMIT`, and `STRESS_TIMEOUT`.

Build artifacts, locks, temporary outputs, and REPL history are stored below the appropriate XDG cache/state directories rather than beside source files.

## Stress protocol

The generator receives the 1-based case number as its first argument and writes an input case to stdout. For every case, `run-cli`:

1. Runs the generator with the configured timeout.
2. Sends its output to both the main program and brute-force solution.
3. Compares stdout byte for byte.
4. Saves the first mismatch or abnormal-exit input as the next `.in<ID>` case.
5. Prints a unified diff and exits with status 1.

Defaults are 1,000 cases and two seconds for each process.

## Exit status

- Successful operation: `0`
- Diff or stress mismatch: `1`
- Invalid command or configuration: `2`
- Timed out program: `124`
- Interrupted program: `130`
- Otherwise, compiler and program exit statuses are propagated.

## Development

```sh
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test --all-targets
scripts/package-release.sh
```

`run.sh` remains unchanged as the compatibility reference during migration.
CI runs unit, parity, PTY, and all available real-toolchain tests. Its clean
Ubuntu environment installs Kotlin directly, avoiding the local Snap/AppArmor
failure that can affect sandboxed `kotlinc` installations.
