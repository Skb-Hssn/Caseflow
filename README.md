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

Interactive sessions use the terminal's alternate screen, like Vim. Exiting
with `/exit` or Ctrl-D restores the previous terminal contents. Terminal state
is also restored after handled signals, errors, and panics. One-shot commands
continue to print in the normal terminal.

The compact session header stays fixed at the top, while mouse actions and the
command line stay fixed at the bottom. Each submitted command starts a distinct
block in the middle viewport, and older blocks are dimmed. Scroll the retained
output with the mouse wheel without moving the surrounding UI. Interactive
stdin is retained in a labeled `Input` block after the program exits.
Ctrl-C stops the active execution and returns to the prompt. If the program
ignores the interrupt, run-cli escalates to termination and then a forced stop
instead of waiting forever.

To keep runaway print loops responsive, the REPL streams a bounded output
prefix, suppresses the middle, and retains a small final tail containing the
execution status. One-shot commands and explicit `--output` files are not
truncated. Inherited REPL output uses a pseudo-terminal, so line-buffered C,
C++, Python, and similar programs display completed lines while still running.

Run `run-cli` without a source to open a fuzzy source picker: type any
subsequence of the path to filter it, use arrows to move, and press Enter to
select. Inside a session:

```text
/run
/run clipboard
/run --input sample.in --output answer.out
/run --save
/debug
/debug on
/build
/test                       # all saved cases
/test 1 3
/test last
/case list
/case add                   # create the next case in an external editor
/case edit 2                # edit an existing case
/case paste --run
/compare 1 expected.out
/stress brute.cpp generator.py --runs 500 --timeout 2
/companion                  # wait for samples from the browser extension
/open B.cpp
/again
/clear
/status
/exit
```

`/run` uses interactive terminal input. `/run clipboard` reads the clipboard,
stores it as the next case ID, and runs that saved case. `/test` runs all saved
cases by default; use `last` or one or more space- or comma-separated IDs to
narrow the selection. When `A.out<ID>` exists beside `A.in<ID>`, `/test`
compares the generated output byte for byte and shows per-case plus aggregate
PASS/FAIL verdicts. Cases without expected output remain run-only. `/again`
repeats the most recent run or test, while `/clear` clears the retained output
viewport.

`/debug` toggles debug mode. The explicit forms `/debug on`, `/debug off`, and
`/debug toggle` are useful in history and scripts. `/open` switches the active
source and opens the source picker when its path is omitted.

Typing `/` opens a dimmed command menu immediately; continue typing to filter
it or press Tab to insert the highlighted full command. Options and files also
appear passively when they become relevant after a command; Tab accepts the
highlighted choice. Source positions prioritize
supported language files, inputs prioritize saved cases and `.in`/`.txt` files,
and expected output positions prioritize `.out`, `.ans`, and `.txt` files. Paths
containing spaces are quoted automatically. Completion lists are scrollable,
resize-aware, and capped to the available terminal height.

Source pickers, file-completion menus, and destructive confirmations have keyboard-accessible `[ Select ]`, `[ Delete ]`, and `[ Cancel ]` controls. Enable optional mouse clicks with `run-cli --mouse`, `/mouse on`, or `ui.mouse = true` in configuration. Mouse mode adds a fixed bottom action bar with `Interactive` and `Clipboard` under `Run`, an on/off debug toggle, each saved case number under `Test`, and `All`. Clipboard run saves the clipboard as the next case before executing it, so it can be rerun from its numbered button. Mouse reporting is enabled only while the REPL owns the terminal, disabled while a submitted program owns it, and cleaned up on every normal, error, panic, or handled-signal exit. The build action remains available as `/build` rather than occupying the action bar.

Clipboard and saved-case runs use a single grey `File` block with orange
`Input` and `Output` labels, followed by a dim grey divider and a compact,
dim-green one-line `System status` summary.

Use the action bar's `Select` button to release the mouse to the terminal, drag
over any visible text, and copy with `Ctrl+Shift+C`; the wheel, arrow keys, and
Page Up/Down continue scrolling the output. Press `Esc` to restore run-cli
mouse controls. Terminals that support the standard override can also select
immediately with Shift-drag.

The toolbar uses compact labels on narrow terminals: `I` is interactive run,
`C` is clipboard run, `D` toggles debug mode, `S` is native text selection,
and `A` tests all saved cases.

## One-shot commands

