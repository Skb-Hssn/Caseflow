# Caseflow

Caseflow is a Linux-first competitive-programming runner for building,
executing, testing, and debugging solutions without leaving the terminal. It
combines a full-screen interactive workspace with predictable one-shot
commands for scripts, editor tasks, and CI.

> **Command name:** the project is named **Caseflow**; the installed executable
> is currently named **`run-cli`**.

Caseflow supports C++, C, Python, Java, Rust, Go, and Kotlin. It manages saved
test cases, automatically judges paired expected outputs, imports samples from
Competitive Companion, performs differential stress testing, and keeps build
artifacts outside the solution directory.

## Highlights

- Full-screen REPL with a fixed header, scrollable output, command history,
  fuzzy source selection, and context-aware suggestions.
- Optional mouse toolbar for interactive runs, clipboard runs, saved tests,
  debug mode, scrolling, and native text selection.
- Script-friendly commands with clean stdout, diagnostics on stderr, and
  meaningful exit statuses.
- Saved cases using the familiar `<stem>.in<ID>` convention.
- Automatic byte-for-byte judging when `<stem>.out<ID>` exists.
- Competitive Companion sample import through a loopback-only HTTP receiver.
- Differential stress testing with automatic failing-case preservation.
- Standard and debug builds for seven languages.
- Process-group timeouts and Ctrl-C escalation that also stop child processes.
- Atomic output writes and cross-process locking for saved cases.

## Contents

