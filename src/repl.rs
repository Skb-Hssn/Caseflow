use crate::cases;
use crate::cli::Cli;
use crate::commands::{self, GlobalOptions};
use crate::config::Config;
use crate::editor::{EditorAction, EditorSignal, LineEditor};
use crate::error::{AppError, AppResult};
use crate::model::{BuildMode, SourceSpec};
use crate::source::{resolve_source, scan_sources};
use crate::terminal;
use crate::ui::Ui;
use clap::Parser;
use std::ffi::OsString;
use std::path::{Path, PathBuf};

pub fn start(source: SourceSpec, base_globals: GlobalOptions) -> AppResult<i32> {
    let (config, ui) = commands::configured(Some(&source.path), base_globals)?;
    let mut session = Session {
        source,
        mode: config.default_mode,
        mouse: config.mouse,
        config,
        ui,
        base_globals,
    };
    session
        .ui
        .welcome(&session.source, session.mode, session.mouse);

    let mut editor = LineEditor::new(
        session.config.state_dir.join("history.txt"),
        session.mouse,
        session.ui.color_enabled(),
    );
    if let Some(warning) = editor.take_warning() {
        session.ui.warning(warning);
    }

    let mut first_prompt = true;
    loop {
        editor.set_case_ids(
            cases::list(&session.source)?
                .into_iter()
                .map(|saved| saved.id)
                .collect(),
        );
        if !first_prompt {
            eprintln!();
        }
        first_prompt = false;
        let prompt = format!(
            "run-cli:{} · {} › ",
            session
                .source
                .path
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("source"),
            session.mode
        );
        match editor.read_line(&prompt)? {
            EditorSignal::Success(line) => {
                let line = line.trim();
                if line.is_empty() {
                    continue;
                }
                match session.handle(line) {
                    Ok(SessionAction::Continue) => {}
                    Ok(SessionAction::Exit) => return Ok(0),
                    Err(error) => session.ui.error(error.message),
                }
                editor.set_mouse(session.mouse);
                editor.set_color(session.ui.color_enabled());
                if let Some(warning) = editor.take_warning() {
                    session.ui.warning(warning);
                }
            }
            EditorSignal::Action(action) => {
                let command = match action {
                    EditorAction::RunInteractive => "/run interactive".to_string(),
                    EditorAction::RunClipboard => "/run clipboard".to_string(),
                    EditorAction::Build => "/build".to_string(),
                    EditorAction::TestCase(id) => format!("/test {id}"),
                    EditorAction::TestAll => "/test all".to_string(),
                };
                match session.handle(&command) {
                    Ok(SessionAction::Continue) => {}
                    Ok(SessionAction::Exit) => return Ok(0),
                    Err(error) => session.ui.error(error.message),
                }
                editor.set_mouse(session.mouse);
                editor.set_color(session.ui.color_enabled());
            }
            EditorSignal::CtrlC => {
                eprintln!("^C");
            }
            EditorSignal::CtrlD => return Ok(0),
        }
    }
}

struct Session {
    source: SourceSpec,
    mode: BuildMode,
    mouse: bool,
    config: Config,
    ui: Ui,
    base_globals: GlobalOptions,
}

enum SessionAction {
    Continue,
    Exit,
}

