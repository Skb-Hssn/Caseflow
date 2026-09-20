use crate::build;
use crate::cases;
use crate::cli::{
    BuildArgs, CaseArgs, CaseCommand, ColorArg, Command as CliCommand, CompareArgs, CompleteArgs,
    RunArgs, StressArgs, TestArgs,
};
use crate::config::{parse_duration, ColorPolicy, Config};
use crate::error::{AppError, AppResult};
use crate::model::{BuildMode, CaseSelector, OutputTarget, RunRequest, StressRequest};
use crate::process;
use crate::source::{is_supported_source, resolve_source};
use crate::ui::Ui;
use chrono::{DateTime, Local};
use clap::CommandFactory;
use std::collections::HashSet;
use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::path::{Component, Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};

static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Clone, Copy, Default)]
pub struct GlobalOptions {
    pub color: Option<ColorArg>,
    pub mouse: bool,
    pub mode: Option<BuildMode>,
}

pub fn configured(source: Option<&Path>, globals: GlobalOptions) -> AppResult<(Config, Ui)> {
    let mut config = Config::load(source).map_err(AppError::new)?;
    if let Some(color) = globals.color {
        config.color = match color {
            ColorArg::Auto => ColorPolicy::Auto,
            ColorArg::Always => ColorPolicy::Always,
            ColorArg::Never => ColorPolicy::Never,
        };
    }
    if globals.mouse {
        config.mouse = true;
    }
    let ui = Ui::new(config.color);
    Ok((config, ui))
}

pub fn execute(command: CliCommand, globals: GlobalOptions) -> AppResult<i32> {
    match command {
        CliCommand::Run(args) => execute_run(args, globals),
        CliCommand::Build(args) => execute_build(args, globals),
        CliCommand::Test(args) => execute_test(args, globals),
        CliCommand::Case(args) => execute_case(args, globals),
        CliCommand::Compare(args) => execute_compare(args, globals),
        CliCommand::Stress(args) => execute_stress(args, globals),
        CliCommand::Doctor => execute_doctor(globals),
        CliCommand::Completion(args) => {
            generate_completions(args.shell);
            Ok(0)
        }
        CliCommand::Complete(args) => execute_complete(args),
    }
}

fn execute_complete(args: CompleteArgs) -> AppResult<i32> {
    let cursor = args.cursor.unwrap_or(args.line.len());
    for candidate in crate::suggest::complete_cli(&args.line, cursor) {
        println!("{}\t{}", candidate.value, candidate.description);
    }
    Ok(0)
}

fn generate_completions(shell: clap_complete::Shell) {
    match shell {
        clap_complete::Shell::Bash => print!(
            r#"_run_cli_complete() {{
    local entry
    COMPREPLY=()
    while IFS= read -r entry; do
        COMPREPLY+=("${{entry%%$'\t'*}}")
    done < <(run-cli __complete --line "$COMP_LINE" --cursor "$COMP_POINT")
}}
complete -F _run_cli_complete run-cli
"#
        ),
        clap_complete::Shell::Zsh => print!(
            r#"#compdef run-cli
_run_cli_complete() {{
    local -a entries values
    local prefix="${{BUFFER[1,$CURSOR]}}"
    entries=("${{(@f)$(run-cli __complete --line "$prefix")}}")
    local entry
    for entry in $entries; do
        values+=("${{entry%%$'\t'*}}")
    done
    compadd -Q -- $values
}}
compdef _run_cli_complete run-cli
"#
        ),
        clap_complete::Shell::Fish => print!(
            r#"function __run_cli_complete
    run-cli __complete --line (commandline -cp)
end
complete -c run-cli -f -a '(__run_cli_complete)'
"#
        ),
        _ => {
            let mut command = crate::cli::Cli::command();
            clap_complete::generate(shell, &mut command, "run-cli", &mut io::stdout());
        }
    }
}

