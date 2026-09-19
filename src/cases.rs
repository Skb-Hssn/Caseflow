use crate::config::Config;
use crate::error::{AppError, AppResult};
use crate::model::SourceSpec;
use std::collections::hash_map::DefaultHasher;
use std::fs::{self, File, OpenOptions};
use std::hash::{Hash, Hasher};
use std::io::{self, Write};
use std::os::fd::AsRawFd;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::SystemTime;

static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Clone)]
pub struct SavedCase {
    pub id: u64,
    pub path: PathBuf,
    pub size: u64,
    pub modified: SystemTime,
}

pub struct CaseLock {
    file: File,
}

impl Drop for CaseLock {
    fn drop(&mut self) {
        unsafe {
            libc::flock(self.file.as_raw_fd(), libc::LOCK_UN);
        }
    }
}

pub fn lock_cases(source: &SourceSpec, config: &Config) -> AppResult<CaseLock> {
    let directory = config.cache_dir.join("locks");
    fs::create_dir_all(&directory)?;
    let key = canonical_stem(source);
    let mut hasher = DefaultHasher::new();
    key.hash(&mut hasher);
    let path = directory.join(format!("{:016x}.lock", hasher.finish()));
    let file = OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .truncate(false)
        .open(&path)
        .map_err(|error| {
            AppError::new(format!(
                "cannot open case lock '{}': {error}",
                path.display()
            ))
        })?;
    let status = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX) };
    if status != 0 {
        return Err(AppError::new(format!(
            "cannot lock saved cases: {}",
            io::Error::last_os_error()
        )));
    }
    Ok(CaseLock { file })
}

pub fn list(source: &SourceSpec) -> AppResult<Vec<SavedCase>> {
    let parent = parent_or_current(&source.stem);
    let stem_name = source
        .stem
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| AppError::new("source stem is not valid UTF-8"))?;
    let prefix = format!("{stem_name}.in");
    let entries = match fs::read_dir(parent) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error.into()),
    };
    let mut cases = Vec::new();
    for entry in entries.filter_map(Result::ok) {
        let name = entry.file_name();
        let Some(name) = name.to_str() else { continue };
        let Some(suffix) = name.strip_prefix(&prefix) else {
            continue;
        };
        if suffix.starts_with('0')
            || suffix.is_empty()
            || !suffix.bytes().all(|byte| byte.is_ascii_digit())
        {
            continue;
        }
        let Ok(id) = suffix.parse::<u64>() else {
            continue;
        };
        if id == 0 {
            continue;
        }
        let path = entry.path();
        let Ok(metadata) = entry.metadata() else {
            continue;
        };
        if !metadata.is_file() {
            continue;
        }
        cases.push(SavedCase {
            id,
            path,
            size: metadata.len(),
            modified: metadata.modified().unwrap_or(SystemTime::UNIX_EPOCH),
        });
    }
    cases.sort_by_key(|case| case.id);
    Ok(cases)
}

pub fn require(source: &SourceSpec, id: u64) -> AppResult<SavedCase> {
    if id == 0 {
        return Err(AppError::usage("case IDs must be positive integers"));
    }
    list(source)?
        .into_iter()
        .find(|case| case.id == id)
        .ok_or_else(|| {
            AppError::new(format!(
                "saved input ID {id} ('{}') does not exist",
                case_path(source, id).display()
            ))
        })
}

pub fn last(source: &SourceSpec) -> AppResult<SavedCase> {
    list(source)?
        .into_iter()
        .max_by(|left, right| {
            left.modified
                .cmp(&right.modified)
                .then_with(|| left.id.cmp(&right.id))
        })
        .ok_or_else(|| {
            AppError::new(format!(
                "no saved inputs exist for '{}'",
                source.path.display()
            ))
        })
}

pub fn next_id(source: &SourceSpec) -> AppResult<u64> {
    list(source)?
        .last()
        .map(|case| {
            case.id
                .checked_add(1)
                .ok_or_else(|| AppError::new("saved case ID overflow"))
        })
        .unwrap_or(Ok(1))
}

pub fn save_bytes(
    source: &SourceSpec,
    id: Option<u64>,
    contents: &[u8],
    config: &Config,
) -> AppResult<SavedCase> {
    let _lock = lock_cases(source, config)?;
    let id = match id {
        Some(0) => return Err(AppError::usage("case IDs must be positive integers")),
        Some(id) => id,
        None => next_id(source)?,
    };
    let path = case_path(source, id);
    atomic_write(&path, contents)?;
    require(source, id)
}

pub fn save_file_as_next(
    source: &SourceSpec,
    captured: &Path,
    config: &Config,
) -> AppResult<SavedCase> {
    let contents = fs::read(captured).map_err(|error| {
        AppError::new(format!(
            "cannot read captured input '{}': {error}",
            captured.display()
        ))
    })?;
    save_bytes(source, None, &contents, config)
}

pub fn delete(source: &SourceSpec, id: u64, config: &Config) -> AppResult<PathBuf> {
    let _lock = lock_cases(source, config)?;
    let saved = require(source, id)?;
    fs::remove_file(&saved.path).map_err(|error| {
        AppError::new(format!("cannot delete '{}': {error}", saved.path.display()))
    })?;
    Ok(saved.path)
}

