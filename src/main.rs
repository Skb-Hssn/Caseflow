mod build;
mod cases;
mod cli;
mod commands;
mod config;
mod editor;
mod error;
mod model;
mod process;
mod repl;
mod source;
mod suggest;
mod terminal;
mod theme;
mod ui;

use clap::Parser;
use cli::Cli;
use commands::GlobalOptions;
use error::{AppError, AppResult};
use std::io::IsTerminal;
use std::path::Path;

fn main() {
    install_panic_cleanup();
    install_signal_cleanup();
    let cli = Cli::parse();
    let globals = GlobalOptions {
        color: cli.color,
        mouse: cli.mouse,
        mode: None,
    };
    let result = match cli.command {
        Some(command) => commands::execute(command, globals),
        None => open_session(cli.source, globals),
    };
    match result {
        Ok(code) => std::process::exit(normalize_exit(code)),
        Err(error) => {
            let (_, ui) = commands::configured(None, globals).unwrap_or_else(|_| {
                let config = config::Config::default();
                (config.clone(), ui::Ui::new(config.color))
            });
            ui.error(&error.message);
            std::process::exit(normalize_exit(error.code));
        }
    }
}

fn open_session(source: Option<std::path::PathBuf>, globals: GlobalOptions) -> AppResult<i32> {
    if !std::io::stdin().is_terminal() || !std::io::stderr().is_terminal() {
        return Err(AppError::usage(
            "interactive mode requires a terminal; use 'run-cli exec SOURCE' for scripts",
        ));
    }
    let _screen = terminal::ScreenGuard::enter()?;
    let source = match source {
        Some(source) => source,
        None => {
            let (config, ui) = commands::configured(None, globals)?;
            let sources = source::scan_sources(Path::new(".")).map_err(AppError::new)?;
            if sources.is_empty() {
                return Err(AppError::new(
                    "no supported source files found in the current directory",
                ));
            }
            terminal::select_source(&sources, config.mouse, ui.color_enabled())?
                .ok_or_else(|| AppError::new("source selection cancelled"))?
        }
    };
    let source = source::resolve_source(source).map_err(AppError::usage)?;
    if !source.path.is_file() {
        return Err(AppError::new(format!(
            "source file '{}' does not exist",
            source.path.display()
        )));
    }
    repl::start(source, globals)
}

fn normalize_exit(code: i32) -> i32 {
    code.clamp(0, 255)
}

fn install_panic_cleanup() {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |information| {
        terminal::restore_terminal_and_screen();
        previous(information);
    }));
}

fn install_signal_cleanup() {
    use signal_hook::consts::signal::{SIGHUP, SIGQUIT, SIGTERM};
    use signal_hook::iterator::Signals;

    if let Ok(mut signals) = Signals::new([SIGHUP, SIGQUIT, SIGTERM]) {
        std::thread::spawn(move || {
            if let Some(signal) = signals.forever().next() {
                terminal::restore_terminal_and_screen();
                std::process::exit(128 + signal);
            }
        });
    }
}