fn execute_build(args: BuildArgs, globals: GlobalOptions) -> AppResult<i32> {
    let source = resolve_source(&args.source).map_err(AppError::new)?;
    let (config, ui) = configured(Some(&source.path), globals)?;
    if !source.language.is_compiled() {
        return Err(AppError::usage(format!(
            "build is unsupported for Python; run '{}' directly",
            source.path.display()
        )));
    }
    build::build(&source, mode(&config, globals), &config, &ui)?;
    Ok(0)
}

fn execute_run(args: RunArgs, globals: GlobalOptions) -> AppResult<i32> {
    let source = resolve_source(&args.source).map_err(AppError::new)?;
    let (config, ui) = configured(Some(&source.path), globals)?;
    validate_exec_files(&source.path, &args.inputs, &args.outputs, &config)?;
    let timeout = args
        .timeout
        .map(|value| parse_duration(value, "--timeout"))
        .transpose()
        .map_err(AppError::usage)?
        .or(config.timeout);
    if args.clipboard {
        if args.save_input || !args.inputs.is_empty() || !args.outputs.is_empty() {
            return Err(AppError::usage(
                "--clipboard cannot be combined with --save, --input, or --output",
            ));
        }
        let saved = cases::paste(&source, None, &config)?;
        ui.success(format!(
            "Saved input #{}  {}",
            saved.id,
            saved.path.display()
        ));
        let mut run_config = config.clone();
        run_config.timeout = timeout;
        return run_saved_cases(
            &source,
            CaseSelector::Ids(vec![saved.id]),
            mode(&config, globals),
            &run_config,
            &ui,
        );
    }
    if args.save_input && !args.inputs.is_empty() {
        return Err(AppError::usage(
            "--save is only valid for interactive or piped stdin",
        ));
    }
    let product = build::build(&source, mode(&config, globals), &config, &ui)?;

    if args.inputs.is_empty() {
        let capture = if args.save_input {
            Some(temporary_path(&config, "captured-input")?)
        } else {
            None
        };
        if ui.interactive() {
            ui.header("RUN", "interactive input");
        }
        let output = args
            .outputs
            .first()
            .cloned()
            .map(OutputTarget::AtomicFile)
            .unwrap_or(OutputTarget::Inherit);
        let report = process::run(
            &RunRequest {
                command: product.command,
                input: None,
                output,
                timeout,
                capture_input: capture.clone(),
                show_report: true,
            },
            &ui,
        )?;
        if let Some(capture) = capture {
            let saved = cases::save_file_as_next(&source, &capture, &config)?;
            let _ = fs::remove_file(capture);
            ui.success(format!(
                "Saved input #{}  {}",
                saved.id,
                saved.path.display()
            ));
        }
        return Ok(report.exit_code);
    }

    let mut status = 0;
    for (index, input) in args.inputs.iter().enumerate() {
        if !input.is_file() {
            ui.error(format!(
                "input file '{}' does not exist; skipping it",
                input.display()
            ));
            status = 1;
            continue;
        }
        show_input(input, &ui)?;
        let output = args
            .outputs
            .get(index)
            .cloned()
            .map(OutputTarget::AtomicFile)
            .unwrap_or(OutputTarget::Inherit);
        ui.header(
            "OUTPUT",
            args.outputs
                .get(index)
                .map(|path| format!("saving to {}", path.display()))
                .unwrap_or_default(),
        );
        let report = process::run(
            &RunRequest {
                command: product.command.clone(),
                input: Some(input.clone()),
                output,
                timeout,
                capture_input: None,
                show_report: true,
            },
            &ui,
        )?;
        if report.exit_code != 0 {
            status = report.exit_code;
        }
    }
    Ok(status)
}

fn execute_test(args: TestArgs, globals: GlobalOptions) -> AppResult<i32> {
    let selector = test_selector(&args)?;
    let source = resolve_source(&args.source).map_err(AppError::new)?;
    let (config, ui) = configured(Some(&source.path), globals)?;
    run_saved_cases(&source, selector, mode(&config, globals), &config, &ui)
}