pub fn clear(source: &SourceSpec, config: &Config) -> AppResult<Vec<PathBuf>> {
    let _lock = lock_cases(source, config)?;
    let cases = list(source)?;
    let mut deleted = Vec::new();
    for saved in cases {
        fs::remove_file(&saved.path).map_err(|error| {
            AppError::new(format!("cannot delete '{}': {error}", saved.path.display()))
        })?;
        deleted.push(saved.path);
    }
    Ok(deleted)
}

pub fn paste(source: &SourceSpec, id: Option<u64>, config: &Config) -> AppResult<SavedCase> {
    let output = if command_exists("wl-paste") {
        Command::new("wl-paste").output()
    } else if command_exists("xclip") {
        Command::new("xclip")
            .args(["-selection", "clipboard", "-o"])
            .output()
    } else {
        return Err(AppError::new(
            "clipboard mode requires 'wl-paste' or 'xclip'",
        ));
    }
    .map_err(|error| AppError::new(format!("could not read the clipboard: {error}")))?;
    if !output.status.success() {
        return Err(AppError::new(format!(
            "clipboard command failed with exit status {}",
            output.status.code().unwrap_or(1)
        )));
    }
    save_bytes(source, id, &output.stdout, config)
}

pub fn copy(saved: &SavedCase) -> AppResult<()> {
    let input = File::open(&saved.path)?;
    let status = if command_exists("wl-copy") {
        Command::new("wl-copy").stdin(Stdio::from(input)).status()
    } else if command_exists("xclip") {
        Command::new("xclip")
            .args(["-selection", "clipboard", "-i"])
            .stdin(Stdio::from(input))
            .status()
    } else {
        return Err(AppError::new("copy mode requires 'wl-copy' or 'xclip'"));
    }
    .map_err(|error| AppError::new(format!("could not write the clipboard: {error}")))?;
    if status.success() {
        Ok(())
    } else {
        Err(AppError::new(format!(
            "clipboard command failed with exit status {}",
            status.code().unwrap_or(1)
        )))
    }
}

pub fn case_path(source: &SourceSpec, id: u64) -> PathBuf {
    PathBuf::from(format!("{}.in{id}", source.stem.display()))
}

pub fn command_exists(command: &str) -> bool {
    std::env::var_os("PATH")
        .map(|paths| std::env::split_paths(&paths).any(|path| path.join(command).is_file()))
        .unwrap_or(false)
}

fn atomic_write(destination: &Path, contents: &[u8]) -> AppResult<()> {
    let parent = parent_or_current(destination);
    let name = destination
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| AppError::new("case path is not valid UTF-8"))?;
    let (temporary, mut file) = (0..100)
        .find_map(|_| {
            let count = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
            let path = parent.join(format!(
                ".{name}.run-cli.{}.{count}.tmp",
                std::process::id()
            ));
            match OpenOptions::new().write(true).create_new(true).open(&path) {
                Ok(file) => Some(Ok((path, file))),
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => None,
                Err(error) => Some(Err(AppError::new(format!(
                    "cannot create temporary case beside '{}': {error}",
                    destination.display()
                )))),
            }
        })
        .transpose()?
        .ok_or_else(|| AppError::new("could not allocate a temporary case file"))?;
    if let Err(error) = file.write_all(contents).and_then(|_| file.sync_all()) {
        let _ = fs::remove_file(&temporary);
        return Err(error.into());
    }
    drop(file);
    fs::rename(&temporary, destination).map_err(|error| {
        let _ = fs::remove_file(&temporary);
        AppError::new(format!(
            "cannot save case '{}': {error}",
            destination.display()
        ))
    })
}

fn canonical_stem(source: &SourceSpec) -> PathBuf {
    let parent = parent_or_current(&source.stem);
    let parent = fs::canonicalize(parent).unwrap_or_else(|_| parent.to_path_buf());
    match source.stem.file_name() {
        Some(name) => parent.join(name),
        None => parent,
    }
}

fn parent_or_current(path: &Path) -> &Path {
    path.parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::source::resolve_source;

    fn unique_dir() -> PathBuf {
        let count = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
        std::env::temp_dir().join(format!("run-cli-cases-{}-{count}", std::process::id()))
    }

    #[test]
    fn ids_are_sorted_and_gaps_are_not_reused() {
        let directory = unique_dir();
        fs::create_dir_all(&directory).unwrap();
        let source = resolve_source(directory.join("a.cpp")).unwrap();
        fs::write(case_path(&source, 3), "three").unwrap();
        fs::write(case_path(&source, 1), "one").unwrap();
        fs::write(directory.join("a.in02"), "ignored").unwrap();
        let ids = list(&source)
            .unwrap()
            .into_iter()
            .map(|case| case.id)
            .collect::<Vec<_>>();
        assert_eq!(ids, vec![1, 3]);
        assert_eq!(next_id(&source).unwrap(), 4);
        let _ = fs::remove_dir_all(directory);
    }

    #[test]
    fn relative_case_paths_use_the_current_directory() {
        let source = resolve_source("a.cpp").unwrap();
        assert_eq!(parent_or_current(&source.stem), Path::new("."));
        assert_eq!(case_path(&source, 1), PathBuf::from("a.in1"));
    }
}