```sh
# Interactive or piped stdin
run-cli run A.cpp
printf '2 3\n' | run-cli run A.cpp

# Save clipboard input as the next case, then run it
run-cli run A.cpp --clipboard

# Named input and atomic output
run-cli run A.cpp --input sample.in --output answer.out

# Batch input/output pairs
run-cli run A.cpp \
  --input first.in --output first.out \
  --input second.in --output second.out

# Build modes
run-cli build A.cpp
run-cli build A.cpp --debug
run-cli run A.cpp --debug --timeout 2

# Saved cases
run-cli test A.cpp
run-cli test A.cpp 1 3
run-cli test A.cpp last
run-cli case list A.cpp
run-cli case show A.cpp 1
run-cli case copy A.cpp 1
run-cli case add A.cpp
run-cli case edit A.cpp 2
run-cli case paste A.cpp --run
run-cli case paste A.cpp 2
run-cli case delete A.cpp 1 --force
run-cli case clear A.cpp --force

# Compare and stress
run-cli compare A.cpp 1 expected.out
run-cli stress A.cpp brute.cpp generator.py --runs 500

# Import one problem from Competitive Companion
run-cli companion A.cpp

# Shell completion
run-cli completion zsh > ~/.zfunc/_run-cli
run-cli completion bash > ~/.local/share/bash-completion/completions/run-cli
run-cli completion fish > ~/.config/fish/completions/run-cli.fish
```

Bash, Zsh, and Fish integrations call the same parser-aware filtering and
ranking engine used by the REPL, so source, input, output, and expected-file
suggestions remain consistent in both interfaces.

When stdout is redirected, `run-cli` leaves it exclusively for program output. Build messages, errors, and resource measurements go to stderr. An explicit output file is replaced atomically only after a successful run.

## Competitive Companion

Start a receiver for the source you want to test, then click
[Competitive Companion](https://github.com/jmerle/competitive-companion)'s
green `+` button on a single problem page:

```sh
# Inside the full-screen session
/companion

# Or as a one-shot command
run-cli companion A.cpp
```

The default port is `10043`, one of Competitive Companion's built-in ports.
For a custom extension port or a different wait time, use
`/companion --port 4244 --wait 300` (or the same options after the one-shot
command). The listener accepts only local loopback connections, stops after
one problem, and can be cancelled with Ctrl-C. Send a single problem page;
multi-problem contest batches are rejected so samples from different problems
cannot be mixed into one source.

Each sample appends a matched pair such as `A.in3` and `A.out3`. Existing IDs
are never overwritten. The `.out<ID>` file preserves the sample's expected
output. `/test` automatically compares generated output with it byte for byte
and returns a failure status if any judged case differs.

## Saved cases

Cases remain beside the source stem and are shared across languages:

```text
A.cpp
A.py
A.in1
A.in2
A.out1
A.out2
```

- IDs are positive integers.
- A new case uses `max(existing ID) + 1`; gaps are not renumbered or reused.
- `last` selects the newest modification time, breaking ties with the highest ID
  (the legacy `--last` spelling remains accepted).
- Concurrent writers use a shared file lock.
- Competitive Companion imports expected output as `.out<ID>`; deleting or
  clearing an imported input also removes its matched output.
- Tests automatically judge a case when its matching `.out<ID>` is present;
  cases without one continue to run without a verdict.
- Clipboard access prefers `wl-copy`/`wl-paste`, then `xclip`.
- In the REPL, `/run clipboard` is the short form of saving the clipboard as
  the next case and immediately running it.
- `/case add` creates the next case and `/case edit ID` updates an existing
  case using `$VISUAL`, `$EDITOR`, `nvim`, or `vim`, in that order. A failed or
  cancelled editor leaves saved cases unchanged.

Interactive deletion asks for confirmation. One-shot deletion and clearing require `--force`.

## Compatibility aliases

Existing scripts continue to work while help and completion advertise the
shorter canonical vocabulary:

- `run-cli exec` is an alias for `run-cli run`, and `run-cli diff` is an alias
  for `run-cli compare`.
- `/source` aliases `/open`; `/mode debug` and `/mode standard` alias
  `/debug on` and `/debug off`.
- `/diff` aliases `/compare`.
- `/quit` aliases `/exit`.
- Legacy test flags `--all`, `--last`, and `--id`, clipboard targets `next`,
  `--next`, and `--id`, and stress flags `--brute`, `--generator`, and
  `--limit` remain accepted.
- `run-cli completions` remains an alias for `run-cli completion`.

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

Press Ctrl-C at any time to stop the complete stress session. run-cli cancels
the active generator, candidate, or brute process, skips all remaining cases,
and does not save the interrupted input as a mismatch.

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