fn test_selector(args: &TestArgs) -> AppResult<CaseSelector> {
    if args.all {
        return Ok(CaseSelector::All);
    }
    if args.last {
        return Ok(CaseSelector::Last);
    }
    if !args.ids.is_empty() {
        return Ok(CaseSelector::Ids(args.ids.clone()));
    }
    if args.selectors.is_empty() {
        return Ok(CaseSelector::All);
    }

    let parts = args
        .selectors
        .iter()
        .flat_map(|selector| selector.split(','))
        .filter(|selector| !selector.is_empty())
        .collect::<Vec<_>>();
    if parts.is_empty() {
        return Err(AppError::usage("case selector cannot be empty"));
    }
    match parts.as_slice() {
        ["all"] => return Ok(CaseSelector::All),
        ["last"] => return Ok(CaseSelector::Last),
        _ if parts
            .iter()
            .any(|selector| matches!(*selector, "all" | "last")) =>
        {
            return Err(AppError::usage(
                "'all' and 'last' cannot be combined with other case selectors",
            ));
        }
        _ => {}
    }

    let ids = parts
        .into_iter()
        .map(|selector| {
            let id = selector.parse::<u64>().map_err(|_| {
                AppError::usage(format!(
                    "invalid case selector '{selector}'; use all, last, or positive IDs"
                ))
            })?;
            if id == 0 {
                return Err(AppError::usage("case IDs must be positive integers"));
            }
            Ok(id)
        })
        .collect::<AppResult<Vec<_>>>()?;
    Ok(CaseSelector::Ids(ids))
}

pub fn run_saved_cases(
    source: &crate::model::SourceSpec,
    selector: CaseSelector,
    build_mode: BuildMode,
    config: &Config,
    ui: &Ui,
) -> AppResult<i32> {
    let selected = match selector {
        CaseSelector::All => cases::list(source)?,
        CaseSelector::Last => {
            let last = cases::last(source)?;
            ui.info(format!("Most recent input  {}", last.path.display()));
            vec![last]
        }
        CaseSelector::Ids(ids) => {
            let mut seen = HashSet::new();
            let mut selected = Vec::new();
            for id in ids {
                if id == 0 {
                    return Err(AppError::usage("case IDs must be positive integers"));
                }
                if seen.insert(id) {
                    selected.push(cases::require(source, id)?);
                }
            }
            selected
        }
    };
    if selected.is_empty() {
        return Err(AppError::new(format!(
            "no saved inputs exist for '{}'",
            source.path.display()
        )));
    }
    let product = build::build(source, build_mode, config, ui)?;
    let mut status = 0;
    for saved in selected {
        show_saved_case(&saved.path, ui)?;
        let captured_output = ui
            .interactive()
            .then(|| temporary_path(config, "case-output"))
            .transpose()?;
        let output = captured_output
            .as_ref()
            .map(|path| OutputTarget::File(path.clone()))
            .unwrap_or(OutputTarget::Inherit);
        let result = process::run(
            &RunRequest {
                command: product.command.clone(),
                input: Some(saved.path),
                output,
                timeout: config.timeout,
                capture_input: None,
                show_report: false,
            },
            ui,
        );
        let report = match result {
            Ok(report) => report,
            Err(error) => {
                if let Some(path) = captured_output {
                    let _ = fs::remove_file(path);
                }
                return Err(error);
            }
        };
        if let Some(path) = captured_output {
            show_captured_output(&path)?;
            let _ = fs::remove_file(path);
        }
        ui.case_report(&report);
        if report.exit_code != 0 {
            status = report.exit_code;
        }
    }
    Ok(status)
}

fn show_saved_case(path: &Path, ui: &Ui) -> AppResult<()> {
    if !ui.interactive() {
        return Ok(());
    }
    ui.header("File", path.display().to_string());
    ui.section_title("Input");
    let contents = fs::read(path)?;
    io::stderr().write_all(&contents)?;
    if !contents.is_empty() && contents.last() != Some(&b'\n') {
        eprintln!();
    }
    ui.section_title("Output");
    Ok(())
}

fn show_captured_output(path: &Path) -> AppResult<()> {
    let contents = fs::read(path)?;
    io::stderr().write_all(&contents)?;
    if !contents.is_empty() && contents.last() != Some(&b'\n') {
        eprintln!();
    }
    Ok(())
}

