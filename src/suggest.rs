use crate::source::is_supported_source;
use reedline::{Completer, Span, Suggestion};
use std::fs;
use std::path::{Path, PathBuf};

const COMMANDS: &[(&str, &str)] = &[
    ("/run", "build and run the active source"),
    ("/build", "compile without running"),
    ("/test", "run saved cases"),
    ("/case", "manage saved cases"),
    ("/diff", "compare a saved case"),
    ("/stress", "run differential stress tests"),
    ("/source", "switch the active source"),
    ("/mode", "change the build mode"),
    ("/mouse", "toggle mouse controls"),
    ("/status", "show session settings"),
    ("/doctor", "inspect installed tools"),
    ("/help", "show commands"),
    ("/quit", "exit the session"),
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PathKind {
    Source,
    Input,
    Output,
    Expected,
}

#[derive(Default)]
pub struct RunCompleter;

impl Completer for RunCompleter {
    fn complete(&mut self, line: &str, pos: usize) -> Vec<Suggestion> {
        let before = &line[..pos.min(line.len())];
        let (start, raw) = current_token(before);
        let tokens = loose_tokens(before);
        if tokens.len() <= 1 && !before.ends_with(char::is_whitespace) {
            return COMMANDS
                .iter()
                .filter(|(command, _)| command.starts_with(raw))
                .map(|(command, description)| Suggestion {
                    value: (*command).to_string(),
                    description: Some((*description).to_string()),
                    span: Span::new(start, pos),
                    append_whitespace: true,
                    ..Suggestion::default()
                })
                .collect();
        }

        let Some(kind) = expected_path_kind(&tokens, before.ends_with(char::is_whitespace)) else {
            return complete_literals(&tokens, raw, start, pos);
        };
        complete_paths(raw, start, pos, kind)
    }
}

fn current_token(line: &str) -> (usize, &str) {
    let start = line
        .char_indices()
        .rev()
        .find(|(_, character)| character.is_whitespace())
        .map(|(index, character)| index + character.len_utf8())
        .unwrap_or(0);
    let raw = line[start..].trim_start_matches(['\'', '"']);
    let quote_bytes = line[start..].len() - raw.len();
    (start + quote_bytes, raw)
}

fn loose_tokens(line: &str) -> Vec<String> {
    shell_words::split(line).unwrap_or_else(|_| {
        line.split_whitespace()
            .map(|token| token.trim_matches(['\'', '"']).to_string())
            .collect()
    })
}

fn expected_path_kind(tokens: &[String], trailing_space: bool) -> Option<PathKind> {
    let command = tokens.first()?.as_str();
    let current_index = if trailing_space {
        tokens.len()
    } else {
        tokens.len().saturating_sub(1)
    };
    match command {
        "/source" if current_index == 1 => Some(PathKind::Source),
        "/diff" if current_index == 2 => Some(PathKind::Expected),
        "/stress" if current_index == 1 || current_index == 2 => Some(PathKind::Source),
        "/run" => {
            let previous = if trailing_space {
                tokens.last().map(String::as_str)
            } else if current_index > 0 {
                tokens.get(current_index - 1).map(String::as_str)
            } else {
                None
            };
            match previous {
                Some("--input") | Some("-i") => Some(PathKind::Input),
                Some("--output") | Some("-o") => Some(PathKind::Output),
                _ => None,
            }
        }
        _ => None,
    }
}

fn complete_literals(tokens: &[String], raw: &str, start: usize, end: usize) -> Vec<Suggestion> {
    let values: &[(&str, &str)] = match tokens.first().map(String::as_str) {
        Some("/mode") => &[
            ("standard", "normal compiler settings"),
            ("debug", "sanitizers and debug checks"),
        ],
        Some("/mouse") => &[
            ("on", "enable picker mouse controls"),
            ("off", "disable mouse controls"),
        ],
        Some("/test") => &[
            ("all", "all saved cases"),
            ("last", "most recently modified case"),
        ],
        Some("/case") if tokens.len() <= 2 => &[
            ("list", "list cases"),
            ("show", "show a case"),
            ("copy", "copy a case"),
            ("paste", "paste a case"),
            ("delete", "delete a case"),
            ("clear", "delete all cases"),
        ],
        _ => &[],
    };
    values
        .iter()
        .filter(|(value, _)| value.starts_with(raw))
        .map(|(value, description)| Suggestion {
            value: (*value).to_string(),
            description: Some((*description).to_string()),
            span: Span::new(start, end),
            append_whitespace: true,
            ..Suggestion::default()
        })
        .collect()
}

fn complete_paths(raw: &str, start: usize, end: usize, kind: PathKind) -> Vec<Suggestion> {
    let expanded = expand_home(raw);
    let (directory, fragment) = if raw.ends_with('/') {
        (expanded.clone(), String::new())
    } else {
        (
            expanded
                .parent()
                .unwrap_or_else(|| Path::new("."))
                .to_path_buf(),
            expanded
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or_default()
                .to_string(),
        )
    };
    let Ok(entries) = fs::read_dir(&directory) else {
        return Vec::new();
    };
    let show_hidden = fragment.starts_with('.');
    let mut candidates = entries
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let name = entry.file_name().to_str()?.to_string();
            if !name.starts_with(&fragment) || (!show_hidden && name.starts_with('.')) {
                return None;
            }
            let path = entry.path();
            let is_dir = entry.file_type().ok()?.is_dir();
            if !is_dir && kind == PathKind::Source && !is_supported_source(&path) {
                return None;
            }
            if !is_dir && kind == PathKind::Output && is_supported_source(&path) {
                return None;
            }
            let rank = path_rank(&path, is_dir, kind);
            let mut value_path = PathBuf::from(raw);
            if !raw.ends_with('/') {
                value_path.pop();
            }
            value_path.push(&name);
            let mut value = value_path.to_string_lossy().to_string();
            if is_dir {
                value.push('/');
            }
            let description = if is_dir {
                "directory".to_string()
            } else if kind == PathKind::Output && path.exists() {
                "existing output · replace".to_string()
            } else {
                path.extension()
                    .and_then(|extension| extension.to_str())
                    .map(|extension| format!(".{extension} file"))
                    .unwrap_or_else(|| "file".to_string())
            };
            Some((rank, name, value, description, is_dir))
        })
        .collect::<Vec<_>>();
    candidates.sort_by(|left, right| left.0.cmp(&right.0).then_with(|| left.1.cmp(&right.1)));
    candidates
        .into_iter()
        .map(|(_, _, value, description, is_dir)| Suggestion {
            value: quote_if_needed(&value),
            description: Some(description),
            span: Span::new(start, end),
            append_whitespace: !is_dir,
            ..Suggestion::default()
        })
        .collect()
}

