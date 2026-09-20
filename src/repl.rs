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
        mode: base_globals.mode.unwrap_or(config.default_mode),
        mouse: config.mouse,
        config,
        ui,
        base_globals,
    };

    let mut editor = LineEditor::new(
        session.config.state_dir.join("history.txt"),
        session.mouse,
        session.ui.color_enabled(),
    );
    if let Some(warning) = editor.take_warning() {
        editor.append_output(format!("! {warning}\n").as_bytes());
    }
    let mut last_repeatable: Option<String> = None;

    loop {
        editor.set_context(
            session.source.path.display().to_string(),
            session.source.language.to_string(),
            session.mode.to_string(),
            session.mouse,
        );
        editor.set_case_ids(
            cases::list(&session.source)?
                .into_iter()
                .map(|saved| saved.id)
                .collect(),
        );
        let prompt = format!(
            " run-cli:{} › ",
            session
                .source
                .path
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("source")
        );
        match editor.read_line(&prompt)? {
            EditorSignal::Success(line) => {
                let mut line = line.trim().to_string();
                if line.is_empty() {
                    continue;
                }
                if line == "/clear" {
                    editor.clear_output();
                    continue;
                }
                if line == "/again" {
                    let Some(previous) = last_repeatable.clone() else {
                        editor.begin_command("/again");
                        editor.append_output(
                            "✗ Error  nothing to repeat; run /run or /test first\n".as_bytes(),
                        );
                        continue;
                    };
                    line = previous;
                }
                if matches!(first_command(&line), "/open" | "/source") {
                    last_repeatable = None;
                }
                if submit(&mut session, &mut editor, &line)? == SessionAction::Exit {
                    return Ok(0);
                }
                if is_repeatable(&line) {
                    last_repeatable = Some(line);
                }
                editor.set_mouse(session.mouse);
                editor.set_color(session.ui.color_enabled());
                if let Some(warning) = editor.take_warning() {
                    editor.append_output(format!("! {warning}\n").as_bytes());
                }
            }
            EditorSignal::Action(action) => {
                let command = match action {
                    EditorAction::RunInteractive => "/run interactive".to_string(),
                    EditorAction::RunClipboard => "/run clipboard".to_string(),
                    EditorAction::ToggleDebug => "/debug toggle".to_string(),
                    EditorAction::SelectText => unreachable!("handled by the editor"),
                    EditorAction::TestCase(id) => format!("/test {id}"),
                    EditorAction::TestAll => "/test all".to_string(),
                };
                if submit(&mut session, &mut editor, &command)? == SessionAction::Exit {
                    return Ok(0);
                }
                if is_repeatable(&command) {
                    last_repeatable = Some(command);
                }
                editor.set_mouse(session.mouse);
                editor.set_color(session.ui.color_enabled());
            }
            EditorSignal::CtrlC => {
                editor.append_output(b"^C\n");
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

#[derive(Clone, Copy, PartialEq, Eq)]
enum SessionAction {
    Continue,
    Exit,
    EditCase(Option<u64>),
}

fn submit(
    session: &mut Session,
    editor: &mut LineEditor,
    command: &str,
) -> AppResult<SessionAction> {
    editor.begin_command(command);
    terminal::begin_workspace_output(session.mouse)?;
    let captured = terminal::capture_output(session.mouse, || match session.handle(command) {
        Ok(action) => action,
        Err(error) => {
            session.ui.error(error.message);
            SessionAction::Continue
        }
    });
    let restore = terminal::end_workspace_output();
    let (action, captured) = captured?;
    restore?;
    editor.append_captured_output(&captured.output, &captured.input);
    if let SessionAction::EditCase(id) = action {
        let result = cases::edit_with_external_editor(&session.source, id, &session.config);
        match result {
            Ok(cases::ExternalEditOutcome::Added(saved)) => editor.append_output(
                format!("✓ Added input #{}  {}\n", saved.id, saved.path.display()).as_bytes(),
            ),
            Ok(cases::ExternalEditOutcome::Updated(saved)) => editor.append_output(
                format!("✓ Updated input #{}  {}\n", saved.id, saved.path.display()).as_bytes(),
            ),
            Ok(cases::ExternalEditOutcome::Unchanged(saved)) => editor.append_output(
                format!("Input #{} unchanged  {}\n", saved.id, saved.path.display()).as_bytes(),
            ),
            Err(error) => editor.append_output(format!("✗ Error  {}\n", error.message).as_bytes()),
        }
        return Ok(SessionAction::Continue);
    }
    Ok(action)
}

fn first_command(line: &str) -> &str {
    line.split_whitespace().next().unwrap_or_default()
}

fn is_repeatable(line: &str) -> bool {
    matches!(first_command(line), "/run" | "/test")
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
            "/exit" | "/quit" => Ok(SessionAction::Exit),
            "/help" => {
                if tokens.len() > 2 {
                    return Err(AppError::usage("usage: /help [COMMAND]"));
                }
                print_help(tokens.get(1).map(String::as_str));
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
            "/debug" | "/mode" => {
                if tokens.len() > 2 {
                    return Err(AppError::usage("usage: /debug [on|off|toggle]"));
                }
                match tokens.get(1).map(String::as_str) {
                    None | Some("toggle") => {
                        self.mode = if self.mode == BuildMode::Debug {
                            BuildMode::Standard
                        } else {
                            BuildMode::Debug
                        }
                    }
                    Some("on" | "debug") => self.mode = BuildMode::Debug,
                    Some("off" | "standard") => self.mode = BuildMode::Standard,
                    _ => return Err(AppError::usage("usage: /debug [on|off|toggle]")),
                }
                self.ui.success(format!("Build mode: {}", self.mode));
                Ok(SessionAction::Continue)
            }
            "/mouse" => {
                if tokens.len() > 2 {
                    return Err(AppError::usage("usage: /mouse [on|off|toggle]"));
                }
                match tokens.get(1).map(String::as_str) {
                    None | Some("toggle") => self.mouse = !self.mouse,
                    Some("on") => self.mouse = true,
                    Some("off") => self.mouse = false,
                    _ => return Err(AppError::usage("usage: /mouse [on|off|toggle]")),
                }
                self.ui.success(format!(
                    "Mouse controls: {}",
                    if self.mouse { "on" } else { "off" }
                ));
                Ok(SessionAction::Continue)
            }
            "/open" | "/source" => {
                if tokens.len() > 2 {
                    return Err(AppError::usage("usage: /open [PATH]"));
                }
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
                    self.mode = self.base_globals.mode.unwrap_or(config.default_mode);
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
                        OsString::from("run"),
                        self.source.requested.as_os_str().to_owned(),
                        OsString::from("--clipboard"),
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
                    OsString::from("run"),
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
                let mut args = vec![
                    OsString::from("run-cli"),
                    OsString::from("test"),
                    self.source.requested.as_os_str().to_owned(),
                ];
                args.extend(tokens.iter().skip(1).map(OsString::from));
                self.execute(args)?;
                Ok(SessionAction::Continue)
            }
            "/case" => self.handle_case(&tokens),
            "/compare" | "/diff" => {
                if tokens.len() != 3 {
                    return Err(AppError::usage("usage: /compare ID EXPECTED"));
                }
                self.execute(vec![
                    OsString::from("run-cli"),
                    OsString::from("compare"),
                    self.source.requested.as_os_str().to_owned(),
                    OsString::from(&tokens[1]),
                    OsString::from(&tokens[2]),
                ])?;
                Ok(SessionAction::Continue)
            }
            "/stress" => {
                if tokens.len() < 3 {
                    return Err(AppError::usage(
                        "usage: /stress BRUTE GENERATOR [--runs N] [--timeout SEC]",
                    ));
                }
                let mut args = vec![
                    OsString::from("run-cli"),
                    OsString::from("stress"),
                    self.source.requested.as_os_str().to_owned(),
                    OsString::from(&tokens[1]),
                    OsString::from(&tokens[2]),
                ];
                args.extend(tokens.iter().skip(3).map(OsString::from));
                self.execute(args)?;
                Ok(SessionAction::Continue)
            }
            "/companion" => {
                let mut args = vec![
                    OsString::from("run-cli"),
                    OsString::from("companion"),
                    self.source.requested.as_os_str().to_owned(),
                ];
                args.extend(tokens.iter().skip(1).map(OsString::from));
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

    fn handle_case(&mut self, tokens: &[String]) -> AppResult<SessionAction> {
        let action = tokens.get(1).map(String::as_str).ok_or_else(|| {
            AppError::usage("usage: /case list|show|copy|paste|add|edit|delete|clear")
        })?;
        if action == "add" {
            if tokens.len() != 2 {
                return Err(AppError::usage("usage: /case add"));
            }
            return Ok(SessionAction::EditCase(None));
        }
        if action == "edit" {
            if tokens.len() != 3 {
                return Err(AppError::usage("usage: /case edit ID"));
            }
            let id = tokens[2]
                .parse::<u64>()
                .map_err(|_| AppError::usage("case IDs must be positive integers"))?;
            if id == 0 {
                return Err(AppError::usage("case IDs must be positive integers"));
            }
            return Ok(SessionAction::EditCase(Some(id)));
        }
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
                if tokens.len() != 3 {
                    return Err(AppError::usage(format!("usage: /case {action} ID")));
                }
                let id = tokens
                    .get(2)
                    .ok_or_else(|| AppError::usage(format!("usage: /case {action} ID")))?;
                args.push(OsString::from(id));
            }
            "paste" => {
                let mut position = 2;
                if let Some(target) = tokens.get(position) {
                    if target == "next" {
                        position += 1;
                    } else if !target.starts_with('-') {
                        args.push(OsString::from(target));
                        position += 1;
                    }
                }
                args.extend(tokens.iter().skip(position).map(OsString::from));
            }
            "delete" => {
                if tokens.len() != 3 {
                    return Err(AppError::usage("usage: /case delete ID"));
                }
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
                    return Ok(SessionAction::Continue);
                }
                args.push(OsString::from(id));
                args.push(OsString::from("--force"));
            }
            "clear" => {
                if tokens.len() != 2 {
                    return Err(AppError::usage("usage: /case clear"));
                }
                if !terminal::confirm(
                    "Delete all saved inputs?",
                    "Clear",
                    self.mouse,
                    self.ui.color_enabled(),
                )? {
                    self.ui.info("Cancelled");
                    return Ok(SessionAction::Continue);
                }
                args.push(OsString::from("--force"));
            }
            _ => return Err(AppError::usage(format!("unknown case action '{action}'"))),
        }
        self.execute(args)?;
        Ok(SessionAction::Continue)
    }

    fn execute(&self, args: Vec<OsString>) -> AppResult<()> {
        let cli = Cli::try_parse_from(args).map_err(|error| AppError::usage(error.to_string()))?;
        let command = cli
            .command
            .ok_or_else(|| AppError::usage("missing command"))?;
        let globals = GlobalOptions {
            color: self.base_globals.color,
            mouse: self.mouse,
            mode: Some(if cli.debug {
                BuildMode::Debug
            } else {
                self.mode
            }),
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

fn print_help(topic: Option<&str>) {
    if let Some(topic) = topic {
        let topic = topic.trim_start_matches('/');
        let help = match topic {
            "run" => "/run [interactive] [--timeout SEC] [--save] [-i FILE] [-o FILE]\n/run clipboard [--timeout SEC]",
            "build" => "/build\nCompile the active source without running it.",
            "test" => "/test [all|last|ID ...]\nNo selector runs every saved case.",
            "case" => "/case list|show ID|copy ID|paste [ID] [--run]|add|edit ID|delete ID|clear",
            "compare" | "diff" => "/compare ID EXPECTED",
            "stress" => "/stress BRUTE GENERATOR [--runs N] [--timeout SEC]",
            "companion" => "/companion [--port PORT] [--wait SEC]\nWait for one problem from the Competitive Companion extension and append its samples.",
            "open" | "source" => "/open [PATH]\nWith no path, open the fuzzy source picker.",
            "debug" | "mode" => "/debug [on|off|toggle]\nWith no value, toggle debug mode.",
            "mouse" => "/mouse [on|off|toggle]\nWith no value, toggle mouse controls.",
            "again" => "/again\nRepeat the most recent /run or /test command.",
            "clear" => "/clear\nClear retained output from the viewport.",
            "status" => "/status\nShow current session settings.",
            "doctor" => "/doctor\nInspect local toolchains and clipboard support.",
            "exit" | "quit" => "/exit\nLeave run-cli and restore the previous terminal.",
            _ => {
                println!("No help for '/{topic}'. Type /help for all commands.");
                return;
            }
        };
        println!("{help}");
        return;
    }

    println!(
        "\
RUN
  /run [interactive] [--timeout SEC] [--save] [-i FILE] [-o FILE]
  /run clipboard [--timeout SEC]   save clipboard as next case and run it
  /build
  /test [all|last|ID ...]          default: all

CASES
  /case list|show ID|copy ID|paste [ID] [--run]
  /case add|edit ID|delete ID|clear
  /compare ID EXPECTED
  /stress BRUTE GENERATOR [--runs N] [--timeout SEC]
  /companion [--port PORT] [--wait SEC]
                          import samples from Competitive Companion

SESSION
  /open [PATH]          switch the active source
  /debug [on|off|toggle]
  /mouse [on|off|toggle]
  /again                repeat the latest run or test
  /clear                clear retained output
  /status               show current settings
  /doctor               inspect local tools
  /help [COMMAND]       show help
  /exit                 leave run-cli

KEYS
  Tab suggestions · ↑↓ navigate · Enter select · Esc close · Ctrl-D exit
"
    );
}