fn execute_case(args: CaseArgs, globals: GlobalOptions) -> AppResult<i32> {
    let source_path = match &args.command {
        CaseCommand::List(args) => &args.source,
        CaseCommand::Show(args) | CaseCommand::Copy(args) | CaseCommand::Edit(args) => &args.source,
        CaseCommand::Paste(args) => &args.source,
        CaseCommand::Add(args) => &args.source,
        CaseCommand::Delete(args) => &args.source,
        CaseCommand::Clear(args) => &args.source,
    };
    let source = resolve_source(source_path).map_err(AppError::new)?;
    let (config, ui) = configured(Some(&source.path), globals)?;
    match args.command {
        CaseCommand::List(_) => {
            let saved = cases::list(&source)?;
            if saved.is_empty() {
                ui.info(format!("No saved inputs for {}", source.path.display()));
            }
            for case in saved {
                show_case(&case)?;
            }
            Ok(0)
        }
        CaseCommand::Show(args) => {
            show_case(&cases::require(&source, args.id)?)?;
            Ok(0)
        }
        CaseCommand::Copy(args) => {
            let saved = cases::require(&source, args.id)?;
            cases::copy(&saved)?;
            ui.success(format!("Copied {} to the clipboard", saved.path.display()));
            Ok(0)
        }
        CaseCommand::Paste(args) => {
            let target = match args.target.as_deref() {
                None | Some("next") => args.id,
                Some(value) => {
                    let id = value.parse::<u64>().map_err(|_| {
                        AppError::usage(format!(
                            "invalid case target '{value}'; use a positive ID or 'next'"
                        ))
                    })?;
                    if id == 0 {
                        return Err(AppError::usage("case IDs must be positive integers"));
                    }
                    Some(id)
                }
            };
            let saved = cases::paste(&source, target, &config)?;
            ui.success(format!(
                "Saved input #{}  {}",
                saved.id,
                saved.path.display()
            ));
            if args.run {
                run_saved_cases(
                    &source,
                    CaseSelector::Ids(vec![saved.id]),
                    mode(&config, globals),
                    &config,
                    &ui,
                )
            } else {
                Ok(0)
            }
        }
        CaseCommand::Add(_) => {
            edit_case_external(&source, None, &config, &ui)?;
            Ok(0)
        }
        CaseCommand::Edit(args) => {
            edit_case_external(&source, Some(args.id), &config, &ui)?;
            Ok(0)
        }
        CaseCommand::Delete(args) => {
            if !args.force {
                return Err(AppError::usage(
                    "case delete is destructive; pass --force or use the interactive REPL",
                ));
            }
            let path = cases::delete(&source, args.id, &config)?;
            ui.success(format!("Deleted {}", path.display()));
            Ok(0)
        }
        CaseCommand::Clear(args) => {
            if !args.force {
                return Err(AppError::usage(
                    "case clear is destructive; pass --force or use the interactive REPL",
                ));
            }
            let deleted = cases::clear(&source, &config)?;
            ui.success(format!("Cleared {} saved input(s)", deleted.len()));
            Ok(0)
        }
    }
}

pub fn edit_case_external(
    source: &crate::model::SourceSpec,
    id: Option<u64>,
    config: &Config,
    ui: &Ui,
) -> AppResult<()> {
    let outcome = cases::edit_with_external_editor(source, id, config)?;
    match outcome {
        cases::ExternalEditOutcome::Added(saved) => ui.success(format!(
            "Added input #{}  {}",
            saved.id,
            saved.path.display()
        )),
        cases::ExternalEditOutcome::Updated(saved) => ui.success(format!(
            "Updated input #{}  {}",
            saved.id,
            saved.path.display()
        )),
        cases::ExternalEditOutcome::Unchanged(saved) => ui.info(format!(
            "Input #{} unchanged  {}",
            saved.id,
            saved.path.display()
        )),
    }
    Ok(())
}

