use crate::model::BuildMode;
use serde::Deserialize;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(default)]
struct FileConfig {
    toolchains: ToolchainPatch,
    flags: FlagPatch,
    defaults: DefaultPatch,
    ui: UiPatch,
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(default)]
struct ToolchainPatch {
    cxx: Option<String>,
    cc: Option<String>,
    python: Option<String>,
    javac: Option<String>,
    java: Option<String>,
    rustc: Option<String>,
    go: Option<String>,
    kotlinc: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(default)]
struct FlagPatch {
    cpp: Option<Vec<String>>,
    cpp_debug: Option<Vec<String>>,
    c: Option<Vec<String>>,
    c_debug: Option<Vec<String>>,
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(default)]
struct DefaultPatch {
    mode: Option<String>,
    timeout: Option<f64>,
    stress_limit: Option<u64>,
    stress_timeout: Option<f64>,
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(default)]
struct UiPatch {
    color: Option<String>,
    mouse: Option<bool>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColorPolicy {
    Auto,
    Always,
    Never,
}

impl ColorPolicy {
    pub fn parse(value: &str) -> Result<Self, String> {
        match value {
            "auto" => Ok(Self::Auto),
            "always" => Ok(Self::Always),
            "never" => Ok(Self::Never),
            _ => Err(format!(
                "invalid color policy '{value}'; use auto, always, or never"
            )),
        }
    }
}

#[derive(Debug, Clone)]
pub struct Toolchains {
    pub cxx: String,
    pub cc: String,
    pub python: String,
    pub javac: String,
    pub java: String,
    pub rustc: String,
    pub go: String,
    pub kotlinc: String,
}

#[derive(Debug, Clone)]
pub struct Flags {
    pub cpp: Vec<String>,
    pub cpp_debug: Vec<String>,
    pub c: Vec<String>,
    pub c_debug: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct Config {
    pub toolchains: Toolchains,
    pub flags: Flags,
    pub default_mode: BuildMode,
    pub timeout: Option<Duration>,
    pub stress_limit: u64,
    pub stress_timeout: Duration,
    pub color: ColorPolicy,
    pub mouse: bool,
    pub cache_dir: PathBuf,
    pub state_dir: PathBuf,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            toolchains: Toolchains {
                cxx: "g++".into(),
                cc: "gcc".into(),
                python: "python3".into(),
                javac: "javac".into(),
                java: "java".into(),
                rustc: "rustc".into(),
                go: "go".into(),
                kotlinc: "kotlinc".into(),
            },
            flags: Flags {
                cpp: words("-std=c++17 -Wall -Wshadow -Wno-unused-result -DLOCAL"),
                cpp_debug: words("-std=c++17 -DLOCAL -g3 -Og -Wall -Wextra -pedantic -Wshadow -Wformat=2 -Wfloat-equal -Wconversion -Wlogical-op -Wshift-overflow=2 -Wduplicated-cond -Wduplicated-branches -Wcast-qual -Wcast-align -Wno-unused-result -D_GLIBCXX_DEBUG -D_GLIBCXX_DEBUG_PEDANTIC -D_FORTIFY_SOURCE=2 -fsanitize=address,undefined -fno-sanitize-recover=all -fno-omit-frame-pointer -fstack-protector-strong"),
                c: words("-std=c17 -Wall -Wextra -Wshadow -Wno-unused-result"),
                c_debug: words("-std=c17 -g3 -Og -Wall -Wextra -pedantic -Wshadow -Wconversion -Wformat=2 -Wno-unused-result -D_FORTIFY_SOURCE=2 -fsanitize=address,undefined -fno-sanitize-recover=all -fno-omit-frame-pointer -fstack-protector-strong"),
            },
            default_mode: BuildMode::Standard,
            timeout: None,
            stress_limit: 1000,
            stress_timeout: Duration::from_secs(2),
            color: ColorPolicy::Auto,
            mouse: false,
            cache_dir: xdg_dir("XDG_CACHE_HOME", ".cache").join("run-cli"),
            state_dir: xdg_dir("XDG_STATE_HOME", ".local/state").join("run-cli"),
        }
    }
}

impl Config {
    pub fn load(source: Option<&Path>) -> Result<Self, String> {
        let mut config = Self::default();
        let user_path = xdg_dir("XDG_CONFIG_HOME", ".config")
            .join("run-cli")
            .join("config.toml");
        if user_path.is_file() {
            config.apply_file(&user_path)?;
        }
        if let Some(project_path) = source.and_then(find_project_config) {
            config.apply_file(&project_path)?;
        }
        config.apply_environment()?;
        Ok(config)
    }

    fn apply_file(&mut self, path: &Path) -> Result<(), String> {
        let contents = fs::read_to_string(path)
            .map_err(|error| format!("cannot read config '{}': {error}", path.display()))?;
        let patch: FileConfig = toml::from_str(&contents)
            .map_err(|error| format!("invalid config '{}': {error}", path.display()))?;
        self.apply_patch(patch)
    }

