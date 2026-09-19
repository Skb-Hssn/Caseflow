use crate::config::Config;
use crate::error::{AppError, AppResult};
use crate::model::{BuildMode, BuildProduct, CommandSpec, Language, SourceSpec};
use crate::ui::Ui;
use std::collections::hash_map::DefaultHasher;
use std::fs;
use std::hash::{Hash, Hasher};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;

pub fn build(
    source: &SourceSpec,
    mode: BuildMode,
    config: &Config,
    ui: &Ui,
) -> AppResult<BuildProduct> {
    if !source.path.is_file() {
        return Err(AppError::new(format!(
            "source file '{}' does not exist",
            source.path.display()
        )));
    }

    let artifact_dir = artifact_dir(source, mode, config)?;
    fs::create_dir_all(&artifact_dir).map_err(|error| {
        AppError::new(format!(
            "cannot create build directory '{}': {error}",
            artifact_dir.display()
        ))
    })?;

    if source.language == Language::Python {
        require_tool(&config.toolchains.python)?;
        let mut command = CommandSpec::new(&config.toolchains.python);
        if mode == BuildMode::Debug {
            command = command.args(["-X", "dev"]);
        }
        command = command.arg(source.path.as_os_str());
        return Ok(BuildProduct { command });
    }

    let (compile, run) = build_commands(source, mode, config, &artifact_dir)?;
    require_tool(&compile.program.to_string_lossy())?;
    ui.header("BUILD", format!("{} · {mode}", source.path.display()));
    ui.info(format!("Compiler  {}", compile.program.to_string_lossy()));
    if ui.interactive() {
        ui.info(format!("Command   {}", compile.display()));
    }

    let status = Command::new(&compile.program)
        .args(&compile.args)
        .status()
        .map_err(|error| {
            AppError::new(format!(
                "could not start compiler '{}': {error}",
                compile.program.to_string_lossy()
            ))
        })?;
    if !status.success() {
        let code = status.code().unwrap_or(1);
        return Err(AppError::with_code(
            format!("compilation failed with exit status {code}"),
            code,
        ));
    }
    ui.success("Compilation finished");

    Ok(BuildProduct { command: run })
}

fn build_commands(
    source: &SourceSpec,
    mode: BuildMode,
    config: &Config,
    output: &Path,
) -> AppResult<(CommandSpec, CommandSpec)> {
    let program = output.join("program");
    let source_path = source.path.as_os_str();
    match source.language {
        Language::Cpp => {
            let flags = if mode == BuildMode::Debug {
                &config.flags.cpp_debug
            } else {
                &config.flags.cpp
            };
            let compile = CommandSpec::new(&config.toolchains.cxx)
                .args(flags.iter().cloned())
                .arg("-o")
                .arg(program.as_os_str())
                .arg(source_path);
            Ok((compile, CommandSpec::new(program.as_os_str())))
        }
        Language::C => {
            let flags = if mode == BuildMode::Debug {
                &config.flags.c_debug
            } else {
                &config.flags.c
            };
            let compile = CommandSpec::new(&config.toolchains.cc)
                .args(flags.iter().cloned())
                .arg("-o")
                .arg(program.as_os_str())
                .arg(source_path);
            Ok((compile, CommandSpec::new(program.as_os_str())))
        }
        Language::Rust => {
            let mut compile = CommandSpec::new(&config.toolchains.rustc).arg("--edition=2021");
            if mode == BuildMode::Debug {
                compile = compile.args([
                    "-C",
                    "debuginfo=2",
                    "-C",
                    "debug-assertions=yes",
                    "-C",
                    "overflow-checks=yes",
                ]);
            }
            compile = compile.arg("-o").arg(program.as_os_str()).arg(source_path);
            Ok((compile, CommandSpec::new(program.as_os_str())))
        }
        Language::Go => {
            let mut compile = CommandSpec::new(&config.toolchains.go).arg("build");
            if mode == BuildMode::Debug {
                compile = compile.args(["-race", "-gcflags=all=-N -l"]);
            }
            compile = compile.arg("-o").arg(program.as_os_str()).arg(source_path);
            Ok((compile, CommandSpec::new(program.as_os_str())))
        }
        Language::Java => {
            require_tool(&config.toolchains.java)?;
            let classes = output.join("classes");
            fs::create_dir_all(&classes)?;
            let mut compile = CommandSpec::new(&config.toolchains.javac);
            if mode == BuildMode::Debug {
                compile = compile.args(["-g", "-Xlint:all"]);
            }
            compile = compile.arg("-d").arg(classes.as_os_str()).arg(source_path);
            let main_class = java_main_class(&source.path)?;
            let mut run = CommandSpec::new(&config.toolchains.java);
            if mode == BuildMode::Debug {
                run = run.arg("-ea");
            }
            run = run.arg("-cp").arg(classes.as_os_str()).arg(main_class);
            Ok((compile, run))
        }
        Language::Kotlin => {
            require_tool(&config.toolchains.java)?;
            let jar = output.join("program.jar");
            let mut compile = CommandSpec::new(&config.toolchains.kotlinc);
            if mode == BuildMode::Debug {
                compile = compile.args(["-Xdebug", "-Xassertions=jvm"]);
            }
            compile = compile
                .arg(source_path)
                .arg("-include-runtime")
                .arg("-d")
                .arg(jar.as_os_str());
            let mut run = CommandSpec::new(&config.toolchains.java);
            if mode == BuildMode::Debug {
                run = run.arg("-ea");
            }
            run = run.arg("-jar").arg(jar.as_os_str());
            Ok((compile, run))
        }
        Language::Python => unreachable!("Python is handled before build_commands"),
    }
}

