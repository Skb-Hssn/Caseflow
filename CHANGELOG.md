# Changelog

All notable changes to this project are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and releases use
[Semantic Versioning](https://semver.org/).

## [Unreleased]

## [0.15.1] - 2026-09-24

### Changed

- Increased the vertical spacing between saved-case reports and before the
  final test summary so adjacent cases remain visually distinct.
- Removed the CPU percentage from run and system-status summaries, retaining
  the exit status, wall-clock duration, and peak memory usage.
- Aligned interactive input transcripts at the left edge by removing the
  separate padding previously applied to their Input/Output labels and values.

### Fixed

- Restored compatibility with the Rust 1.98 Clippy warning set used by CI,
  including signal-handler casts, PTY reader control flow, and action-panel
  rendering without changing runtime behavior.
- Removed a port-reservation race between parallel Competitive Companion
  integration tests that could make a receiver exit before the test connected.

## [0.15.0] - 2026-09-23

### Added

- Added an Edit file entry to the More actions panel for opening the active
  source directly in the configured external editor.

### Changed

- Redesigned the interactive workspace around the Caseflow visual identity:
  the header now presents the version and source alongside right-aligned
  language, build-mode, and mouse status; the command prompt now uses the
  `caseflow:` brand.
- Reworked the fixed toolbar into unfilled, bracketed text controls with
  responsive group separators and wide-screen alignment for Run, Test, Debug,
  Select, and More.
- Replaced the full-width More list with a bordered, right-aligned action panel
  that uses full-row selection highlighting and an integrated keyboard hint.
- Refined terminal colors and weights for file headings, input/output labels,
  verdicts, system status details, test summaries, inactive controls, and
  menu chrome.
- Aligned saved-case file headings, input/output labels, separator rules,
  system status, and test summaries at the left edge without extra padding.

## [0.14.0] - 2026-09-23

### Added

- Added `/edit PATH` in normal and source-less sessions to open any file in
  the configured terminal editor and create the file when it is missing.
- Added all-file path completion for `/edit`, including quoted paths with
  spaces.

## [0.13.0] - 2026-09-23

### Added

- Ctrl-C now cancels the entire active saved-case batch, skips every remaining
  case, reports the skipped count, and returns directly to the session prompt.

## [0.12.0] - 2026-09-23

### Added

- Failed saved cases now display their expected output before the case verdict.
- Multi-case test runs now end with ordered, color-coded case badges: green for
  passed, red for failed, and dim grey for unjudged cases.

### Changed

- Moved each saved-case verdict ahead of its system-status footer so the
  verdict remains visually grouped with the corresponding input and output.

## [0.11.0] - 2026-09-22

### Added

- Added `--no-mouse` as an explicit override for terminals or workflows where
  mouse reporting is not wanted.
- Added Session status and Check tools to the More menu.

### Changed

- Enabled mouse controls by default for interactive sessions while retaining
  `--mouse` as an explicit compatibility override.
- Reordered the More menu around common, safe actions and moved destructive
  and exit actions toward the bottom.

## [0.10.0] - 2026-09-22

### Added

- Added persistent toolbar actions for repeating the latest run or test and
  creating a saved case in the configured external editor.
- Added a responsive More menu with source switching, case editing and
  deletion, Competitive Companion problem and contest import, output clearing,
  help, and exit actions.
- Added mouse and keyboard navigation for toolbar overflow menus, including a
  saved-case picker for edit and delete actions.

### Changed

- Improved toolbar responsiveness by reserving room for utility controls,
  showing an omission marker when numbered tests do not fit, and using compact
  `R`, `+`, and More labels on narrow terminals.

## [0.9.2] - 2026-09-21

### Changed

- Restored the compact toolbar layout so the command prompt sits directly
  below the controls.
- Refined toolbar colors to distinguish run, test, debug, and selection
  actions while preserving the existing mouse targets.

## [0.9.1] - 2026-09-21

### Changed

- Refined the mouse toolbar layout with a dedicated spacing row before the
  command prompt, keeping controls and input visually separate while output
  remains scrollable.
- Adjusted the fixed footer and live-output viewport for the new layout and
  kept toolbar hit targets aligned after resizing.

## [0.9.0] - 2026-09-21

### Added

- Added `--no-source` for starting an interactive workspace before any source
  exists.
- Added `/contest DIRECTORY` to receive a complete Competitive Companion
  contest, create editable source placeholders, and open the generated
  workspace automatically.
- `run-cli companion --contest` now permits an omitted source and defaults to
  `A.cpp`; missing contest source paths are created as placeholders.

## [0.8.0] - 2026-09-21

### Added

- Added complete Competitive Companion contest parsing with `--contest` and
  `/companion contest`.
- Contest batches validate their shared ID and problem count, queue all
  requests before writing, import samples into derived problem stems, and
  roll back the entire batch if an import fails.
- Added integration coverage for two-problem contest batches.

## [0.7.0] - 2026-09-20

### Added

- Automatically judge `/test` and `run-cli test` cases when a matching
  `<stem>.out<ID>` file exists, while continuing to stream the generated
  output and leaving cases without expected output in run-only mode.
- Added per-case PASS/FAIL results and a final aggregate verdict; any judged
  mismatch, timeout, interruption, or runtime failure now makes testing fail.

## [0.6.1] - 2026-09-20

### Fixed

- Stopped treating the visible viewport height as the excessive-output limit,
  so build context and echoed input no longer cause a small program result to
  display the live-output suppression warning.

## [0.6.0] - 2026-09-20

### Added

- Added a loopback-only Competitive Companion receiver through `/companion`
  and `run-cli companion SOURCE`, with configurable port and wait timeout.
- Imported each browser sample as an append-only `.in<ID>`/`.out<ID>` pair,
  preserving the extension's expected output for later comparison.

### Changed

- Deleting or clearing an imported case now also removes its paired expected
  output file.

## [0.5.9] - 2026-09-20

### Added

- Made Ctrl-C stop the complete stress-testing session during the generator,
  candidate, brute, or between cases, returning status 130 without running the
  remaining stages or saving an interrupted input as a mismatch.

## [0.5.8] - 2026-09-20

### Fixed

- Streamed saved-case and numbered Test-button output directly beneath the
  `Output` label while the program is running instead of hiding it in a
  temporary file until execution finished.

## [0.5.7] - 2026-09-20

### Fixed

- Limited each live-output burst to the visible workspace height, allowing
  terminals to paint tight print loops immediately in mouse mode instead of
  appearing blank while processing thousands of queued scroll operations.

## [0.5.6] - 2026-09-20

### Fixed

- Routed inherited REPL output through a pseudo-terminal so buffered C, C++,
  Python, and similar programs display each completed line while still
  running, without giving up retained output or runaway-output protection.

## [0.5.5] - 2026-09-20

### Fixed

- Bounded live REPL output from runaway print loops so the terminal remains
  responsive, the first Ctrl-C returns to the prompt, and run-cli retains the
  final execution status instead of flushing an unbounded output backlog.

## [0.5.4] - 2026-09-20

### Fixed

- Made Ctrl-C reliably stop non-terminating executions by escalating from
  SIGINT to SIGTERM and finally SIGKILL for the complete child process group,
  while reporting the operation as interrupted with exit status 130.

## [0.5.3] - 2026-09-20

### Fixed

- Prevented confirmation dialogs from redrawing for ignored or queued key
  events, eliminating repeated destructive-confirmation messages.

## [0.5.2] - 2026-09-20

### Fixed

- Fixed REPL confirmations and source selection incorrectly treating captured
  output as a non-terminal and cancelling immediately.
- Focused the requested destructive action in confirmation dialogs so Enter
  confirms `/case delete` and `/case clear`; Esc and `n` still cancel safely.

## [0.5.1] - 2026-09-20

### Changed

- Restyled saved-case output with a grey file heading, orange Input and Output
  labels, a dim grey divider, and a compact one-line dim status summary.

## [0.5.0] - 2026-09-20

### Added

- Added `/case add` and `/case edit` workflows for creating and updating saved
  inputs in an external editor without losing run-cli's terminal state.
- Added `/again` for repeating the latest run or test and `/clear` for clearing
  the retained output viewport.

### Changed

- Simplified the public vocabulary around `run`, `compare`, `/open`, and
  `/debug`, while retaining the former command spellings as compatibility
  aliases.
- Made bare `test` run every saved case and accepted positional case IDs,
  simplified clipboard case targets, and changed stress testing to positional
  helper sources with `--runs`.

## [0.4.7] - 2026-09-20

### Changed

- Reordered the action bar as Run, Test, Debug, and Select, with tighter button
  styling so more numbered cases fit at common terminal widths.

### Fixed

- Kept output scrolling available in native selection mode through wheel
  reports, arrow keys, Page Up/Down, Home, and End.

## [0.4.6] - 2026-09-20

### Changed

- Unified clipboard and saved-case presentation into a single `File` block
  with `Input`, `Output`, a divider, `System status`, and trailing spacing.

## [0.4.5] - 2026-09-20

### Changed

- Grouped `Interactive` and `Clipboard` beneath `Run`, matching the numbered
  `Test` actions.
- Replaced the toolbar's build action with an on/off debug-mode toggle; `/build`
  remains available from the command line.

## [0.4.4] - 2026-09-20

### Changed

- Removed the vertical gutter from retained interactive input.
- Added a native terminal-selection mode that releases application mouse
  tracking for drag selection and clipboard copying, then restores it on Esc.

## [0.4.3] - 2026-09-20

### Fixed

- Retained terminal input from interactive runs in a labeled input block after
  submission, instead of losing the terminal echo during the next redraw.

## [0.4.2] - 2026-09-20

### Fixed

- Preserved terminal interactivity detection while capturing retained session
  output, restoring the input and output sections for saved-case runs.

## [0.4.1] - 2026-09-20

### Changed

- Simplified the full-screen layout with a compact header, shorter action bar,
  one-line run summaries, and visible command boundaries in retained output.
- Added semantic highlighting to commands, results, warnings, and build sections
  so successive runs remain easy to scan.

## [0.4.0] - 2026-09-20

### Changed

- Anchored the session header, mouse actions, and command line while moving
  program output and diagnostics into a mouse-scrollable middle viewport.

## [0.3.0] - 2026-09-20

### Changed

- Simplified clipboard execution to `/run clipboard` and added the explicit
  `/run interactive` mode while retaining bare `/run` as the default.
- Made command options and file candidates appear passively when they become
  relevant, including the new choices immediately after `/run`.
- Expanded the fixed mouse bar with separate interactive/clipboard run actions
  and clickable saved-case numbers plus an all-cases action.
- Moved interactive sessions into a Vim-style alternate screen that restores
  the previous terminal contents on every exit path.
- Replaced the multicolor interface with a restrained slate-and-blue palette;
  green, amber, and red are now reserved for semantic status messages.
- Refined the welcome panel, prompt, grouped help, session status, result
  summaries, source picker, confirmation dialogs, and completion menus into a
  consistent responsive terminal UI.
- Added clearer selection markers, full-row highlighting, result counts, and
  contextual keyboard hints for interactive menus.
- Added a colored REPL prompt and a persistent `Run`/`Build`/`Test` mouse action
  bar, fixed to the bottom terminal row while the editor is active.
- Typing `/` now opens a passive dimmed command menu, and Tab accepts the
  highlighted complete command.
- Kept the mouse action bar outside the prompt redraw region so typing no
  longer causes the fixed buttons to flicker; it now repaints only when the
  terminal is resized or scrolled.

### Fixed

- Fixed saved-case discovery for relative sources in the current directory
  (`run-cli a.cpp` now correctly finds `a.in1`, `a.in2`, and so on).
- Removed mouse-mode flicker by requesting click/scroll events instead of
  pointer-motion events and skipping redraws for events that do not change UI.

## [0.2.0] - 2026-09-19

### Added

- Fuzzy, resize-aware source selection with bounded rendering for large lists.
- Mouse selection for REPL file-completion menus.
- Dynamic Bash, Zsh, and Fish completion backed by the REPL ranking engine.
- PTY coverage for REPL input, child interruption, resize handling, mouse
  clicks, and terminal restoration.
- Differential parity tests for concurrency, signals, configuration precedence,
  clipboard failures, and unusual filenames.
- CI, release archives, checksums, changelog, and MIT license.

### Changed

- Replaced the line editor with a small terminal-native editor so completion
  mouse events and terminal cleanup are fully controlled by `run-cli`.
- Completion and source-list rendering now adapt to narrow terminals.

## [0.1.0] - 2026-09-18

### Added

- Initial interactive and one-shot competitive-programming runner.
- C++, C, Python, Java, Rust, Go, and Kotlin toolchain support.
- Saved cases, batch I/O, diff, stress testing, resource reports, configuration,
  clipboard integration, and optional mouse controls.
