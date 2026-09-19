# Changelog

All notable changes to this project are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and releases use
[Semantic Versioning](https://semver.org/).

## [Unreleased]

### Changed

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