impl Session {
    fn handle(&mut self, line: &str) -> AppResult<SessionAction> {
        if !line.starts_with('/') {
            return Err(AppError::usage(
                "commands start with '/'; type /help to see available commands",
            ));
        }
        let tokens = shell_words::split(line)
            .map_err(|error| AppError::usage(format!("cannot parse command: {error}")))?;
        let command = tokens.first().map(String::as_str).unwrap_or_default();
        match command {
            "/quit" | "/exit" => Ok(SessionAction::Exit),
            "/help" => {
                print_help();
                Ok(SessionAction::Continue)
            }
            "/status" => {
                let case_count = cases::list(&self.source)?.len();
                self.ui.header("SESSION", "");
                self.ui
                    .field("Source", self.source.path.display().to_string());
                self.ui.field("Language", self.source.language.to_string());
                self.ui.field("Mode", self.mode.to_string());
                self.ui.field("Cases", case_count.to_string());
                self.ui
                    .field("Mouse", if self.mouse { "on" } else { "off" });
                self.ui
                    .field("Cache", self.config.cache_dir.display().to_string());
                Ok(SessionAction::Continue)
            }
            "/mode" => {
                match tokens.get(1).map(String::as_str) {
                    Some("standard") => self.mode = BuildMode::Standard,
                    Some("debug") => self.mode = BuildMode::Debug,
                    _ => return Err(AppError::usage("usage: /mode standard|debug")),
                }
                self.ui.success(format!("Build mode: {}", self.mode));
                Ok(SessionAction::Continue)
            }
            "/mouse" => {
                match tokens.get(1).map(String::as_str) {
                    Some("on") => self.mouse = true,
                    Some("off") => self.mouse = false,
                    _ => return Err(AppError::usage("usage: /mouse on|off")),
                }
                self.ui.success(format!(
                    "Mouse controls: {}",
                    if self.mouse { "on" } else { "off" }
                ));
                Ok(SessionAction::Continue)
            }
            "/source" => {
                let path = if let Some(path) = tokens.get(1) {
                    Some(PathBuf::from(path))
                } else {
                    terminal::select_source(
                        &scan_sources(Path::new(".")).map_err(AppError::new)?,
                        self.mouse,
                        self.ui.color_enabled(),
                    )?
                };
                if let Some(path) = path {
                    let source = resolve_source(path).map_err(AppError::usage)?;
                    if !source.path.is_file() {
                        return Err(AppError::new(format!(
                            "source file '{}' does not exist",
                            source.path.display()
                        )));
                    }
                    let (config, ui) = commands::configured(Some(&source.path), self.base_globals)?;
                    self.source = source;
                    self.mode = config.default_mode;
                    self.mouse = self.mouse || config.mouse;
                    self.config = config;
                    self.ui = ui;
                    self.ui
                        .success(format!("Active source: {}", self.source.path.display()));
                }
                Ok(SessionAction::Continue)
            }
            "/run" => {
                let mode = tokens.get(1).map(String::as_str);
                if mode == Some("clipboard") {
                    let mut args = vec![
                        OsString::from("run-cli"),
                        OsString::from("case"),
                        OsString::from("paste"),
                        self.source.requested.as_os_str().to_owned(),
                        OsString::from("--next"),
                        OsString::from("--run"),
                    ];
                    args.extend(tokens.iter().skip(2).map(OsString::from));
                    self.execute(args)?;
                    return Ok(SessionAction::Continue);
                }
                let option_start = if mode == Some("interactive") { 2 } else { 1 };
                if mode.is_some_and(|value| !value.starts_with('-') && value != "interactive") {
                    return Err(AppError::usage(
                        "usage: /run [interactive|clipboard] [OPTIONS]",
                    ));
                }
                let mut args = vec![
                    OsString::from("run-cli"),
                    OsString::from("exec"),
                    self.source.requested.as_os_str().to_owned(),
                ];
                args.extend(tokens.iter().skip(option_start).map(OsString::from));
                self.execute(args)?;
                Ok(SessionAction::Continue)
            }
            "/build" => {
                let mut args = vec![OsString::from("run-cli"), OsString::from("build")];
                args.push(self.source.requested.as_os_str().to_owned());
                args.extend(tokens.iter().skip(1).map(OsString::from));
                self.execute(args)?;
                Ok(SessionAction::Continue)
            }
            "/test" => {
                let selector = tokens
                    .get(1)
                    .ok_or_else(|| AppError::usage("usage: /test all|last|ID[,ID...]"))?;
                let mut args = vec![
                    OsString::from("run-cli"),
                    OsString::from("test"),
                    self.source.requested.as_os_str().to_owned(),
                ];
                match selector.as_str() {
                    "all" => args.push(OsString::from("--all")),
                    "last" => args.push(OsString::from("--last")),
                    ids => {
                        args.push(OsString::from("--id"));
                        args.push(OsString::from(ids));
                    }
                }
                self.execute(args)?;
                Ok(SessionAction::Continue)
            }
            "/case" => {
                self.handle_case(&tokens)?;
                Ok(SessionAction::Continue)
            }
            "/diff" => {
                if tokens.len() != 3 {
                    return Err(AppError::usage("usage: /diff ID EXPECTED"));
                }
                self.execute(vec![
                    OsString::from("run-cli"),
                    OsString::from("diff"),
                    self.source.requested.as_os_str().to_owned(),
                    OsString::from(&tokens[1]),
                    OsString::from(&tokens[2]),
                ])?;
                Ok(SessionAction::Continue)
            }
            "/stress" => {
                if tokens.len() < 3 {
                    return Err(AppError::usage(
                        "usage: /stress BRUTE GENERATOR [--limit N] [--timeout SEC]",
                    ));
                }
                let mut args = vec![
                    OsString::from("run-cli"),
                    OsString::from("stress"),
                    self.source.requested.as_os_str().to_owned(),
                    OsString::from("--brute"),
                    OsString::from(&tokens[1]),
                    OsString::from("--generator"),
                    OsString::from(&tokens[2]),
                ];
                args.extend(tokens.iter().skip(3).map(OsString::from));
                self.execute(args)?;
                Ok(SessionAction::Continue)
            }
            "/doctor" => {
                self.execute(vec![OsString::from("run-cli"), OsString::from("doctor")])?;
                Ok(SessionAction::Continue)
            }
            _ => Err(AppError::usage(format!(
                "unknown command '{command}'; type /help"
            ))),
        }
    }

