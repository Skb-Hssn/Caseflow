# Changelog

All notable changes to this project are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and releases use
[Semantic Versioning](https://semver.org/).

## [Unreleased]

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