fn execute_compare(args: CompareArgs, globals: GlobalOptions) -> AppResult<i32> {
    let source = resolve_source(&args.source).map_err(AppError::new)?;
    let (config, ui) = configured(Some(&source.path), globals)?;
    let saved = cases::require(&source, args.id)?;
    if !args.expected.is_file() {
        return Err(AppError::new(format!(
            "expected output '{}' does not exist",
            args.expected.display()
        )));
    }
    let product = build::build(&source, mode(&config, globals), &config, &ui)?;
    let actual = temporary_path(&config, "actual-output")?;
    ui.header(
        "COMPARE",
        format!("{} · {}", saved.path.display(), args.expected.display()),
    );
    let report = process::run(
        &RunRequest {
            command: product.command,
            input: Some(saved.path.clone()),
            output: OutputTarget::File(actual.clone()),
            timeout: config.timeout,
            capture_input: None,
            show_report: true,
        },
        &ui,
    )?;
    if !report.success() {
        let _ = fs::remove_file(actual);
        return Ok(report.exit_code);
    }
    let same = fs::read(&actual)? == fs::read(&args.expected)?;
    if same {
        ui.success(format!("Output matches {}", args.expected.display()));
        let _ = fs::remove_file(actual);
        return Ok(0);
    }
    print_diff(&args.expected, &actual, "expected", "actual")?;
    ui.warning(format!("✗ Output differs from {}", args.expected.display()));
    let _ = fs::remove_file(actual);
    Ok(1)
}

fn execute_stress(args: StressArgs, globals: GlobalOptions) -> AppResult<i32> {
    let brute_path = args.brute.or(args.brute_option).ok_or_else(|| {
        AppError::usage("missing BRUTE source; usage: stress SOURCE BRUTE GENERATOR")
    })?;
    let generator_path = args.generator.or(args.generator_option).ok_or_else(|| {
        AppError::usage("missing GENERATOR source; usage: stress SOURCE BRUTE GENERATOR")
    })?;
    let source = resolve_source(&args.source).map_err(AppError::new)?;
    let (config, ui) = configured(Some(&source.path), globals)?;
    let brute = resolve_source(&brute_path).map_err(AppError::new)?;
    let generator = resolve_source(&generator_path).map_err(AppError::new)?;
    let limit = args.runs.unwrap_or(config.stress_limit);
    if limit == 0 {
        return Err(AppError::usage("--runs must be a positive integer"));
    }
    let timeout = args
        .timeout
        .map(|value| parse_duration(value, "--timeout"))
        .transpose()
        .map_err(AppError::usage)?
        .unwrap_or(config.stress_timeout);
    let request = StressRequest {
        source,
        brute,
        generator,
        mode: mode(&config, globals),
        limit,
        timeout,
    };
    run_stress(request, &config, &ui)
}

pub fn run_stress(request: StressRequest, config: &Config, ui: &Ui) -> AppResult<i32> {
    let main = build::build(&request.source, request.mode, config, ui)?;
    let brute = build::build(&request.brute, request.mode, config, ui)?;
    let generator = build::build(&request.generator, request.mode, config, ui)?;
    let directory = temporary_directory(config, "stress")?;
    let input = directory.join("input");
    let actual = directory.join("actual");
    let expected = directory.join("expected");
    ui.header(
        "STRESS",
        format!(
            "{} cases · {:.3}s per program",
            request.limit,
            request.timeout.as_secs_f64()
        ),
    );
    for case_number in 1..=request.limit {
        let generator_report = process::run(
            &RunRequest {
                command: generator.command.clone().arg(case_number.to_string()),
                input: None,
                output: OutputTarget::File(input.clone()),
                timeout: Some(request.timeout),
                capture_input: None,
                show_report: false,
            },
            ui,
        )?;
        if !generator_report.success() {
            let _ = fs::remove_dir_all(&directory);
            return Err(AppError::with_code(
                format!(
                    "generator failed on case {case_number} (exit {})",
                    generator_report.exit_code
                ),
                generator_report.exit_code,
            ));
        }
        let actual_report = process::run(
            &RunRequest {
                command: main.command.clone(),
                input: Some(input.clone()),
                output: OutputTarget::File(actual.clone()),
                timeout: Some(request.timeout),
                capture_input: None,
                show_report: false,
            },
            ui,
        )?;
        let expected_report = process::run(
            &RunRequest {
                command: brute.command.clone(),
                input: Some(input.clone()),
                output: OutputTarget::File(expected.clone()),
                timeout: Some(request.timeout),
                capture_input: None,
                show_report: false,
            },
            ui,
        )?;
        let same = actual_report.success()
            && expected_report.success()
            && fs::read(&actual)? == fs::read(&expected)?;
        if !same {
            let saved = cases::save_file_as_next(&request.source, &input, config)?;
            ui.warning(format!(
                "Mismatch on case {case_number} · saved {}",
                saved.path.display()
            ));
            ui.info(format!(
                "Program exit: {} · brute exit: {}",
                actual_report.exit_code, expected_report.exit_code
            ));
            print_diff(&expected, &actual, "brute", "program")?;
            let _ = fs::remove_dir_all(&directory);
            return Ok(1);
        }
        if case_number % 100 == 0 || case_number == request.limit {
            ui.info(format!("Passed {case_number}/{} case(s)", request.limit));
        }
    }
    let _ = fs::remove_dir_all(directory);
    ui.success(format!("Passed all {} stress case(s)", request.limit));
    Ok(0)
}

