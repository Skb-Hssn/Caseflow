use clap::{Args, Parser, Subcommand, ValueEnum, ValueHint};
use std::path::PathBuf;

#[derive(Debug, Parser)]
#[command(
    name = "run-cli",
    version,
    about = "Interactive competitive-programming runner",
    subcommand_precedence_over_arg = true
)]
pub struct Cli {
    /// Color output policy.
    #[arg(long, global = true, value_enum)]
    pub color: Option<ColorArg>,

    /// Enable mouse controls in interactive pickers and dialogs.
    #[arg(long, global = true)]
    pub mouse: bool,

    /// Use debug compiler/runtime settings.
    #[arg(long, global = true)]
    pub debug: bool,

    #[command(subcommand)]
    pub command: Option<Command>,

    /// Open an interactive session with this source active.
    #[arg(value_hint = ValueHint::FilePath)]
    pub source: Option<PathBuf>,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum ColorArg {
    Auto,
    Always,
    Never,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Build if necessary and execute a source.
    #[command(alias = "exec")]
    Run(RunArgs),
    /// Compile a source without running it.
    Build(BuildArgs),
    /// Run saved input cases.
    Test(TestArgs),
    /// Manage saved input cases.
    Case(CaseArgs),
    /// Compare a saved case with expected output.
    #[command(alias = "diff")]
    Compare(CompareArgs),
    /// Compare generated cases against a brute-force solution.
    Stress(StressArgs),
    /// Inspect installed toolchains and clipboard support.
    Doctor,
    /// Generate shell completions.
    #[command(alias = "completions")]
    Completion(CompletionArgs),
    /// Internal completion protocol used by generated shell integrations.
    #[command(name = "__complete", hide = true)]
    Complete(CompleteArgs),
}

#[derive(Debug, Args)]
pub struct RunArgs {
    #[arg(value_hint = ValueHint::FilePath)]
    pub source: PathBuf,

    #[arg(long, value_name = "SECONDS")]
    pub timeout: Option<f64>,

    /// Save the clipboard as the next case and run it.
    #[arg(long, conflicts_with_all = ["save_input", "inputs", "outputs"])]
    pub clipboard: bool,

    /// Save interactive stdin as the next <stem>.in<ID> case.
    #[arg(long = "save", alias = "save-input")]
    pub save_input: bool,

    /// Read from an input file. Repeat for batch execution.
    #[arg(short = 'i', long = "input", value_hint = ValueHint::FilePath)]
    pub inputs: Vec<PathBuf>,

    /// Atomically save stdout. Repeat once per input in batch mode.
    #[arg(short = 'o', long = "output", value_hint = ValueHint::FilePath)]
    pub outputs: Vec<PathBuf>,
}

#[derive(Debug, Args)]
pub struct BuildArgs {
    #[arg(value_hint = ValueHint::FilePath)]
    pub source: PathBuf,
}

#[derive(Debug, Args)]
pub struct TestArgs {
    #[arg(value_hint = ValueHint::FilePath)]
    pub source: PathBuf,

    /// Case selectors: all, last, or one or more IDs. Defaults to all.
    #[arg(value_name = "CASE")]
    pub selectors: Vec<String>,

    #[arg(long, hide = true, conflicts_with_all = ["last", "ids", "selectors"])]
    pub all: bool,

    #[arg(long, hide = true, conflicts_with_all = ["all", "ids", "selectors"])]
    pub last: bool,

    #[arg(
        long = "id",
        hide = true,
        value_delimiter = ',',
        conflicts_with_all = ["all", "last", "selectors"]
    )]
    pub ids: Vec<u64>,
}

#[derive(Debug, Args)]
pub struct CaseArgs {
    #[command(subcommand)]
    pub command: CaseCommand,
}

