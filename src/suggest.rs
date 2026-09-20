use crate::source::is_supported_source;
use std::fs;
use std::path::{Path, PathBuf};

const REPL_COMMANDS: &[(&str, &str)] = &[
    ("/run", "build and run the active source"),
    ("/build", "compile without running"),
    ("/test", "run saved cases"),
    ("/case", "manage saved cases"),
    ("/compare", "compare a saved case"),
    ("/stress", "run differential stress tests"),
    ("/open", "switch the active source"),
    ("/debug", "toggle debug builds"),
    ("/mouse", "toggle mouse controls"),
    ("/status", "show session settings"),
    ("/again", "repeat the previous run or test"),
    ("/clear", "clear retained output"),
    ("/doctor", "inspect installed tools"),
    ("/help", "show commands"),
    ("/exit", "exit the session"),
];

const CLI_COMMANDS: &[(&str, &str)] = &[
    ("run", "build and execute a source"),
    ("build", "compile without running"),
    ("test", "run saved cases"),
    ("case", "manage saved cases"),
    ("compare", "compare a saved case"),
    ("stress", "run differential stress tests"),
    ("doctor", "inspect installed tools"),
    ("completion", "generate shell completion"),
];

const CLI_COMMAND_ALIASES: &[&str] = &["exec", "diff", "completions"];