- [Requirements](#requirements)
- [Installation](#installation)
- [Quick start](#quick-start)
- [Supported languages](#supported-languages)
- [Interactive workspace](#interactive-workspace)
- [Saved cases and verdicts](#saved-cases-and-verdicts)
- [One-shot commands](#one-shot-commands)
- [Clipboard workflows](#clipboard-workflows)
- [Competitive Companion](#competitive-companion)
- [Stress testing](#stress-testing)
- [Configuration](#configuration)
- [Shell completion](#shell-completion)
- [Compatibility aliases](#compatibility-aliases)
- [Safety and process behavior](#safety-and-process-behavior)
- [Exit statuses](#exit-statuses)
- [Troubleshooting](#troubleshooting)
- [Development](#development)
- [License](#license)

## Requirements

Caseflow itself is a native Rust program, but each language requires its own
compiler or runtime. Install only the tools you use.

| Language | Extension | Default tools         |
| -------- | --------- | --------------------- |
| C++      | `.cpp`  | `g++`               |
| C        | `.c`    | `gcc`               |
| Python   | `.py`   | `python3`           |
| Java     | `.java` | `javac`, `java`   |
| Rust     | `.rs`   | `rustc`             |
| Go       | `.go`   | `go`                |
| Kotlin   | `.kt`   | `kotlinc`, `java` |

Optional integrations use:

- `wl-copy` and `wl-paste` on Wayland, or `xclip` on X11, for clipboard
  operations.
- `$VISUAL`, `$EDITOR`, `nvim`, or `vim` for `/edit`, `/case add`, and
  `/case edit`.
- `diff` for unified output from the explicit `compare` command. Caseflow falls
  back to printing both files if `diff` is unavailable.
- A terminal with ANSI escape-sequence support for the full-screen workspace.

After installation, inspect the available toolchains and clipboard support:

```sh
run-cli doctor
```

## Installation

### Prebuilt Linux archive

Tagged releases produce archives for:

- `x86_64-unknown-linux-gnu`
- `x86_64-unknown-linux-musl`

Download the archive for your system from the
[GitHub Releases page](https://github.com/Skb-Hssn/Caseflow/releases), extract
it, and place `run-cli` somewhere on `PATH`:

```sh
tar -xzf run-cli-v*-x86_64-unknown-linux-gnu.tar.gz
install -Dm755 run-cli-v*-x86_64-unknown-linux-gnu/run-cli \
  "$HOME/.local/bin/run-cli"
```

Ensure `~/.local/bin` is on `PATH`, then verify the installation:

```sh
run-cli --version
run-cli doctor
```

Use the musl archive instead when that target is more suitable for your Linux
distribution.

### Install from source

Install a current stable Rust toolchain, then clone and install the project:

```sh
git clone https://github.com/Skb-Hssn/Caseflow.git
cd Caseflow
cargo install --path . --locked
run-cli --version
run-cli doctor
```

By default, Cargo installs the executable into `~/.cargo/bin`.

### Build without installing

For local development or a repository-scoped binary:

```sh
cargo build --release
./target/release/run-cli --help
```

Throughout this README, replace `run-cli` with
`./target/release/run-cli` when using the local release binary.

## Quick start

Given a solution named `A.cpp`, open the interactive workspace:

```sh
run-cli A.cpp
```

To start without an existing source and download a complete contest:

```sh
run-cli --no-source
```

At the source-less prompt, run:

```text
/contest contests/spring-round
```

The contest receiver creates `A.cpp`, `B.cpp`, and the corresponding sample
case files in that directory, then opens the normal workspace on `A.cpp`.
Until the contest has been received, a source-less session accepts `/edit`,
`/contest`, `/help`, and `/exit` (or `/quit`). Ctrl-D also exits.

Then enter:

```text
/run
```

Type the program input normally. Press Ctrl-C to stop a running program and
return to Caseflow.

To create and run saved cases:

```text
/case add
/test
```

`/case add` opens the next input in your configured terminal editor. You can
also create cases manually:

```sh
printf '2 3\n' > A.in1
printf '5\n' > A.out1
run-cli test A.cpp
```

Because `A.out1` exists, Caseflow judges the generated output and prints a
verdict:

```text
✓ Case #1 verdict: PASS · matches A.out1
✓ Verdict: PASS · 1/1 judged case(s) passed
```

For a direct, non-interactive run:

```sh
printf '2 3\n' | run-cli run A.cpp
```

If the source argument has no extension, Caseflow treats it as C++:

```sh
run-cli A       # resolves to A.cpp
```

Running `run-cli` without a source scans the current directory and opens a
fuzzy source picker.

## Supported languages

Caseflow resolves the language from the source extension. Compiled artifacts
are stored below the XDG cache directory, not beside the source.

| Language | Standard mode                                      | Debug mode                                                      |
| -------- | -------------------------------------------------- | --------------------------------------------------------------- |
| C++      | C++17, warnings,`LOCAL`                          | Debug symbols, libstdc++ checks, ASan, UBSan, stronger warnings |
| C        | C17 and warnings                                   | Debug symbols, ASan, UBSan, stronger warnings                   |
| Python   | `python3 SOURCE`                                 | `python3 -X dev SOURCE`                                       |
| Rust     | Rust 2021                                          | Debug information, assertions, overflow checks                  |
| Go       | Normal`go build`                                 | Race detector and disabled optimizations/inlining               |
| Java     | Compiled classes and discovered package/class name | Debug information, linting, assertions                          |
| Kotlin   | Executable JAR                                     | Kotlin/JVM debug options and assertions                         |

Use standard mode for normal submissions and debug mode while diagnosing
undefined behavior or assertion failures:

```sh
run-cli run A.cpp --debug
run-cli build A.cpp --debug
```

Inside a session, toggle the persistent mode with `/debug` or set it explicitly
with `/debug on` and `/debug off`.

Python does not have a separate compilation step, so `run-cli build A.py` is
rejected with an explanatory message. Use `run-cli run A.py` instead.

## Interactive workspace

### Starting a session

```sh
# Choose from supported sources in the current directory
run-cli

# Open a specific source
run-cli A.cpp

# Disable mouse controls for this session
run-cli --no-mouse A.cpp

# Start in debug mode
run-cli --debug A.cpp
```

The workspace uses the terminal's alternate screen, like Vim. A single fixed
header shows `Caseflow`, its version, and the active source on the left, with
the language, build mode, and mouse state aligned on the right. Program output
and diagnostics occupy a scrollable middle viewport. Framing rules, the action
bar, and the `caseflow:SOURCE ›` command prompt stay fixed at the bottom.
`/exit` or Ctrl-D restores the previous terminal contents.

Terminal state is also restored after handled errors, panics, and termination
signals.

### Slash commands

| Command                               | Purpose                                                                                                                                       |
| ------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------- |
| `/run [interactive] [OPTIONS]`      | Build when needed and run with terminal input.                                                                                                |
| `/run clipboard [OPTIONS]`          | Append clipboard input as the next saved case and run it.                                                                                     |
| `/build`                            | Compile the active source without running it.                                                                                                 |
| `/test [all\|last\|ID ...]`           | Run all cases or selected saved-case IDs.                                                                                                     |
| `/case list`                        | List saved inputs for the active source stem.                                                                                                 |
| `/case show ID`                     | Print a saved input.                                                                                                                          |
| `/case copy ID`                     | Copy a saved input to the clipboard.                                                                                                          |
| `/case paste [ID] [--run]`          | Append clipboard input, or replace a specified ID.                                                                                            |
| `/case add`                         | Create the next saved input in an external editor.                                                                                            |
| `/case edit ID`                     | Edit an existing input in an external editor.                                                                                                 |
| `/edit PATH`                        | Edit any file in an external editor, creating the file when missing.                                                                          |
| `/case delete ID`                   | Confirm and delete an input and its paired expected output.                                                                                   |
| `/case clear`                       | Confirm and delete every case for the active source.                                                                                          |
| `/compare ID EXPECTED`              | Run one case and compare it with an arbitrary expected file.                                                                                  |
| `/stress BRUTE GENERATOR [OPTIONS]` | Differentially test against a trusted solution.                                                                                               |
| `/contest [OPTIONS]`                | Download a complete contest using the active source as the first problem. In a source-less session, it also accepts an optional`DIRECTORY`. |
| `/companion [contest] [OPTIONS]`    | Import one problem or a complete Competitive Companion contest batch.                                                                         |
| `/open [PATH]`                      | Switch source; omit the path to open the fuzzy picker.                                                                                        |
| `/debug [on\|off\|toggle]`            | Inspect or change the session build mode.                                                                                                     |
| `/mouse [on\|off\|toggle]`            | Enable or disable mouse controls.                                                                                                             |
| `/again`                            | Repeat the latest`/run` or `/test`.                                                                                                       |
| `/clear`                            | Clear retained output from the viewport.                                                                                                      |
| `/status`                           | Show the active source, language, mode, cases, mouse state, and cache.                                                                        |
| `/doctor`                           | Inspect configured compilers, runtimes, and clipboard tools.                                                                                  |
| `/help [COMMAND]`                   | Show general or command-specific help.                                                                                                        |
| `/exit`                             | Leave Caseflow and restore the terminal.                                                                                                      |

Run options inside the workspace are the same as the corresponding one-shot
options after the source argument:

```text
/run --timeout 2
/run --save
/run --input sample.in
/run --input sample.in --output answer.out
/run clipboard --timeout 2
```

### Run versus test

Use `/run` for one execution with interactive input, clipboard input, or an
explicit input file. Use `/test` for repeatable saved cases discovered from
`<stem>.in<ID>` files. A test also checks the matching `<stem>.out<ID>` when
that file exists. `/run clipboard` is the bridge between both workflows: it
appends the clipboard as the next saved case and immediately runs that case.

### Suggestions and history

Typing `/` immediately opens a dimmed command menu. Continue typing to filter
it, or press Tab to insert the highlighted command. Options and files appear
automatically when the cursor reaches a relevant argument.

- Source positions prioritize supported language files.
- Input positions prioritize `.in<ID>`, `.in`, and `.txt` files.
- Expected-output positions prioritize `.out`, `.ans`, and `.txt` files.
- Output positions suggest safe output-like destinations.
- Nested directories, absolute paths, `~`, hidden files, and paths containing
  spaces are supported.
- Paths containing spaces are quoted automatically.

History is persisted at `$XDG_STATE_HOME/run-cli/history.txt` and retains the
latest 1,000 commands.

### Keyboard controls

| Key                         | Behavior                                                            |
| --------------------------- | ------------------------------------------------------------------- |
| `Tab`                     | Accept a suggestion or activate a suggestion list.                  |
| `Up` / `Down`           | Navigate suggestions, or command history when no menu is open.      |
| `Enter`                   | Submit the typed command; in an active picker, select the item.     |
| `Esc`                     | Close suggestions, cancel a picker, or leave native selection mode. |
| `Ctrl-A` / `Ctrl-E`     | Move to the beginning or end of the command line.                   |
| `Home` / `End`          | Move to the beginning or end; in selection mode, scroll fully.      |
| `Page Up` / `Page Down` | Scroll retained output in selection mode.                           |
| `Ctrl-C`                  | Clear an idle command or cancel the active run/test batch.          |
| `Ctrl-D`                  | Exit when the command line is empty.                                |

### Mouse controls and text selection

Mouse controls are enabled by default in interactive sessions. Disable them for
one session with:

```sh
run-cli --no-mouse A.cpp
```

```text
/mouse off
```

```toml
[ui]
mouse = false
```

`--mouse` remains available as an explicit override, which is useful when a
user or project configuration has disabled mouse controls. Inside a running
session, `/mouse on`, `/mouse off`, and `/mouse toggle` take effect immediately.

The fixed toolbar exposes the following compact controls:

- **Run `[Int]`** starts an interactive run.
- **Run `[Clip]`** appends clipboard input as a case and runs it.
- **Run `[Again]`** repeats the latest run or test with its original options.
- **Test `[+Case]`** opens the configured editor to create the next saved case.
- **Test `[1]`, `[2]`, ... `[All]`** runs one saved case or every case.
- **Debug `[Off]` / `[On]`** toggles the persistent build mode.
- **`[Select]`** releases the mouse for native terminal selection and copying.
- **`[More]`** opens the overflow action menu.

Controls are rendered as bracketed terminal text rather than filled buttons.
Run, add-case, and More actions use the primary blue accent; active debug mode
is green; numbered tests and unavailable controls use muted slate; and `[All]`
uses the main text color. On wide terminals, subtle vertical dividers and
flexible spacing separate the Run, Test, Debug, selection, and More groups.
Only the output viewport scrolls.

`[Again]` remains neutral until a repeatable command exists. `[More]` opens a
bordered panel on the right side of the output viewport. Its actions are
ordered around common tasks: source switching, editing the active file or a
saved case, output and session utilities, Companion imports, deletion,
diagnostics, help, and exit. It also exposes Session status and Check tools
without adding permanent toolbar buttons. Choose with the mouse or with
Up/Down and Enter; Esc closes the panel. Edit case and Delete case open a
second saved-case picker, and deletion still requires confirmation.

The toolbar becomes more compact on narrow terminals: `[Again]`, `[+Case]`,
and `[More]` become `[R]`, `[+]`, and `[…]`, then single-character controls at
the narrowest supported width. Numbered tests that do not fit are represented
by an ellipsis while `[All]` remains available. Mouse reporting is enabled only
while Caseflow owns the workspace and is disabled while a child program or
external editor owns the terminal.

Choose **Select**, drag over visible text, and copy with your terminal's normal
shortcut, commonly Ctrl-Shift-C. The wheel, arrow keys, Page Up/Down, Home, and
End continue to navigate retained output. Press Esc to return to Caseflow mouse
controls. Many terminals also support Shift-drag as a direct override.

### Live output and runaway programs

Caseflow streams output while the child is running, including line-buffered C,
C++, and Python output. The interactive safety limiter prevents an infinite
print loop from overwhelming the terminal:

- The live prefix is capped at 16 KiB and a line budget of at least 256 lines.
- Once suppressed, the final 8 KiB is retained and shown after the child exits.
- The scrollable workspace retains up to 20,000 lines across commands.
- One-shot output and explicit output files are not truncated.

When the limit is reached, press Ctrl-C once. Caseflow signals the complete
child process group, escalates to termination and then forced termination when
necessary, and returns to the prompt.

To preserve complete output explicitly:

```text
/run --input A.in1 --output full.out
```

## Saved cases and verdicts

### File layout

Cases live beside the source and are shared by files with the same stem:

```text
A.cpp
A.py
A.in1
A.out1
A.in2
A.out2
A.in3
```

`A.cpp` and `A.py` both use `A.in1`, `A.in2`, and `A.in3`.

- Input IDs are positive integers.
- New cases use `max(existing input ID) + 1`.
- Gaps are not renumbered or reused.
- `last` selects the most recently modified input, breaking ties with the
  highest ID.
- Concurrent Caseflow writers share a file lock.

### Creating cases

Use an external editor:

```text
/case add
/case edit 2
```

```sh
run-cli case add A.cpp
run-cli case edit A.cpp 2
```

Editor selection order is `$VISUAL`, `$EDITOR`, `nvim`, then `vim`. The edit is
staged in the cache and committed atomically only when the editor exits
successfully. A failed or cancelled edit leaves the saved case unchanged.

Edit any project file without leaving Caseflow:

```text
/edit A.cpp
/edit "notes with spaces.txt"
```

`/edit` opens existing files directly and creates an empty file first when the
target is missing. Its parent directory must already exist. The Caseflow
workspace is suspended while the editor owns the terminal and restored when
the editor exits.

Use the clipboard:

```text
/case paste
/case paste --run
/case paste 2
```

```sh
run-cli case paste A.cpp
run-cli case paste A.cpp --run
run-cli case paste A.cpp 2
```

Capture interactive or piped stdin while running:

```sh
printf '2 3\n' | run-cli run A.cpp --save
```

Or create the files directly with any editor or script.

### Automatic judging

For input `A.in4`, Caseflow looks for `A.out4`.

- If `A.out4` exists, stdout is streamed normally and simultaneously captured
  for an exact byte-for-byte comparison.
- Matching output produces a PASS verdict.
- A mismatch, timeout, interruption, or runtime failure produces a FAIL verdict
  and a nonzero test status. The expected output is printed inside the same
  case block before its verdict and system-status footer.
- If `A.out4` does not exist, the case remains run-only and receives no
  correctness verdict.
- When multiple cases run and at least one is judged, Caseflow prints a final
  aggregate verdict followed by case badges. Passed IDs are green, failed IDs
  are red, and IDs without an expected output are dim grey:

```text
Test summary  ·  2/3 passed  ·  [1] [2] [3] [4]
```

Whitespace and final newlines matter. For example, `YES` and `YES ` are
different outputs, and a missing final newline also causes a mismatch.

Run all cases or select specific IDs:

```text
/test
/test all
/test 1 3 5
/test 1,3,5
/test last
```

Press Ctrl-C while `/test` is running to stop the current program and cancel
the rest of the batch. Caseflow reports how many cases were skipped and returns
to the session prompt; a second Ctrl-C is not required.

```sh
run-cli test A.cpp
run-cli test A.cpp 1 3 5
run-cli test A.cpp last
```

Use `compare` when the expected file does not follow the `.out<ID>` naming
convention:

```sh
run-cli compare A.cpp 1 official-answer.txt
```

### Inspecting and deleting cases

```sh
run-cli case list A.cpp
run-cli case show A.cpp 1
run-cli case copy A.cpp 1
```

Interactive deletion asks for confirmation:

```text
/case delete 1
/case clear
```

One-shot deletion requires `--force`:

```sh
run-cli case delete A.cpp 1 --force
run-cli case clear A.cpp --force
```

Deleting an input also deletes its paired `.out<ID>` file when present.

## One-shot commands

The general form is:

```text
run-cli [GLOBAL OPTIONS] COMMAND [COMMAND OPTIONS]
```

These global execution options are accepted before or after subcommands:

| Option                        | Meaning                                                     |
| ----------------------------- | ----------------------------------------------------------- |
| `--debug`                   | Use debug compiler/runtime settings.                        |
| `--mouse`                   | Explicitly enable mouse controls, overriding configuration. |
| `--no-mouse`                | Disable mouse controls when an interactive UI is opened.    |
| `--color auto\|always\|never` | Set color output policy.                                    |

`--no-source` is a top-level session option rather than a subcommand option:

```sh
run-cli --no-source
```

Use `run-cli --help` for top-level help, `run-cli COMMAND --help` for a
subcommand, and `run-cli --version` for the installed version.

### Run

```sh
# Interactive stdin
run-cli run A.cpp

# Piped stdin
printf '2 3\n' | run-cli run A.cpp

# Read a named file
run-cli run A.cpp --input sample.in

# Save stdout atomically
run-cli run A.cpp --input sample.in --output answer.out

# Run several input/output pairs with one build
run-cli run A.cpp \
  --input first.in  --output first.out \
  --input second.in --output second.out

# Apply a time limit
run-cli run A.cpp --timeout 2

# Save stdin as the next .in<ID> case
printf '2 3\n' | run-cli run A.cpp --save

# Append clipboard input and run the saved case
run-cli run A.cpp --clipboard
```

Repeat `--input` for batch execution. When `--output` is present in a batch,
provide exactly one output for each input. Existing output destinations are
replaced atomically only after successful execution.

Caseflow rejects output destinations that would overwrite a source, an input,
another selected output, or an internal build artifact.

### Build

```sh
run-cli build A.cpp
run-cli build A.cpp --debug
```

### Test and case management

```sh
run-cli test A.cpp
run-cli test A.cpp all
run-cli test A.cpp last
run-cli test A.cpp 1 3,5

run-cli case list A.cpp
run-cli case show A.cpp 1
run-cli case copy A.cpp 1
run-cli case paste A.cpp
run-cli case paste A.cpp 2
run-cli case paste A.cpp --run
run-cli case add A.cpp
run-cli case edit A.cpp 2
run-cli case delete A.cpp 2 --force
run-cli case clear A.cpp --force
```

### Compare, stress, and sample import

```sh
run-cli compare A.cpp 1 expected.out
run-cli stress A.cpp brute.cpp generator.py --runs 500 --timeout 2
run-cli companion A.cpp
run-cli companion A.cpp --port 10043 --wait 300
```

### Clean stdout in scripts

When stdout is redirected, Caseflow reserves it for program output. Build
messages, errors, resource measurements, and verdicts go to stderr:

```sh
run-cli run A.cpp --input A.in1 > actual.out
```

Use the command's exit status to decide whether the operation succeeded.

## Clipboard workflows

Clipboard access prefers Wayland tools and falls back to X11:

1. `wl-paste` / `wl-copy`
2. `xclip`

Append and run clipboard input:

```text
/run clipboard
```

This is equivalent to saving the clipboard as the next case and testing that
new case. It does not replace existing cases.

Append without running, or replace an explicit case:

```text
/case paste
/case paste 3
```

Copy an existing case:

```text
/case copy 3
```

Clipboard command failures do not create or replace case files.

## Competitive Companion

Caseflow supports the documented custom-tool protocol from
[Competitive Companion](https://github.com/jmerle/competitive-companion).

### Which command should I use?

| Situation                                   | Command                                              | Result                                                                                                 |
| ------------------------------------------- | ---------------------------------------------------- | ------------------------------------------------------------------------------------------------------ |
| Import one problem into the active source   | `/companion`                                       | Receives one payload and appends its samples to the active source stem.                                |
| Import a full contest from a normal session | `/contest`                                         | Uses the active source for the first problem and derives the remaining source names.                   |
| Equivalent explicit contest form            | `/companion contest`                               | Performs the same complete-batch import as`/contest`.                                                |
| Create a new contest workspace              | `run-cli --no-source`, then `/contest DIRECTORY` | Creates the directory, generates`A.cpp`, `B.cpp`, and so on, imports samples, and opens `A.cpp`. |
| Import from a script or normal shell        | `run-cli companion SOURCE [--contest]`             | Runs the receiver without opening the workspace.                                                       |

`/contest` is therefore the convenient full-contest workflow. `/companion` is
the general receiver and defaults to one problem unless `contest` is supplied.

### Import from the interactive workspace

Open the solution:

```sh
run-cli A.cpp
```

Start the receiver:

```text
/companion
```

Then open a single problem page in the browser and click Competitive
Companion's green `+` button.

### Import with a one-shot command

```sh
run-cli companion A.cpp
```

While the command is waiting, click the extension button.

To import an entire contest from an active-source session, open the contest
page, start contest mode, and click the extension button. Competitive
Companion then sends one request for each problem in the batch:

```text
/contest
```

`/companion contest` is an equivalent spelling.

To create a new workspace before any source exists:

```sh
run-cli --no-source
```

```text
/contest contests/spring-round
```

Caseflow creates the directory if necessary, generates editable source
placeholders, imports every sample pair, and opens the session on `A.cpp`.

Contest mode uses Competitive Companion's shared batch ID and problem count to
keep collecting requests. It does not write any files until every expected
problem has arrived; cancelling or timing out a partial batch therefore leaves
the workspace unchanged.

### Receiver behavior

- The default port is `10043`, one of Competitive Companion's built-in ports.
- The default wait timeout is 120 seconds.
- The listener binds only to IPv4/IPv6 loopback interfaces.
- Normal mode accepts one problem payload and then exits.
- Contest mode accepts every problem in one `batch` payload sequence and exits
  after the advertised problem count is received.
- Ctrl-C cancels the listener and returns status 130.
- Request headers are limited to 64 KiB and the JSON body to 8 MiB.
- Existing case IDs are never overwritten.
- Each sample is imported atomically as a paired `.in<ID>` and `.out<ID>`.
- Contest mode validates that every request has the same batch ID and size,
  rejects batches larger than 512 problems, and imports all problems
  transactionally. If one file cannot be written, already-imported files are
  removed.
- Contest sample text is capped at 64 MiB across the whole batch, in addition
  to the 8 MiB limit on each individual request body.
- Requests are assigned in arrival order. The first problem uses the source
  passed on the command line; later problems derive source stems as follows:
  `A.cpp` becomes `B.cpp`, `C.cpp`, and so on, while any other stem such as
  `main.cpp` becomes `main-2.cpp`, `main-3.cpp`, and so on. Case files are
  created for derived stems. Existing source files are never overwritten;
  missing contest stems are created as editable placeholders, including in
  source-less download mode.

Use a custom port or wait duration when needed:

```text
/companion --port 4244 --wait 300
```

```sh
run-cli companion A.cpp --port 4244 --wait 300
```

Collect a contest batch from a one-shot command:

```sh
run-cli companion A.cpp --contest --port 10043 --wait 300
```

No source file is required for a new contest directory. This uses `A.cpp` as
the first problem stem and creates the directory when needed:

```sh
run-cli companion contests/spring-round/A.cpp --contest --port 10043
```

The same behavior with the default current directory is available as:

```sh
run-cli companion --contest
```

Competitive Companion sends the contest metadata (`batch.id` and
`batch.size`) with each problem. If the extension is configured to a custom
port, pass the same port to Caseflow and the extension.

After import, run `/test` or click a numbered Test button. Imported expected
outputs are judged automatically.

## Stress testing

Stress testing compares the active solution with a trusted brute-force
solution over inputs produced by a generator:

```sh
run-cli stress solution.cpp brute.cpp generator.py --runs 500 --timeout 2
```

Inside the workspace:

```text
/stress brute.cpp generator.py --runs 500 --timeout 2
```

The generator receives the 1-based case number as its first command-line
argument and writes one input to stdout. For each iteration, Caseflow:

1. Runs the generator.
2. Sends the generated input to the candidate and brute-force programs.
3. Compares their stdout byte for byte.
4. Stops on the first mismatch or abnormal exit.
5. Saves the failing input as the candidate's next `.in<ID>` case.
6. Prints a unified diff and returns status 1.

Example Python generator:

```python
import random
import sys

case_number = int(sys.argv[1])
random.seed(case_number)
n = random.randint(1, 20)
values = [random.randint(-100, 100) for _ in range(n)]

print(n)
print(*values)
```

Defaults are 1,000 generated cases and two seconds for each generator,
candidate, and brute execution. Configure them globally, per project, through
environment variables, or with `--runs` and `--timeout`.

Press Ctrl-C at any stage to stop the entire stress session. Caseflow cancels
the active process, skips remaining cases, and does not save an interrupted
input as a mismatch.

## Configuration

### Precedence

Configuration is applied from lowest to highest precedence:

1. Built-in defaults
2. `$XDG_CONFIG_HOME/run-cli/config.toml`, or `~/.config/run-cli/config.toml`
3. The nearest `.run-cli.toml` found above the active source
4. Environment variables
5. Command-line flags

Copy [`.run-cli.toml.example`](.run-cli.toml.example) into a project as
`.run-cli.toml`, or use it as the basis for the user-level configuration.

### Example

```toml
[toolchains]
cxx = "g++"
cc = "gcc"
python = "python3"
javac = "javac"
java = "java"
rustc = "rustc"
go = "go"
kotlinc = "kotlinc"

[flags]
cpp = ["-std=c++20", "-Wall", "-Wextra", "-DLOCAL"]
c = ["-std=c17", "-Wall", "-Wextra"]
# cpp_debug and c_debug may also replace the complete built-in debug lists.

[defaults]
mode = "standard"       # "standard" or "debug"
timeout = 2.0           # omit for no normal run/test timeout
stress_limit = 1000
stress_timeout = 2.0

[ui]
color = "auto"          # "auto", "always", or "never"
mouse = true             # enabled by default; set false to opt out
```

Flag arrays replace the corresponding built-in list; they are not appended.

### Environment variables

| Variable           | Overrides                                                   |
| ------------------ | ----------------------------------------------------------- |
| `CXX`            | C++ compiler                                                |
| `CC`             | C compiler                                                  |
| `PYTHON`         | Python interpreter                                          |
| `JAVAC`          | Java compiler                                               |
| `JAVA`           | Java/Kotlin runtime                                         |
| `RUSTC`          | Rust compiler                                               |
| `GO`             | Go tool                                                     |
| `KOTLINC`        | Kotlin compiler                                             |
| `NO_COLOR`       | Disables color unless a later CLI color option overrides it |
| `STRESS_LIMIT`   | Default positive stress iteration count                     |
| `STRESS_TIMEOUT` | Default positive per-process stress timeout in seconds      |

### Cache and state directories

By default, Caseflow stores:

- Build artifacts, locks, editor staging files, and temporary outputs below
  `$XDG_CACHE_HOME/run-cli`, or `~/.cache/run-cli`.
- REPL history below `$XDG_STATE_HOME/run-cli`, or
  `~/.local/state/run-cli`.

Only source files and explicitly saved `.in<ID>`/`.out<ID>` cases remain in the
solution directory.

## Shell completion

Bash, Zsh, and Fish completion uses the same parser-aware filesystem filtering
and ranking engine as the interactive workspace.

### Bash

```sh
mkdir -p ~/.local/share/bash-completion/completions
run-cli completion bash \
  > ~/.local/share/bash-completion/completions/run-cli
```

Start a new shell or source the generated file.

### Zsh

```sh
mkdir -p ~/.zfunc
run-cli completion zsh > ~/.zfunc/_run-cli
```

Ensure the directory is in `fpath`, then initialize completion:

```zsh
fpath=(~/.zfunc $fpath)
autoload -Uz compinit
compinit
```

### Fish

```fish
mkdir -p ~/.config/fish/completions
run-cli completion fish > ~/.config/fish/completions/run-cli.fish
```

The completion subcommand also accepts the other shells supported by
`clap_complete`, including Elvish and PowerShell.

## Compatibility aliases

Canonical help stays concise, while older spellings remain accepted:

- `run-cli exec` → `run-cli run`
- `run-cli diff` → `run-cli compare`
- `run-cli completions` → `run-cli completion`
- `/source` → `/open`
- `/mode debug` / `/mode standard` → `/debug on` / `/debug off`
- `/diff` → `/compare`
- `/quit` → `/exit`

Legacy test flags `--all`, `--last`, and `--id`; clipboard targets `next`,
`--next`, and `--id`; and stress options `--brute`, `--generator`, and `--limit`
also remain parseable.

Caseflow reads the same `<stem>.in<ID>` files as [`run.sh`](run.sh), so existing
cases require no migration. The legacy script remains unchanged as a parity
reference.

## Safety and process behavior

- Child programs run in their own process groups.
- Ctrl-C targets the full child process group, then escalates to SIGTERM and
  SIGKILL if necessary.
- Timeouts stop descendants rather than only the immediate child.
- Explicit output files use same-directory temporary files and atomic rename.
- Failed programs do not replace an existing explicit output file.
- Concurrent case writers use file locking.
- External-editor changes are staged and committed only after success.
- Destructive interactive actions require confirmation; scripted deletion
  requires `--force`.
- Mouse tracking, raw mode, cursor visibility, scroll regions, and the alternate
  screen are restored on every handled exit path.
- Competitive Companion listens only on local loopback interfaces and only
  while an import command is active.

## Exit statuses

| Status  | Meaning                                                                          |
| ------- | -------------------------------------------------------------------------------- |
| `0`   | Successful operation or all judged cases passed.                                 |
| `1`   | Output mismatch, stress mismatch, missing doctor dependency, or general failure. |
| `2`   | Invalid command, option, selector, or configuration.                             |
| `124` | Execution or listener wait timed out.                                            |
| `130` | Interrupted with Ctrl-C.                                                         |
| Other   | Compiler or child-program exit status, normalized to the shell range.            |

For a mixed test selection, any judged mismatch or abnormal execution makes
the overall command fail even though later cases may still run.

## Troubleshooting

### A compiler or runtime is not found

Run:

```sh
run-cli doctor
```

Install the missing tool, set its environment variable, or configure its path
under `[toolchains]`.

### Clipboard commands fail

Install `wl-clipboard` on Wayland or `xclip` on X11. `run-cli doctor` shows which
backend Caseflow detects. A failed clipboard read never modifies saved cases.

### A correct-looking answer receives FAIL

Automatic judging is byte-for-byte. Check trailing spaces, blank lines, and the
final newline:

```sh
run-cli compare A.cpp 1 A.out1
```

The explicit compare command prints a unified diff when possible.

### Live output is suppressed

The interactive safety limiter detected excessive output. Press Ctrl-C if the
program is looping. To preserve the complete output of a finite program:

```sh
run-cli run A.cpp --input A.in1 --output full.out
```

### Competitive Companion times out

- Start `/companion` before clicking the extension button.
- Confirm the extension and Caseflow use the same port.
- Check whether another process already owns the port.
- Use `/companion contest` or `run-cli companion A.cpp --contest` for a
  whole-contest batch; normal mode intentionally accepts one problem only.
- Increase the wait with `--wait 300` if needed.

### Mouse selection captures clicks

Use the toolbar's **Select** action before dragging, or try Shift-drag if your
terminal supports the selection override. Press Esc to restore Caseflow mouse
controls.

### Kotlin fails in a sandboxed installation

Some Snap/AppArmor environments prevent `kotlinc` from starting correctly.
Use a normal Kotlin installation, configure its path with `KOTLINC`, and verify
it through `run-cli doctor`.

### The terminal was interrupted externally

Caseflow restores the terminal for its handled signals. If the process is
forcibly killed before cleanup can run, the standard terminal command can
restore a usable state:

```sh
reset
```

## Development

Run the complete local verification suite:

```sh
cargo fmt --all -- --check
cargo clippy --all-targets -- -D warnings
cargo test --all-targets --locked
cargo build --release --locked
```

Tests include unit coverage, CLI integration, `run.sh` parity, concurrency,
signals, clipboard failures, automatic verdicts, and PTY scenarios for the
full-screen workspace, mouse behavior, live output, resizing, Ctrl-C, and
terminal restoration.

Create a release archive for the current host:

```sh
scripts/package-release.sh
```

Or specify a configured Rust target:

```sh
scripts/package-release.sh x86_64-unknown-linux-musl
```

Artifacts and SHA-256 files are written below `dist/`. Tagged releases use the
GitHub Actions release workflow to package GNU and musl builds.

See [PLAN.md](PLAN.md) for the design and acceptance plan and
[CHANGELOG.md](CHANGELOG.md) for version history.

## License

Caseflow is available under the [MIT License](LICENSE).