#[derive(Debug, Subcommand)]
pub enum CaseCommand {
    /// List saved inputs for a source.
    List(CaseSource),
    /// Print a saved input.
    Show(CaseId),
    /// Copy a saved input to the clipboard.
    Copy(CaseId),
    /// Save clipboard text as a case; appends by default.
    Paste(CasePaste),
    /// Create the next case in Vim, Neovim, or your configured editor.
    Add(CaseSource),
    /// Edit an existing case in Vim, Neovim, or your configured editor.
    Edit(CaseId),
    /// Delete one saved input.
    Delete(CaseDelete),
    /// Delete every saved input for a source.
    Clear(CaseClear),
}

#[derive(Debug, Args)]
pub struct CaseSource {
    #[arg(value_hint = ValueHint::FilePath)]
    pub source: PathBuf,
}

#[derive(Debug, Args)]
pub struct CaseId {
    #[arg(value_hint = ValueHint::FilePath)]
    pub source: PathBuf,
    /// Positive saved-case ID.
    pub id: u64,
}

#[derive(Debug, Args)]
pub struct CasePaste {
    #[arg(value_hint = ValueHint::FilePath)]
    pub source: PathBuf,

    /// Replace this case ID. Without an ID, append the next case.
    pub target: Option<String>,

    /// Replace this case ID. Without --id, append the next ID.
    #[arg(long, hide = true, conflicts_with_all = ["next", "target"])]
    pub id: Option<u64>,

    /// Explicitly append the next ID (the default).
    #[arg(long, hide = true, conflicts_with_all = ["id", "target"])]
    pub next: bool,

    /// Run the pasted case after saving it.
    #[arg(long)]
    pub run: bool,
}

#[derive(Debug, Args)]
pub struct CaseDelete {
    #[arg(value_hint = ValueHint::FilePath)]
    pub source: PathBuf,
    pub id: u64,
    #[arg(long)]
    pub force: bool,
}

#[derive(Debug, Args)]
pub struct CaseClear {
    #[arg(value_hint = ValueHint::FilePath)]
    pub source: PathBuf,
    #[arg(long)]
    pub force: bool,
}

#[derive(Debug, Args)]
pub struct CompareArgs {
    #[arg(value_hint = ValueHint::FilePath)]
    pub source: PathBuf,
    pub id: u64,
    #[arg(value_hint = ValueHint::FilePath)]
    pub expected: PathBuf,
}

#[derive(Debug, Args)]
pub struct StressArgs {
    #[arg(value_hint = ValueHint::FilePath)]
    pub source: PathBuf,

    /// Trusted solution used to produce expected output.
    #[arg(value_hint = ValueHint::FilePath)]
    pub brute: Option<PathBuf>,

    /// Program that prints one generated input per invocation.
    #[arg(value_hint = ValueHint::FilePath)]
    pub generator: Option<PathBuf>,

    #[arg(
        long = "brute",
        hide = true,
        value_hint = ValueHint::FilePath,
        conflicts_with = "brute"
    )]
    pub brute_option: Option<PathBuf>,

    #[arg(
        long = "generator",
        hide = true,
        value_hint = ValueHint::FilePath,
        conflicts_with = "generator"
    )]
    pub generator_option: Option<PathBuf>,

    /// Maximum generated cases.
    #[arg(long = "runs", alias = "limit")]
    pub runs: Option<u64>,

    #[arg(long, value_name = "SECONDS")]
    pub timeout: Option<f64>,
}

#[derive(Debug, Args)]
pub struct CompletionArgs {
    #[arg(value_enum)]
    pub shell: clap_complete::Shell,
}

#[derive(Debug, Args)]
pub struct CompleteArgs {
    #[arg(long)]
    pub line: String,

    #[arg(long)]
    pub cursor: Option<usize>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    #[test]
    fn bare_source_opens_session() {
        let cli = Cli::try_parse_from(["run-cli", "a.cpp"]).unwrap();
        assert_eq!(cli.source, Some(PathBuf::from("a.cpp")));
        assert!(cli.command.is_none());
    }

    #[test]
    fn parses_batch_run_and_legacy_exec() {
        let cli = Cli::try_parse_from([
            "run-cli", "run", "a.cpp", "-i", "one.in", "-i", "two.in", "-o", "one.out", "-o",
            "two.out",
        ])
        .unwrap();
        match cli.command.unwrap() {
            Command::Run(args) => {
                assert_eq!(args.inputs.len(), 2);
                assert_eq!(args.outputs.len(), 2);
            }
            _ => panic!("expected run"),
        }

        let legacy = Cli::try_parse_from(["run-cli", "exec", "a.cpp"]).unwrap();
        assert!(matches!(legacy.command, Some(Command::Run(_))));
    }

