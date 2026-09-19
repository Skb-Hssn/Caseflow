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
    Exec(ExecArgs),
    /// Compile a source without running it.
    Build(BuildArgs),
    /// Run saved input cases.
    Test(TestArgs),
    /// Manage saved input cases.
    Case(CaseArgs),
    /// Compare a saved case with expected output.
    Diff(DiffArgs),
    /// Compare generated cases against a brute-force solution.
    Stress(StressArgs),
    /// Inspect installed toolchains and clipboard support.
    Doctor,
    /// Generate shell completions.
    Completions(CompletionsArgs),
    /// Internal completion protocol used by generated shell integrations.
    #[command(name = "__complete", hide = true)]
    Complete(CompleteArgs),
}

#[derive(Debug, Args)]
pub struct ExecArgs {
    #[arg(value_hint = ValueHint::FilePath)]
    pub source: PathBuf,

    #[arg(long)]
    pub debug: bool,

    #[arg(long, value_name = "SECONDS")]
    pub timeout: Option<f64>,

    /// Save interactive stdin as the next <stem>.in<ID> case.
    #[arg(long, alias = "save")]
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

    #[arg(long)]
    pub debug: bool,
}

#[derive(Debug, Args)]
pub struct TestArgs {
    #[arg(value_hint = ValueHint::FilePath)]
    pub source: PathBuf,

    #[arg(long, conflicts_with_all = ["last", "ids"])]
    pub all: bool,

    #[arg(long, conflicts_with_all = ["all", "ids"])]
    pub last: bool,

    #[arg(long = "id", value_delimiter = ',', conflicts_with_all = ["all", "last"])]
    pub ids: Vec<u64>,

    #[arg(long)]
    pub debug: bool,
}

#[derive(Debug, Args)]
pub struct CaseArgs {
    #[command(subcommand)]
    pub command: CaseCommand,
}

#[derive(Debug, Subcommand)]
pub enum CaseCommand {
    List(CaseSource),
    Show(CaseId),
    Copy(CaseId),
    Paste(CasePaste),
    Delete(CaseDelete),
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
    pub id: u64,
}

#[derive(Debug, Args)]
pub struct CasePaste {
    #[arg(value_hint = ValueHint::FilePath)]
    pub source: PathBuf,

    /// Replace this case ID. Without --id, append the next ID.
    #[arg(long, conflicts_with = "next")]
    pub id: Option<u64>,

    /// Explicitly append the next ID (the default).
    #[arg(long, conflicts_with = "id")]
    pub next: bool,

    /// Run the pasted case after saving it.
    #[arg(long)]
    pub run: bool,

    #[arg(long)]
    pub debug: bool,
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
pub struct DiffArgs {
    #[arg(value_hint = ValueHint::FilePath)]
    pub source: PathBuf,
    pub id: u64,
    #[arg(value_hint = ValueHint::FilePath)]
    pub expected: PathBuf,
    #[arg(long)]
    pub debug: bool,
}

#[derive(Debug, Args)]
pub struct StressArgs {
    #[arg(value_hint = ValueHint::FilePath)]
    pub source: PathBuf,

    #[arg(long, value_hint = ValueHint::FilePath)]
    pub brute: PathBuf,

    #[arg(long, value_hint = ValueHint::FilePath)]
    pub generator: PathBuf,

    #[arg(long)]
    pub limit: Option<u64>,

    #[arg(long, value_name = "SECONDS")]
    pub timeout: Option<f64>,

    #[arg(long)]
    pub debug: bool,
}

#[derive(Debug, Args)]
pub struct CompletionsArgs {
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

    #[test]
    fn bare_source_opens_session() {
        let cli = Cli::try_parse_from(["run-cli", "a.cpp"]).unwrap();
        assert_eq!(cli.source, Some(PathBuf::from("a.cpp")));
        assert!(cli.command.is_none());
    }

    #[test]
    fn parses_batch_exec() {
        let cli = Cli::try_parse_from([
            "run-cli", "exec", "a.cpp", "-i", "one.in", "-i", "two.in", "-o", "one.out", "-o",
            "two.out",
        ])
        .unwrap();
        match cli.command.unwrap() {
            Command::Exec(args) => {
                assert_eq!(args.inputs.len(), 2);
                assert_eq!(args.outputs.len(), 2);
            }
            _ => panic!("expected exec"),
        }
    }
}