    fn handle_case(&mut self, tokens: &[String]) -> AppResult<()> {
        let action = tokens
            .get(1)
            .map(String::as_str)
            .ok_or_else(|| AppError::usage("usage: /case list|show|copy|paste|delete|clear"))?;
        let mut args = vec![
            OsString::from("run-cli"),
            OsString::from("case"),
            OsString::from(action),
            self.source.requested.as_os_str().to_owned(),
        ];
        match action {
            "list" => {
                if tokens.len() != 2 {
                    return Err(AppError::usage("usage: /case list"));
                }
            }
            "show" | "copy" => {
                let id = tokens
                    .get(2)
                    .ok_or_else(|| AppError::usage(format!("usage: /case {action} ID")))?;
                args.push(OsString::from(id));
            }
            "paste" => {
                let target = tokens.get(2).map(String::as_str).unwrap_or("next");
                if target == "next" {
                    args.push(OsString::from("--next"));
                } else {
                    args.push(OsString::from("--id"));
                    args.push(OsString::from(target));
                }
                if tokens.iter().any(|token| token == "--run") {
                    args.push(OsString::from("--run"));
                }
            }
            "delete" => {
                let id = tokens
                    .get(2)
                    .ok_or_else(|| AppError::usage("usage: /case delete ID"))?;
                if !terminal::confirm(
                    &format!("Delete saved input #{id}?"),
                    "Delete",
                    self.mouse,
                    self.ui.color_enabled(),
                )? {
                    self.ui.info("Cancelled");
                    return Ok(());
                }
                args.push(OsString::from(id));
                args.push(OsString::from("--force"));
            }
            "clear" => {
                if !terminal::confirm(
                    "Delete all saved inputs?",
                    "Clear",
                    self.mouse,
                    self.ui.color_enabled(),
                )? {
                    self.ui.info("Cancelled");
                    return Ok(());
                }
                args.push(OsString::from("--force"));
            }
            _ => return Err(AppError::usage(format!("unknown case action '{action}'"))),
        }
        self.execute(args)
    }

    fn execute(&self, args: Vec<OsString>) -> AppResult<()> {
        let cli = Cli::try_parse_from(args).map_err(|error| AppError::usage(error.to_string()))?;
        let command = cli
            .command
            .ok_or_else(|| AppError::usage("missing command"))?;
        let globals = GlobalOptions {
            color: self.base_globals.color,
            mouse: self.mouse,
            mode: Some(self.mode),
        };
        match commands::execute(command, globals) {
            Ok(0) => Ok(()),
            Ok(code) => {
                self.ui
                    .warning(format!("Command finished with exit status {code}"));
                Ok(())
            }
            Err(error) => Err(error),
        }
    }
}

fn print_help() {
    println!(
        "\
RUN
  /run [interactive] [--timeout SEC] [--save-input] [-i FILE] [-o FILE]
  /run clipboard [--debug]   save clipboard as next case and run it
  /build [--debug]
  /test all|last|ID[,ID...]

CASES
  /case list|show ID|copy ID|paste [ID|next] [--run]
  /case delete ID|clear
  /diff ID EXPECTED
  /stress BRUTE GENERATOR [--limit N] [--timeout SEC]

SESSION
  /source [PATH]       switch the active source
  /mode standard|debug
  /mouse on|off
  /status              show current settings
  /doctor              inspect local tools
  /help                show this help
  /quit                leave run-cli

KEYS
  Tab suggestions · ↑↓ navigate · Enter select · Esc close · Ctrl-D exit
"
    );
}