fn artifact_dir(source: &SourceSpec, mode: BuildMode, config: &Config) -> AppResult<PathBuf> {
    let canonical = fs::canonicalize(&source.path).unwrap_or_else(|_| source.path.clone());
    let mut hasher = DefaultHasher::new();
    canonical.hash(&mut hasher);
    source.language.extension().hash(&mut hasher);
    let hash = hasher.finish();
    Ok(config
        .cache_dir
        .join("build")
        .join(format!("{hash:016x}"))
        .join(mode.to_string()))
}

fn java_main_class(source: &Path) -> AppResult<String> {
    let contents = fs::read_to_string(source)?;
    let package = contents.lines().find_map(|line| {
        let line = line.trim();
        let rest = line.strip_prefix("package ")?;
        let name = rest.strip_suffix(';')?.trim();
        (!name.is_empty()).then(|| name.to_string())
    });
    let class = source
        .file_stem()
        .and_then(|stem| stem.to_str())
        .ok_or_else(|| AppError::new("Java source filename is not valid UTF-8"))?;
    Ok(match package {
        Some(package) => format!("{package}.{class}"),
        None => class.to_string(),
    })
}

pub fn require_tool(tool: &str) -> AppResult<()> {
    let path = Path::new(tool);
    if tool.contains('/') {
        if is_executable(path) {
            return Ok(());
        }
    } else if let Some(paths) = std::env::var_os("PATH") {
        if std::env::split_paths(&paths).any(|directory| is_executable(&directory.join(tool))) {
            return Ok(());
        }
    }
    Err(AppError::new(format!(
        "required tool '{tool}' was not found or is not executable"
    )))
}

pub fn probe_tool(tool: &str) -> AppResult<()> {
    require_tool(tool)?;
    let name = Path::new(tool)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or(tool);
    let version_argument = match name {
        "go" => "version",
        "kotlinc" => "-version",
        _ => "--version",
    };
    let status = Command::new(tool)
        .arg(version_argument)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map_err(|error| AppError::new(format!("could not probe tool '{tool}': {error}")))?;
    if status.success() {
        Ok(())
    } else {
        Err(AppError::new(format!(
            "tool '{tool}' is installed but unusable (exit {})",
            status.code().unwrap_or(1)
        )))
    }
}

fn is_executable(path: &Path) -> bool {
    fs::metadata(path)
        .map(|metadata| metadata.is_file() && metadata.permissions().mode() & 0o111 != 0)
        .unwrap_or(false)
}

#[cfg(test)]
pub fn compiler_command(source: &SourceSpec, config: &Config) -> std::ffi::OsString {
    match source.language {
        Language::Cpp => (&config.toolchains.cxx).into(),
        Language::C => (&config.toolchains.cc).into(),
        Language::Python => (&config.toolchains.python).into(),
        Language::Java => (&config.toolchains.javac).into(),
        Language::Rust => (&config.toolchains.rustc).into(),
        Language::Go => (&config.toolchains.go).into(),
        Language::Kotlin => (&config.toolchains.kotlinc).into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::source::resolve_source;

    #[test]
    fn java_package_is_part_of_main_class() {
        let directory = std::env::temp_dir().join(format!("run-cli-java-{}", std::process::id()));
        let _ = fs::create_dir_all(&directory);
        let path = directory.join("Main.java");
        fs::write(&path, "package hello.world;\nclass Main {}\n").unwrap();
        assert_eq!(java_main_class(&path).unwrap(), "hello.world.Main");
        let _ = fs::remove_dir_all(directory);
    }

    #[test]
    fn compiler_selection_matches_language() {
        let config = Config::default();
        let source = resolve_source("a.rs").unwrap();
        assert_eq!(
            compiler_command(&source, &config),
            std::ffi::OsString::from("rustc")
        );
    }
}