fn execute_doctor(globals: GlobalOptions) -> AppResult<i32> {
    let (config, ui) = configured(None, globals)?;
    ui.header("DOCTOR", "toolchain availability");
    let tools = [
        ("C++", config.toolchains.cxx.as_str()),
        ("C", config.toolchains.cc.as_str()),
        ("Python", config.toolchains.python.as_str()),
        ("Java compiler", config.toolchains.javac.as_str()),
        ("Java runtime", config.toolchains.java.as_str()),
        ("Rust", config.toolchains.rustc.as_str()),
        ("Go", config.toolchains.go.as_str()),
        ("Kotlin", config.toolchains.kotlinc.as_str()),
    ];
    let mut missing = 0;
    for (label, tool) in tools {
        if build::probe_tool(tool).is_ok() {
            println!("✓ {label:<14} {tool}");
        } else {
            println!("✗ {label:<14} {tool} (missing or unusable)");
            missing += 1;
        }
    }
    let clipboard = if cases::command_exists("wl-paste") && cases::command_exists("wl-copy") {
        "wl-copy/wl-paste"
    } else if cases::command_exists("xclip") {
        "xclip"
    } else {
        "missing"
    };
    println!(
        "{} {:<14} {clipboard}",
        if clipboard == "missing" { "✗" } else { "✓" },
        "Clipboard"
    );
    Ok(if missing == 0 { 0 } else { 1 })
}

fn mode(config: &Config, globals: GlobalOptions) -> BuildMode {
    globals.mode.unwrap_or(config.default_mode)
}

fn validate_exec_files(
    source: &Path,
    inputs: &[PathBuf],
    outputs: &[PathBuf],
    config: &Config,
) -> AppResult<()> {
    if !inputs.is_empty() && !outputs.is_empty() && inputs.len() != outputs.len() {
        return Err(AppError::usage(format!(
            "input/output file counts must match ({} input, {} output)",
            inputs.len(),
            outputs.len()
        )));
    }
    if inputs.is_empty() && outputs.len() > 1 {
        return Err(AppError::usage(
            "use exactly one output file when no input files are provided",
        ));
    }
    let source = absolute_lexical(source)?;
    let cache = absolute_lexical(&config.cache_dir)?;
    let input_paths = inputs
        .iter()
        .map(|path| absolute_lexical(path))
        .collect::<AppResult<Vec<_>>>()?;
    let mut seen = HashSet::new();
    for output in outputs {
        let absolute = absolute_lexical(output)?;
        if absolute == source {
            return Err(AppError::usage(format!(
                "output '{}' would overwrite the source file",
                output.display()
            )));
        }
        if input_paths.contains(&absolute) {
            return Err(AppError::usage(format!(
                "output '{}' would overwrite an input",
                output.display()
            )));
        }
        if absolute.starts_with(&cache) {
            return Err(AppError::usage(format!(
                "output '{}' cannot be saved inside the run-cli cache",
                output.display()
            )));
        }
        if !seen.insert(absolute.clone()) {
            return Err(AppError::usage(format!(
                "output '{}' is listed more than once",
                output.display()
            )));
        }
        if absolute.is_dir() {
            return Err(AppError::usage(format!(
                "output '{}' is a directory",
                output.display()
            )));
        }
        if absolute.is_file() && is_supported_source(&absolute) {
            return Err(AppError::usage(format!(
                "output '{}' would overwrite an existing source file",
                output.display()
            )));
        }
    }
    Ok(())
}