const HELP_TOPICS: &[(&str, &str)] = &[
    ("run", "run with interactive, clipboard, or file input"),
    ("build", "compile without running"),
    ("test", "run saved cases"),
    ("case", "manage saved cases"),
    ("compare", "compare a saved case"),
    ("stress", "run differential stress tests"),
    ("open", "switch the active source"),
    ("debug", "toggle debug builds"),
    ("mouse", "toggle mouse controls"),
    ("status", "show session settings"),
    ("again", "repeat the previous run or test"),
    ("clear", "clear retained output"),
    ("doctor", "inspect installed tools"),
    ("help", "show commands"),
    ("exit", "exit the session"),
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Completion {
    pub value: String,
    pub description: String,
    pub replace_start: usize,
    pub replace_end: usize,
    pub append_whitespace: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PathKind {
    Source,
    Input,
    Output,
    Expected,
}

pub fn complete_repl(line: &str, cursor: usize) -> Vec<Completion> {
    complete(line, cursor, true)
}

pub fn complete_cli(line: &str, cursor: usize) -> Vec<Completion> {
    complete(line, cursor, false)
}

fn complete(line: &str, cursor: usize, repl: bool) -> Vec<Completion> {
    let mut cursor = cursor.min(line.len());
    while !line.is_char_boundary(cursor) {
        cursor = cursor.saturating_sub(1);
    }
    let before = &line[..cursor];
    let (start, raw) = current_token(before);
    let mut tokens = loose_tokens(before);
    let trailing_space = before.chars().last().is_some_and(char::is_whitespace);

    if !repl
        && tokens.first().is_some_and(|token| {
            Path::new(token)
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name == "run-cli" || name == "run-cli.exe")
        })
    {
        tokens.remove(0);
    }

    if repl && (tokens.is_empty() || (tokens.len() == 1 && !trailing_space)) {
        return literal_completions(REPL_COMMANDS, raw, start, cursor);
    }
    if !repl {
        let current = current_index(&tokens, trailing_space);
        if previous_token(&tokens, current, trailing_space) == Some("--color") {
            return literal_completions(
                &[
                    ("auto", "color only on terminals"),
                    ("always", "always emit color"),
                    ("never", "disable color"),
                ],
                raw,
                start,
                cursor,
            );
        }
        if cli_command_position(&tokens, trailing_space) {
            if raw.starts_with('-') {
                return literal_completions(
                    &[
                        ("--color", "set color output policy"),
                        ("--mouse", "enable interactive mouse controls"),
                        ("--debug", "use debug build settings"),
                    ],
                    raw,
                    start,
                    cursor,
                );
            }
            return literal_completions(CLI_COMMANDS, raw, start, cursor);
        }
    }

    if let Some(kind) = expected_path_kind(&tokens, trailing_space, repl) {
        return complete_paths(raw, start, cursor, kind);
    }

    let literals = expected_literals(&tokens, trailing_space, repl);
    literal_completions(&literals, raw, start, cursor)
}

fn cli_command_position(tokens: &[String], trailing_space: bool) -> bool {
    let current = current_index(tokens, trailing_space);
    let mut index = 0;
    while index < current {
        match tokens[index].as_str() {
            "--mouse" | "--debug" => index += 1,
            "--color" => index += 2,
            value if value.starts_with("--color=") => index += 1,
            _ => return false,
        }
    }
    index == current
        && tokens
            .get(current)
            .map(|token| !is_cli_command(token))
            .unwrap_or(true)
}

fn is_cli_command(token: &str) -> bool {
    CLI_COMMANDS.iter().any(|(command, _)| token == *command)
        || CLI_COMMAND_ALIASES.contains(&token)
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

fn command_index(tokens: &[String], repl: bool) -> Option<usize> {
    if repl {
        tokens.iter().position(|token| token.starts_with('/'))
    } else {
        tokens.iter().position(|token| is_cli_command(token))
    }
}

fn current_index(tokens: &[String], trailing_space: bool) -> usize {
    if trailing_space {
        tokens.len()
    } else {
        tokens.len().saturating_sub(1)
    }
}

fn previous_token(tokens: &[String], current: usize, trailing_space: bool) -> Option<&str> {
    if trailing_space {
        tokens.last().map(String::as_str)
    } else {
        current
            .checked_sub(1)
            .and_then(|index| tokens.get(index))
            .map(String::as_str)
    }
}

fn expected_path_kind(tokens: &[String], trailing_space: bool, repl: bool) -> Option<PathKind> {
    let command_index = command_index(tokens, repl)?;
    let command = tokens.get(command_index)?.as_str();
    let current = current_index(tokens, trailing_space);
    let relative = current.saturating_sub(command_index);
    let previous = previous_token(tokens, current, trailing_space);

    match command {
        "/open" | "/source" if relative == 1 => Some(PathKind::Source),
        "/compare" | "/diff" if relative == 2 => Some(PathKind::Expected),
        "/stress"
            if (relative == 1
                && tokens
                    .get(current)
                    .is_none_or(|token| !token.starts_with('-')))
                || (relative == 2
                    && previous.is_some_and(|token| !token.starts_with('-'))
                    && tokens
                        .get(current)
                        .is_none_or(|token| !token.starts_with('-'))) =>
        {
            Some(PathKind::Source)
        }
        "/run" => match previous {
            Some("--input") | Some("-i") => Some(PathKind::Input),
            Some("--output") | Some("-o") => Some(PathKind::Output),
            _ => None,
        },
        "run" | "exec" => {
            if relative == 1 {
                Some(PathKind::Source)
            } else {
                match previous {
                    Some("--input") | Some("-i") => Some(PathKind::Input),
                    Some("--output") | Some("-o") => Some(PathKind::Output),
                    _ => None,
                }
            }
        }
        "build" | "test" if relative == 1 => Some(PathKind::Source),
        "compare" | "diff" if relative == 1 => Some(PathKind::Source),
        "compare" | "diff" if relative == 3 => Some(PathKind::Expected),
        "stress" if relative == 1 => Some(PathKind::Source),
        "stress"
            if (relative == 2
                && tokens
                    .get(current)
                    .is_none_or(|token| !token.starts_with('-')))
                || (relative == 3
                    && previous.is_some_and(|token| !token.starts_with('-'))
                    && tokens
                        .get(current)
                        .is_none_or(|token| !token.starts_with('-'))) =>
        {
            Some(PathKind::Source)
        }
        "stress" if matches!(previous, Some("--brute") | Some("--generator")) => {
            Some(PathKind::Source)
        }
        "case" if relative == 2 => Some(PathKind::Source),
        _ => None,
    }
}

fn expected_literals(
    tokens: &[String],
    trailing_space: bool,
    repl: bool,
) -> Vec<(&'static str, &'static str)> {
    let Some(command_index) = command_index(tokens, repl) else {
        return Vec::new();
    };
    let command = tokens
        .get(command_index)
        .map(String::as_str)
        .unwrap_or_default();
    let current = current_index(tokens, trailing_space);
    let relative = current.saturating_sub(command_index);
    match command {
        "/debug" => vec![
            ("on", "enable debug builds"),
            ("off", "disable debug builds"),
            ("toggle", "toggle debug builds"),
        ],
        "/mode" => vec![
            ("standard", "normal compiler settings"),
            ("debug", "sanitizers and debug checks"),
        ],
        "/mouse" => vec![
            ("on", "enable mouse controls"),
            ("off", "disable mouse controls"),
            ("toggle", "toggle mouse controls"),
        ],
        "/test" => vec![
            ("all", "all saved cases"),
            ("last", "most recently modified case"),
        ],
        "/case" if relative <= 1 => case_actions(),
        "/run" if relative <= 1 => repl_run_options(),
        "/run" if tokens.get(command_index + 1).map(String::as_str) == Some("clipboard") => {
            clipboard_run_options()
        }
        "/run" => repl_run_options_only(),
        "/case" => repl_case_options(tokens, command_index),
        "/stress" => stress_options(),
        "/help" => HELP_TOPICS.to_vec(),
        "run" | "exec"
            if tokens[command_index + 1..]
                .iter()
                .any(|token| token == "--clipboard") =>
        {
            clipboard_run_options()
        }
        "run" | "exec" => run_options(),
        "build" => vec![("--debug", "use debug build settings")],
        "test" => vec![
            ("all", "all saved cases"),
            ("last", "most recently modified case"),
            ("--debug", "use debug build settings"),
        ],
        "case" if relative <= 1 => case_actions(),
        "case" => cli_case_options(tokens, command_index),
        "stress" => stress_options(),
        "completion" | "completions" => vec![
            ("bash", "Bash completion"),
            ("elvish", "Elvish completion"),
            ("fish", "Fish completion"),
            ("powershell", "PowerShell completion"),
            ("zsh", "Zsh completion"),
        ],
        _ => Vec::new(),
    }
}

fn run_options() -> Vec<(&'static str, &'static str)> {
    let mut options = vec![("--clipboard", "save clipboard input and run")];
    options.extend(repl_run_options_only());
    options
}

fn clipboard_run_options() -> Vec<(&'static str, &'static str)> {
    vec![
        ("--timeout", "set a time limit"),
        ("--debug", "use debug build settings"),
    ]
}

fn repl_run_options_only() -> Vec<(&'static str, &'static str)> {
    vec![
        ("--input", "read a named input"),
        ("--output", "save stdout atomically"),
        ("--timeout", "set a time limit"),
        ("--save", "save interactive stdin"),
        ("--debug", "use debug build settings"),
    ]
}

fn repl_run_options() -> Vec<(&'static str, &'static str)> {
    let mut options = vec![
        ("interactive", "run with terminal input"),
        ("clipboard", "save clipboard as the next case and run"),
    ];
    options.extend(repl_run_options_only());
    options
}

fn case_actions() -> Vec<(&'static str, &'static str)> {
    vec![
        ("list", "list cases"),
        ("show", "show a case"),
        ("copy", "copy a case"),
        ("paste", "paste a case"),
        ("add", "add a case in an external editor"),
        ("edit", "edit a case in an external editor"),
        ("delete", "delete a case"),
        ("clear", "delete all cases"),
    ]
}

fn repl_case_options(tokens: &[String], command_index: usize) -> Vec<(&'static str, &'static str)> {
    match tokens.get(command_index + 1).map(String::as_str) {
        Some("paste") => vec![("--run", "run the case after saving it")],
        _ => Vec::new(),
    }
}

fn cli_case_options(tokens: &[String], command_index: usize) -> Vec<(&'static str, &'static str)> {
    match tokens.get(command_index + 1).map(String::as_str) {
        Some("paste") => vec![
            ("--run", "run the case after saving it"),
            ("--debug", "use debug build settings"),
        ],
        Some("delete") | Some("clear") => {
            vec![("--force", "confirm destructive action")]
        }
        _ => Vec::new(),
    }
}

fn stress_options() -> Vec<(&'static str, &'static str)> {
    vec![
        ("--runs", "number of generated cases"),
        ("--timeout", "per-process time limit"),
        ("--debug", "use debug build settings"),
    ]
}

fn literal_completions(
    values: &[(&str, &str)],
    raw: &str,
    start: usize,
    end: usize,
) -> Vec<Completion> {
    values
        .iter()
        .filter(|(value, _)| value.starts_with(raw))
        .map(|(value, description)| Completion {
            value: (*value).to_string(),
            description: (*description).to_string(),
            replace_start: start,
            replace_end: end,
            append_whitespace: true,
        })
        .collect()
}

fn complete_paths(raw: &str, start: usize, end: usize, kind: PathKind) -> Vec<Completion> {
    let expanded = expand_home(raw);
    let (directory, fragment) = if raw.ends_with('/') {
        (expanded.clone(), String::new())
    } else {
        (
            expanded
                .parent()
                .filter(|parent| !parent.as_os_str().is_empty())
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
        .map(|(_, _, value, description, is_dir)| Completion {
            value: quote_if_needed(&value),
            description,
            replace_start: start,
            replace_end: end,
            append_whitespace: !is_dir,
        })
        .collect()
}

fn path_rank(path: &Path, is_dir: bool, kind: PathKind) -> u8 {
    if is_dir {
        return 0;
    }
    let extension = path
        .extension()
        .and_then(|extension| extension.to_str())
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
    fn recognizes_repl_path_positions() {
        assert_eq!(
            expected_path_kind(&loose_tokens("/run --input "), true, true),
            Some(PathKind::Input)
        );
        assert_eq!(
            expected_path_kind(&loose_tokens("/compare 1 "), true, true),
            Some(PathKind::Expected)
        );
        assert_eq!(
            expected_path_kind(&loose_tokens("/open "), true, true),
            Some(PathKind::Source)
        );
        assert_eq!(
            expected_path_kind(&loose_tokens("/diff 1 "), true, true),
            Some(PathKind::Expected)
        );
        assert_eq!(
            expected_path_kind(&loose_tokens("/source "), true, true),
            Some(PathKind::Source)
        );
    }

    #[test]
    fn recognizes_cli_path_positions() {
        assert_eq!(
            expected_path_kind(&loose_tokens("run-cli run "), true, false),
            Some(PathKind::Source)
        );
        assert_eq!(
            expected_path_kind(&loose_tokens("run-cli exec "), true, false),
            Some(PathKind::Source)
        );
        assert_eq!(
            expected_path_kind(&loose_tokens("run-cli compare a.cpp 1 "), true, false),
            Some(PathKind::Expected)
        );
        assert_eq!(
            expected_path_kind(&loose_tokens("run-cli diff a.cpp 1 "), true, false),
            Some(PathKind::Expected)
        );
        for line in [
            "run-cli stress a.cpp ",
            "run-cli stress a.cpp brute.cpp ",
            "run-cli stress a.cpp --brute ",
        ] {
            assert_eq!(
                expected_path_kind(&loose_tokens(line), true, false),
                Some(PathKind::Source),
                "{line}"
            );
        }
    }

    #[test]
    fn command_completion_is_shared() {
        let repl = complete_repl("/st", 3);
        assert!(repl.iter().any(|candidate| candidate.value == "/stress"));
        let cli = complete_cli("run-cli st", 10);
        assert!(cli.iter().any(|candidate| candidate.value == "stress"));
    }

    #[test]
    fn discovery_uses_canonical_commands_only() {
        let repl = complete_repl("/", 1)
            .into_iter()
            .map(|candidate| candidate.value)
            .collect::<Vec<_>>();
        for command in ["/open", "/debug", "/compare", "/again", "/clear", "/exit"] {
            assert!(repl.contains(&command.to_string()), "missing {command}");
        }
        for alias in ["/source", "/mode", "/diff", "/quit"] {
            assert!(!repl.contains(&alias.to_string()), "exposed alias {alias}");
        }

        let cli = complete_cli("run-cli ", "run-cli ".len())
            .into_iter()
            .map(|candidate| candidate.value)
            .collect::<Vec<_>>();
        for command in ["run", "compare", "completion"] {
            assert!(cli.contains(&command.to_string()), "missing {command}");
        }
        for alias in ["exec", "diff", "completions"] {
            assert!(!cli.contains(&alias.to_string()), "exposed alias {alias}");
        }
    }

    #[test]
    fn run_modes_are_suggested_before_run_options() {
        let line = "/run ";
        let candidates = complete_repl(line, line.len());
        assert!(candidates.iter().any(|item| item.value == "interactive"));
        assert!(candidates.iter().any(|item| item.value == "clipboard"));

        let line = "/run clipboard ";
        let candidates = complete_repl(line, line.len());
        assert!(candidates.iter().any(|item| item.value == "--timeout"));

        let line = "run-cli exec a.cpp --";
        let candidates = complete_cli(line, line.len());
        assert!(candidates.iter().any(|item| item.value == "--clipboard"));
        assert!(candidates.iter().any(|item| item.value == "--save"));
        assert!(!candidates.iter().any(|item| item.value == "--save-input"));
    }

    #[test]
    fn new_command_forms_have_focused_suggestions() {
        for command in ["/debug ", "/mouse "] {
            let values = complete_repl(command, command.len())
                .into_iter()
                .map(|candidate| candidate.value)
                .collect::<Vec<_>>();
            assert_eq!(values, ["on", "off", "toggle"]);
        }
        let legacy_mode = complete_repl("/mode ", 6)
            .into_iter()
            .map(|candidate| candidate.value)
            .collect::<Vec<_>>();
        assert_eq!(legacy_mode, ["standard", "debug"]);

        let case_actions = complete_repl("/case ", 6)
            .into_iter()
            .map(|candidate| candidate.value)
            .collect::<Vec<_>>();
        assert!(case_actions.contains(&"add".to_string()));
        assert!(case_actions.contains(&"edit".to_string()));
        let cli_case_actions = complete_cli("run-cli case ", "run-cli case ".len())
            .into_iter()
            .map(|candidate| candidate.value)
            .collect::<Vec<_>>();
        assert!(cli_case_actions.contains(&"add".to_string()));
        assert!(cli_case_actions.contains(&"edit".to_string()));

        let repl_test = complete_repl("/test ", 6)
            .into_iter()
            .map(|candidate| candidate.value)
            .collect::<Vec<_>>();
        assert!(repl_test.contains(&"all".to_string()));
        assert!(repl_test.contains(&"last".to_string()));

        let test = complete_cli("run-cli test a.cpp ", "run-cli test a.cpp ".len())
            .into_iter()
            .map(|candidate| candidate.value)
            .collect::<Vec<_>>();
        assert!(test.contains(&"all".to_string()));
        assert!(test.contains(&"last".to_string()));
        assert!(!test.contains(&"--all".to_string()));

        let stress_line = "run-cli stress a.cpp brute.cpp gen.py --";
        let stress = complete_cli(stress_line, stress_line.len())
            .into_iter()
            .map(|candidate| candidate.value)
            .collect::<Vec<_>>();
        assert!(stress.contains(&"--runs".to_string()));
        assert!(!stress.contains(&"--limit".to_string()));
        assert!(!stress.contains(&"--brute".to_string()));

        let repl_stress_line = "/stress brute.cpp gen.py --";
        let repl_stress = complete_repl(repl_stress_line, repl_stress_line.len())
            .into_iter()
            .map(|candidate| candidate.value)
            .collect::<Vec<_>>();
        assert!(repl_stress.contains(&"--runs".to_string()));
    }

    #[test]
    fn completion_command_suggests_supported_shells_for_alias_too() {
        for command in ["completion", "completions"] {
            let line = format!("run-cli {command} ");
            let values = complete_cli(&line, line.len())
                .into_iter()
                .map(|candidate| candidate.value)
                .collect::<Vec<_>>();
            assert!(values.contains(&"bash".to_string()));
            assert!(values.contains(&"zsh".to_string()));
            assert!(values.contains(&"fish".to_string()));
        }
    }

    #[test]
    fn cli_command_completion_skips_global_options() {
        let line = "run-cli --color never st";
        let candidates = complete_cli(line, line.len());
        assert_eq!(
            candidates.first().map(|item| item.value.as_str()),
            Some("stress")
        );

        let line = "run-cli --color ";
        let candidates = complete_cli(line, line.len());
        assert!(candidates.iter().any(|item| item.value == "never"));
    }

    #[test]
    fn hidden_files_require_dot_prefix() {
        assert!(!"file".starts_with('.'));
        assert!(".file".starts_with('.'));
    }
}