fn path_rank(path: &Path, is_dir: bool, kind: PathKind) -> u8 {
    if is_dir {
        return 0;
    }
    let extension = path
        .extension()
        .and_then(|ext| ext.to_str())
        .unwrap_or_default();
    match kind {
        PathKind::Source if is_supported_source(path) => 1,
        PathKind::Input
            if extension == "in" || extension == "txt" || has_saved_case_suffix(path) =>
        {
            1
        }
        PathKind::Expected if ["out", "ans", "txt"].contains(&extension) => 1,
        PathKind::Output if ["out", "ans", "txt"].contains(&extension) => 1,
        _ => 2,
    }
}

fn has_saved_case_suffix(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .and_then(|name| name.rsplit_once(".in"))
        .map(|(_, id)| !id.is_empty() && id.bytes().all(|byte| byte.is_ascii_digit()))
        .unwrap_or(false)
}

fn expand_home(raw: &str) -> PathBuf {
    if raw == "~" || raw.starts_with("~/") {
        if let Some(home) = std::env::var_os("HOME") {
            return PathBuf::from(home).join(raw.trim_start_matches("~/"));
        }
    }
    if raw.is_empty() {
        PathBuf::from(".")
    } else {
        PathBuf::from(raw)
    }
}

fn quote_if_needed(path: &str) -> String {
    if path.chars().any(char::is_whitespace) {
        shell_words::quote(path).into_owned()
    } else {
        path.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognizes_path_positions() {
        assert_eq!(
            expected_path_kind(&loose_tokens("/source "), true),
            Some(PathKind::Source)
        );
        assert_eq!(
            expected_path_kind(&loose_tokens("/run --input "), true),
            Some(PathKind::Input)
        );
        assert_eq!(
            expected_path_kind(&loose_tokens("/run --output x"), false),
            Some(PathKind::Output)
        );
        assert_eq!(
            expected_path_kind(&loose_tokens("/diff 1 "), true),
            Some(PathKind::Expected)
        );
    }

    #[test]
    fn hidden_files_require_dot_prefix() {
        assert!(!"file".starts_with('.'));
        assert!(".file".starts_with('.'));
    }
}