fn absolute_lexical(path: &Path) -> AppResult<PathBuf> {
    let joined = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()?.join(path)
    };
    let mut normalized = PathBuf::new();
    for component in joined.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                normalized.pop();
            }
            other => normalized.push(other.as_os_str()),
        }
    }
    Ok(normalized)
}

fn show_input(path: &Path, ui: &Ui) -> AppResult<()> {
    if !ui.interactive() {
        return Ok(());
    }
    ui.header("INPUT", path.display().to_string());
    let mut file = File::open(path)?;
    let mut contents = Vec::new();
    file.read_to_end(&mut contents)?;
    io::stderr().write_all(&contents)?;
    if !contents.is_empty() && contents.last() != Some(&b'\n') {
        eprintln!();
    }
    eprintln!();
    Ok(())
}

fn show_case(saved: &cases::SavedCase) -> AppResult<()> {
    let modified: DateTime<Local> = saved.modified.into();
    println!(
        "━━ INPUT #{} · {} ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━",
        saved.id,
        saved.path.display()
    );
    println!("  Modified  {}", modified.format("%Y-%m-%d %H:%M:%S %:z"));
    println!("  Size      {} bytes\n", saved.size);
    let contents = fs::read(&saved.path)?;
    io::stdout().write_all(&contents)?;
    if contents.is_empty() {
        println!("(empty)");
    } else if contents.last() != Some(&b'\n') {
        println!();
    }
    println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
    Ok(())
}

fn print_diff(
    expected: &Path,
    actual: &Path,
    expected_label: &str,
    actual_label: &str,
) -> AppResult<()> {
    match Command::new("diff")
        .arg("-u")
        .arg("--label")
        .arg(format!("{expected_label} ({})", expected.display()))
        .arg("--label")
        .arg(format!("{actual_label} ({})", actual.display()))
        .arg(expected)
        .arg(actual)
        .stdout(Stdio::inherit())
        .status()
    {
        Ok(_) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            eprintln!(
                "Expected:\n{}",
                String::from_utf8_lossy(&fs::read(expected)?)
            );
            eprintln!("Actual:\n{}", String::from_utf8_lossy(&fs::read(actual)?));
            Ok(())
        }
        Err(error) => Err(error.into()),
    }
}

fn temporary_path(config: &Config, prefix: &str) -> AppResult<PathBuf> {
    let directory = config.cache_dir.join("tmp");
    fs::create_dir_all(&directory)?;
    let count = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
    Ok(directory.join(format!("{prefix}-{}-{count}", std::process::id())))
}

fn temporary_directory(config: &Config, prefix: &str) -> AppResult<PathBuf> {
    let path = temporary_path(config, prefix)?;
    fs::create_dir(&path)?;
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_mismatched_batch_outputs() {
        let config = Config::default();
        let error = validate_exec_files(
            Path::new("a.cpp"),
            &[PathBuf::from("one.in"), PathBuf::from("two.in")],
            &[PathBuf::from("one.out")],
            &config,
        )
        .unwrap_err();
        assert_eq!(error.code, 2);
    }

    #[test]
    fn rejects_duplicate_outputs() {
        let config = Config::default();
        assert!(validate_exec_files(
            Path::new("a.cpp"),
            &[PathBuf::from("one.in"), PathBuf::from("two.in")],
            &[PathBuf::from("same.out"), PathBuf::from("same.out")],
            &config,
        )
        .is_err());
    }
}