    #[test]
    fn parses_canonical_test_selectors() {
        let all = Cli::try_parse_from(["run-cli", "test", "a.cpp"]).unwrap();
        let Some(Command::Test(all)) = all.command else {
            panic!("expected test");
        };
        assert!(all.selectors.is_empty());

        let ids = Cli::try_parse_from(["run-cli", "test", "a.cpp", "1", "3,5"]).unwrap();
        let Some(Command::Test(ids)) = ids.command else {
            panic!("expected test");
        };
        assert_eq!(ids.selectors, ["1", "3,5"]);

        let last = Cli::try_parse_from(["run-cli", "test", "a.cpp", "last"]).unwrap();
        assert!(matches!(last.command, Some(Command::Test(_))));
    }

    #[test]
    fn parses_canonical_and_legacy_compare_and_stress() {
        for command in ["compare", "diff"] {
            let cli = Cli::try_parse_from(["run-cli", command, "a.cpp", "1", "a.out"]).unwrap();
            assert!(matches!(cli.command, Some(Command::Compare(_))));
        }

        let canonical = Cli::try_parse_from([
            "run-cli",
            "stress",
            "a.cpp",
            "brute.cpp",
            "gen.py",
            "--runs",
            "25",
        ])
        .unwrap();
        let Some(Command::Stress(canonical)) = canonical.command else {
            panic!("expected stress");
        };
        assert_eq!(canonical.brute, Some(PathBuf::from("brute.cpp")));
        assert_eq!(canonical.generator, Some(PathBuf::from("gen.py")));
        assert_eq!(canonical.runs, Some(25));

        let legacy = Cli::try_parse_from([
            "run-cli",
            "stress",
            "a.cpp",
            "--brute",
            "brute.cpp",
            "--generator",
            "gen.py",
            "--limit",
            "25",
        ])
        .unwrap();
        let Some(Command::Stress(legacy)) = legacy.command else {
            panic!("expected stress");
        };
        assert_eq!(legacy.brute_option, Some(PathBuf::from("brute.cpp")));
        assert_eq!(legacy.generator_option, Some(PathBuf::from("gen.py")));
        assert_eq!(legacy.runs, Some(25));
    }

    #[test]
    fn parses_case_editor_commands_and_default_paste() {
        let add = Cli::try_parse_from(["run-cli", "case", "add", "a.cpp"]).unwrap();
        assert!(matches!(
            add.command,
            Some(Command::Case(CaseArgs {
                command: CaseCommand::Add(_)
            }))
        ));

        let edit = Cli::try_parse_from(["run-cli", "case", "edit", "a.cpp", "4"]).unwrap();
        assert!(matches!(
            edit.command,
            Some(Command::Case(CaseArgs {
                command: CaseCommand::Edit(CaseId { id: 4, .. })
            }))
        ));

        let paste = Cli::try_parse_from(["run-cli", "case", "paste", "a.cpp"]).unwrap();
        let Some(Command::Case(CaseArgs {
            command: CaseCommand::Paste(paste),
        })) = paste.command
        else {
            panic!("expected case paste");
        };
        assert_eq!(paste.target, None);
    }

    #[test]
    fn clipboard_run_rejects_file_modes() {
        assert!(
            Cli::try_parse_from(["run-cli", "run", "a.cpp", "--clipboard", "--input", "a.in"])
                .is_err()
        );
    }

    #[test]
    fn long_help_exposes_only_canonical_command_names() {
        let help = Cli::command().render_long_help().to_string();
        assert!(help.contains("  run "));
        assert!(help.contains("  compare "));
        assert!(help.contains("  completion "));
        assert!(!help.contains("  exec "));
        assert!(!help.contains("  diff "));
        assert!(!help.contains("  completions "));
    }
}
