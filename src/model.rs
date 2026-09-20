use std::ffi::OsString;
use std::fmt;
use std::path::PathBuf;
use std::time::Duration;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Language {
    Cpp,
    Python,
    C,
    Java,
    Rust,
    Go,
    Kotlin,
}

impl Language {
    pub const ALL: [Self; 7] = [
        Self::Cpp,
        Self::Python,
        Self::C,
        Self::Java,
        Self::Rust,
        Self::Go,
        Self::Kotlin,
    ];

    pub fn extension(self) -> &'static str {
        match self {
            Self::Cpp => "cpp",
            Self::Python => "py",
            Self::C => "c",
            Self::Java => "java",
            Self::Rust => "rs",
            Self::Go => "go",
            Self::Kotlin => "kt",
        }
    }

    pub fn from_extension(extension: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|language| language.extension() == extension)
    }

    pub fn is_compiled(self) -> bool {
        self != Self::Python
    }
}

impl fmt::Display for Language {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = match self {
            Self::Cpp => "C++",
            Self::Python => "Python",
            Self::C => "C",
            Self::Java => "Java",
            Self::Rust => "Rust",
            Self::Go => "Go",
            Self::Kotlin => "Kotlin",
        };
        formatter.write_str(name)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BuildMode {
    #[default]
    Standard,
    Debug,
}

impl fmt::Display for BuildMode {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Standard => formatter.write_str("standard"),
            Self::Debug => formatter.write_str("debug"),
        }
    }
}

#[derive(Debug, Clone)]
pub struct SourceSpec {
    pub requested: PathBuf,
    pub path: PathBuf,
    pub stem: PathBuf,
    pub language: Language,
}

#[derive(Debug, Clone)]
pub struct CommandSpec {
    pub program: OsString,
    pub args: Vec<OsString>,
}

impl CommandSpec {
    pub fn new(program: impl Into<OsString>) -> Self {
        Self {
            program: program.into(),
            args: Vec::new(),
        }
    }

    pub fn arg(mut self, argument: impl Into<OsString>) -> Self {
        self.args.push(argument.into());
        self
    }

    pub fn args<I, S>(mut self, arguments: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<OsString>,
    {
        self.args.extend(arguments.into_iter().map(Into::into));
        self
    }
}

#[derive(Debug, Clone)]
pub struct BuildProduct {
    pub command: CommandSpec,
}

#[derive(Debug, Clone)]
pub struct RunRequest {
    pub command: CommandSpec,
    pub input: Option<PathBuf>,
    pub output: OutputTarget,
    pub timeout: Option<Duration>,
    pub capture_input: Option<PathBuf>,
    pub show_report: bool,
}

#[derive(Debug, Clone)]
pub enum OutputTarget {
    Inherit,
    AtomicFile(PathBuf),
    File(PathBuf),
}

#[derive(Debug, Clone)]
pub struct RunReport {
    pub exit_code: i32,
    pub interrupted: bool,
    pub wall_time: Duration,
    pub user_time: Duration,
    pub system_time: Duration,
    pub peak_memory_kib: i64,
    pub timed_out: bool,
}

impl RunReport {
    pub fn success(&self) -> bool {
        self.exit_code == 0 && !self.timed_out
    }
}

#[derive(Debug, Clone)]
pub enum CaseSelector {
    All,
    Last,
    Ids(Vec<u64>),
}

#[derive(Debug, Clone)]
pub struct StressRequest {
    pub source: SourceSpec,
    pub brute: SourceSpec,
    pub generator: SourceSpec,
    pub mode: BuildMode,
    pub limit: u64,
    pub timeout: Duration,
}