    fn apply_patch(&mut self, patch: FileConfig) -> Result<(), String> {
        macro_rules! assign {
            ($target:expr, $source:expr) => {
                if let Some(value) = $source {
                    $target = value;
                }
            };
        }
        assign!(self.toolchains.cxx, patch.toolchains.cxx);
        assign!(self.toolchains.cc, patch.toolchains.cc);
        assign!(self.toolchains.python, patch.toolchains.python);
        assign!(self.toolchains.javac, patch.toolchains.javac);
        assign!(self.toolchains.java, patch.toolchains.java);
        assign!(self.toolchains.rustc, patch.toolchains.rustc);
        assign!(self.toolchains.go, patch.toolchains.go);
        assign!(self.toolchains.kotlinc, patch.toolchains.kotlinc);
        assign!(self.flags.cpp, patch.flags.cpp);
        assign!(self.flags.cpp_debug, patch.flags.cpp_debug);
        assign!(self.flags.c, patch.flags.c);
        assign!(self.flags.c_debug, patch.flags.c_debug);
        if let Some(mode) = patch.defaults.mode {
            self.default_mode = parse_mode(&mode)?;
        }
        if let Some(timeout) = patch.defaults.timeout {
            self.timeout = Some(parse_duration(timeout, "defaults.timeout")?);
        }
        if let Some(limit) = patch.defaults.stress_limit {
            if limit == 0 {
                return Err("defaults.stress_limit must be positive".into());
            }
            self.stress_limit = limit;
        }
        if let Some(timeout) = patch.defaults.stress_timeout {
            self.stress_timeout = parse_duration(timeout, "defaults.stress_timeout")?;
        }
        if let Some(color) = patch.ui.color {
            self.color = ColorPolicy::parse(&color)?;
        }
        assign!(self.mouse, patch.ui.mouse);
        Ok(())
    }

    fn apply_environment(&mut self) -> Result<(), String> {
        macro_rules! env_assign {
            ($name:literal, $target:expr) => {
                if let Ok(value) = env::var($name) {
                    if !value.is_empty() {
                        $target = value;
                    }
                }
            };
        }
        env_assign!("CXX", self.toolchains.cxx);
        env_assign!("CC", self.toolchains.cc);
        env_assign!("PYTHON", self.toolchains.python);
        env_assign!("JAVAC", self.toolchains.javac);
        env_assign!("JAVA", self.toolchains.java);
        env_assign!("RUSTC", self.toolchains.rustc);
        env_assign!("GO", self.toolchains.go);
        env_assign!("KOTLINC", self.toolchains.kotlinc);
        if env::var_os("NO_COLOR").is_some() {
            self.color = ColorPolicy::Never;
        }
        if let Ok(limit) = env::var("STRESS_LIMIT") {
            self.stress_limit = limit
                .parse::<u64>()
                .ok()
                .filter(|limit| *limit > 0)
                .ok_or_else(|| "STRESS_LIMIT must be a positive integer".to_string())?;
        }
        if let Ok(timeout) = env::var("STRESS_TIMEOUT") {
            let timeout = timeout
                .parse::<f64>()
                .map_err(|_| "STRESS_TIMEOUT must be a positive number".to_string())?;
            self.stress_timeout = parse_duration(timeout, "STRESS_TIMEOUT")?;
        }
        Ok(())
    }
}

fn parse_mode(value: &str) -> Result<BuildMode, String> {
    match value {
        "standard" => Ok(BuildMode::Standard),
        "debug" => Ok(BuildMode::Debug),
        _ => Err(format!(
            "invalid build mode '{value}'; use standard or debug"
        )),
    }
}

pub fn parse_duration(value: f64, name: &str) -> Result<Duration, String> {
    if !value.is_finite() || value <= 0.0 {
        return Err(format!("{name} must be a positive number of seconds"));
    }
    Ok(Duration::from_secs_f64(value))
}

fn find_project_config(source: &Path) -> Option<PathBuf> {
    let start = if source.is_dir() {
        source
    } else {
        source.parent().unwrap_or_else(|| Path::new("."))
    };
    let absolute = fs::canonicalize(start)
        .ok()
        .unwrap_or_else(|| start.to_path_buf());
    absolute
        .ancestors()
        .map(|directory| directory.join(".run-cli.toml"))
        .find(|candidate| candidate.is_file())
}

fn xdg_dir(variable: &str, fallback: &str) -> PathBuf {
    env::var_os(variable)
        .map(PathBuf::from)
        .or_else(|| env::var_os("HOME").map(|home| PathBuf::from(home).join(fallback)))
        .unwrap_or_else(|| PathBuf::from(".").join(fallback))
}

fn words(value: &str) -> Vec<String> {
    value.split_ascii_whitespace().map(str::to_owned).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn durations_must_be_positive_and_finite() {
        assert!(parse_duration(0.1, "timeout").is_ok());
        assert!(parse_duration(0.0, "timeout").is_err());
        assert!(parse_duration(-1.0, "timeout").is_err());
        assert!(parse_duration(f64::NAN, "timeout").is_err());
    }

    #[test]
    fn default_flags_match_runner_baseline() {
        let config = Config::default();
        assert!(config.flags.cpp.contains(&"-std=c++17".to_string()));
        assert!(config
            .flags
            .cpp_debug
            .contains(&"-fsanitize=address,undefined".to_string()));
    }
}
