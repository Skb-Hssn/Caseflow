# Native Competitive-Programming Runner CLI

## Summary

Build `run-cli`, a Linux-first Rust CLI that reimplements all `run.sh` capabilities while adding:

- An inline, scrollback-friendly REPL with slash commands, history, completion, source selection, and status.
- Structured one-shot commands for scripts and CI.
- Compatibility with existing `<stem>.in<ID>` test cases.
- Native process, timeout, resource, locking, clipboard, diff, and stress-test handling without invoking `run.sh`.

The interaction borrows the documented Codex CLI pattern of an interactive project session plus scriptable commands, but this product remains deterministic: no LLM, authentication, network access, or autonomous code editing. See the [official Codex CLI documentation](https://developers.openai.com/es-419/docs/codex/cli).

## Public Interface

### Launch behavior

- `run-cli` scans the current directory for supported sources and opens a fuzzy source picker.
- `run-cli A.cpp` opens an interactive session with `A.cpp` active.
- A source without an extension resolves to `.cpp`.
- Supported languages remain C++, Python, C, Java, Rust, Go, and Kotlin.

### REPL commands

- `/source [PATH]`: select or switch the active source.
- `/run [--timeout SEC] [--save] [--input FILE ...] [--output FILE ...]`: build when applicable and execute.
- `/build [--debug]`: compile without running; reject Python clearly.
- `/mode standard|debug`: set the session build mode.
- `/test all|last|ID[,ID...]`: run saved cases, compiling once for the group.
- `/case list|show|copy|delete|clear`: manage compatible `.in<ID>` files.
- `/case paste 1|next [--run]`: atomically paste clipboard data, optionally execute it.
- `/diff ID EXPECTED`: run a saved case and byte-compare its output.
- `/stress BRUTE GENERATOR [--limit N] [--timeout SEC]`: differential stress testing.
- `/status`, `/doctor`, `/help`, and `/quit`.

The prompt displays the active source, language, and build mode. Ctrl-C cancels an active child process and returns to the prompt; Ctrl-D exits an idle session.

### Context-aware file suggestions

- The REPL shows a selectable suggestion menu whenever the cursor is at a path argument, including `/source`, `/run --input`, `/run --output`, `/diff`, and both `/stress` source arguments. Tab completes the selected path; arrow keys move through candidates.
- Source positions prioritize supported source extensions. Input positions prioritize saved `.in<ID>` cases and common `.in`/`.txt` files. Expected-output positions prioritize `.out`, `.ans`, and `.txt` files.
- Output positions suggest directories and existing output-like files, while excluding the active source, selected inputs, duplicate outputs, and internal artifacts. Existing destinations are visibly marked as replacements and still pass the normal overwrite-safety checks.
- Suggestions support relative and absolute paths, `~`, nested-directory traversal, quoted paths containing spaces, and trailing `/` for directories. Hidden files appear only after the user types a leading `.`.
- Generated shell completions provide the same context-sensitive source, input, output, expected-output, and stress-helper suggestions for one-shot commands where the shell supports dynamic completion.

### Buttons and mouse behavior

- Source pickers, file-suggestion menus, and confirmation dialogs expose button-like actions such as `[ Select ]`, `[ Run ]`, `[ Delete ]`, and `[ Cancel ]` without turning the main interface into a full-screen dashboard.
- Buttons are keyboard-first: Tab and arrow keys move focus, Enter or Space activates the focused action, Esc cancels, and visible shortcut letters provide direct access. Every mouse action has an equivalent keyboard action.
- Mouse support is optional and disabled by default. Users can enable it through `/mouse on`, `--mouse`, or configuration; `/status` shows whether it is active.
- Mouse reporting is enabled only while `run-cli` owns an interactive picker or dialog. It is disabled before a child program receives the terminal and restored only after control returns to the REPL.
- Hover never performs an action. Clicks outside the current frame are ignored, and delete/clear operations still require an explicit confirmation after the click.
- Shift-drag remains available for terminal text selection where supported. Unsupported terminals, non-TTY execution, SSH sessions without mouse reporting, and CI automatically fall back to keyboard-only behavior.

### One-shot commands

- `run-cli exec SOURCE` with repeatable `--input`/`--output`, plus `--debug`, `--timeout`, and `--save-input`.
- `run-cli build SOURCE [--debug]`.
- `run-cli test SOURCE --all|--last|--id IDS [--debug]`.
- `run-cli case list|show|copy|paste|delete|clear SOURCE`.
- `run-cli diff SOURCE ID EXPECTED [--debug]`.
- `run-cli stress SOURCE --brute SOURCE --generator SOURCE [--limit N] [--timeout SEC]`.
- `run-cli doctor` and `run-cli completions SHELL`.
- Global `--color auto|always|never` and `--mouse`; `NO_COLOR` always wins, and mouse mode is never enabled for non-interactive output.

Decorative UI appears only on terminals. In pipelines, child stdout stays clean and runner diagnostics/resource reports go to stderr. Runtime exit codes propagate, timeout returns 124, Ctrl-C returns 130, argument/configuration errors return 2, and comparison/stress mismatches return 1.

## Implementation Changes

- Create a Rust binary using `clap` for one-shot parsing, a small `crossterm`-based line editor for the inline REPL, `serde`/`toml` for configuration, and Unix process APIs for process groups, signals, `wait4` resource metrics, and file locking. Owning the editor permits exact mouse hit-testing and deterministic terminal restoration.
- Separate the core into source resolution, language toolchains, build, execution, saved cases, comparison, stress testing, clipboard, configuration, and terminal presentation services. Both interfaces call the same services.
- Model operations with typed structures such as `Language`, `BuildMode`, `SourceSpec`, `RunRequest`, `RunReport`, `CaseSelector`, and `StressRequest`; UI code must not construct shell command strings.
- Implement one parser-aware path-suggestion service shared by the REPL and generated shell completions. It derives the expected path kind from the command and cursor position, reads the filesystem on demand so suggestions stay current, ranks relevant extensions first, and applies output-safety exclusions before rendering candidates.
- Track buttons as per-frame hit regions bound to typed UI actions rather than command strings. Recompute regions after every redraw or resize, ignore stale/out-of-bounds coordinates, and never interpolate mouse event data into paths or process arguments.
- Wrap raw mode, mouse reporting, cursor visibility, and screen changes in a terminal-state guard that restores the terminal during normal exit, errors, panics, Ctrl-C, and termination signals. Suspend the guard before handing the terminal to a child and restore a known state when the child exits.
- Preserve current compilation/debug behavior:
  - C/C++ warnings, sanitizers, and `LOCAL`.
  - Python `-X dev`.
  - Rust assertions/overflow checks, Go race/debug flags, and Java/Kotlin assertions.
  - Java package/main-class discovery and Kotlin executable JAR generation.
- Rebuild compiled sources once per top-level operation; multi-case and stress operations reuse that build. Store artifacts outside the source tree under `$XDG_CACHE_HOME/run-cli/build`, keyed by canonical source path and mode.
- Execute children in their own process group. Timeouts terminate the whole group, wait one second, then force-kill it. Report status, wall time, user/system CPU, CPU percentage, and peak RSS.
- Preserve case semantics exactly: positive numeric IDs, `next = max + 1`, no renumbering, newest mtime for `last` with highest ID as tie-breaker, and cases shared by source stem across languages.
- Use cross-process locks and same-directory temporary files plus atomic rename for case writes and output files. Failed clipboard reads or program runs leave existing files unchanged.
- Reject outputs that would overwrite a source, input, another output, or internal build artifact. Continue later batch inputs after a failure and return the final nonzero status.
- Preserve byte-for-byte diff/stress comparison. The generator receives the 1-based case number; the first mismatch or abnormal exit saves the failing input as the next case and prints a unified diff.
- Prefer `wl-copy`/`wl-paste`, then `xclip`; `/doctor` reports missing compilers, runtimes, and clipboard utilities.
- Prompt before `/case delete` and `/case clear`; one-shot equivalents require `--force`.
- Read global configuration from `$XDG_CONFIG_HOME/run-cli/config.toml` and the nearest `.run-cli.toml` above the active source. Precedence is CLI flags, environment, project config, user config, then defaults. The UI section includes `mouse = false` by default.
- Support existing environment overrides: `CXX`, `CC`, `PYTHON`, `JAVAC`, `JAVA`, `RUSTC`, `GO`, `KOTLINC`, `NO_COLOR`, `STRESS_LIMIT`, and `STRESS_TIMEOUT`. Config exposes toolchain commands, standard/debug flag arrays, default mode/timeouts, stress defaults, and color policy.
- Keep `run.sh` unchanged during migration. Existing `.in<ID>` files require no conversion; `.rn-build` is neither reused nor automatically deleted.

## Test and Acceptance Plan

The automated suite now includes PTY tests for REPL commands, resize handling,
Ctrl-C forwarding, mouse file selection, and terminal restoration. Differential
tests cover concurrent saves, signal behavior, configuration precedence,
clipboard failures, and unusual filenames. CI runs the suite with all supported
toolchains, including a non-Snap Kotlin installation, and tagged releases build
GNU and musl archives with SHA-256 checksums.

- Unit-test source resolution, extensionless C++, config precedence, duration parsing, case ordering, command construction, Java package parsing, output-safety checks, path-token parsing, candidate filtering, and suggestion ranking.
- Add isolated integration fixtures for all seven languages and both build modes. Use fake toolchains for deterministic failure tests and a CI image containing the real toolchains for end-to-end coverage.
- Differentially exercise Rust services and `run.sh` in temporary directories to verify saved-case naming, ordering, clipboard semantics, batch continuation, output atomicity, strict diffing, and stress mismatch capture.
- Add PTY tests for welcome/status rendering, slash completion, automatic path suggestions, keyboard and mouse button activation, paths with spaces, directory traversal, history, interactive stdin handoff, saved-input capture, Ctrl-C cancellation, Ctrl-D exit, and terminal restoration.
- Verify clicks cannot activate stale regions after resize, clicks outside controls are ignored, destructive clicks require confirmation, mouse mode is absent in non-TTY execution, and terminal mouse reporting is disabled after normal exit, panic, signal, timeout, or child-process completion.
- Test concurrent case saves/deletes, timeout cleanup of descendant processes, missing tools, empty input, absent final newline, paths containing spaces, duplicate outputs, compiler failure, runtime signals, and failed clipboard access.
- Snapshot terminal output with colors disabled and multiple widths; verify non-TTY mode emits unpolluted program stdout.
- MVP acceptance requires complete `run.sh` feature parity, successful use of existing `.in<ID>` files, deterministic REPL/one-shot behavior, and `cargo install --path .` producing the `run-cli` executable.

## Assumptions

- The MVP targets Linux only and uses a native Rust implementation.
- “Like Gemini/Codex/Claude CLI” refers to terminal interaction quality, not AI functionality.
- The initial release is a focused competitive-programming tool rather than a general repository build system.
- No arbitrary shell escape, plugin system, remote execution, session sharing, or Windows/macOS support is included in the MVP.
