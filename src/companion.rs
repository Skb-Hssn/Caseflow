use crate::cases;
use crate::config::Config;
use crate::error::{AppError, AppResult};
use crate::model::SourceSpec;
use crate::process;
use crate::ui::Ui;
use serde::Deserialize;
use std::fs::{self, OpenOptions};
use std::io::{self, Read, Write};
use std::net::{Ipv4Addr, Ipv6Addr, SocketAddr, TcpListener, TcpStream};
use std::path::PathBuf;
use std::thread;
use std::time::{Duration, Instant};

const MAX_HEADER_BYTES: usize = 64 * 1024;
const MAX_BODY_BYTES: usize = 8 * 1024 * 1024;
const MAX_CONTEST_PROBLEMS: usize = 512;
const MAX_CONTEST_SAMPLE_BYTES: usize = 64 * 1024 * 1024;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CompanionProblem {
    name: String,
    #[serde(default)]
    group: String,
    #[serde(default)]
    url: String,
    #[serde(default)]
    interactive: bool,
    memory_limit: Option<u64>,
    time_limit: Option<u64>,
    tests: Vec<CompanionTest>,
    #[serde(default)]
    batch: Option<CompanionBatch>,
}

#[derive(Debug, Deserialize)]
struct CompanionTest {
    input: String,
    output: String,
}

#[derive(Debug, Deserialize)]
struct CompanionBatch {
    #[serde(default)]
    id: String,
    size: usize,
}

struct HttpError {
    status: &'static str,
    message: String,
}

pub fn receive(
    source: &SourceSpec,
    config: &Config,
    ui: &Ui,
    port: u16,
    wait: Duration,
    contest: bool,
) -> AppResult<i32> {
    let listeners = bind_loopback(port)?;
    let interrupt = process::InterruptScope::install();
    ui.info(format!(
        "Listening for Competitive Companion on http://localhost:{port}/"
    ));
    if contest {
        ui.info("Open every problem page and click the green + button · Ctrl-C cancels");
        receive_contest(source, config, ui, &listeners, &interrupt, port, wait)
    } else {
        ui.info("Open a problem page and click the green + button · Ctrl-C cancels");
        receive_single(source, config, ui, &listeners, &interrupt, port, wait)
    }
}

fn receive_single(
    source: &SourceSpec,
    config: &Config,
    ui: &Ui,
    listeners: &[TcpListener],
    interrupt: &process::InterruptScope,
    port: u16,
    wait: Duration,
) -> AppResult<i32> {
    let deadline = Instant::now() + wait;
    let Some((mut stream, problem)) = next_request(listeners, interrupt, deadline, port)? else {
        ui.info("Competitive Companion import cancelled");
        return Ok(130);
    };
    if let Some(batch) = problem.batch.as_ref().filter(|batch| batch.size > 1) {
        let suffix = if batch.id.is_empty() {
            String::new()
        } else {
            format!(" ({})", batch.id)
        };
        let message = format!(
            "contest batch of {} problems{suffix} requires --contest; send a single problem page or run `run-cli companion --contest`",
            batch.size
        );
        let _ = respond(&mut stream, "422 Unprocessable Content", &message);
        return Err(AppError::new(message));
    }
    validate_problem(&mut stream, &problem)?;
    let samples = samples_for(&problem);
    let imported = match cases::import_samples(source, &samples, config) {
        Ok(imported) => imported,
        Err(error) => {
            let _ = respond(&mut stream, "500 Internal Server Error", &error.message);
            return Err(error);
        }
    };
    respond(&mut stream, "200 OK", "Imported by run-cli")?;
    describe_problem(ui, &problem, &imported, None);
    Ok(0)
}

fn receive_contest(
    source: &SourceSpec,
    config: &Config,
    ui: &Ui,
    listeners: &[TcpListener],
    interrupt: &process::InterruptScope,
    port: u16,
    wait: Duration,
) -> AppResult<i32> {
    let deadline = Instant::now() + wait;
    let Some((mut stream, first)) = next_request(listeners, interrupt, deadline, port)? else {
        ui.info("Competitive Companion contest import cancelled");
        return Ok(130);
    };
    let batch = match first.batch.as_ref() {
        Some(batch) if batch.size > 0 && batch.size <= MAX_CONTEST_PROBLEMS => batch,
        Some(batch) => {
            let message = format!(
                "contest batch size {} is invalid; expected between 1 and {MAX_CONTEST_PROBLEMS}",
                batch.size
            );
            let _ = respond(&mut stream, "422 Unprocessable Content", &message);
            return Err(AppError::new(message));
        }
        None => {
            let message = "Competitive Companion payload has no contest batch metadata";
            let _ = respond(&mut stream, "422 Unprocessable Content", message);
            return Err(AppError::new(message));
        }
    };
    validate_problem(&mut stream, &first)?;
    let mut sample_bytes = validate_contest_sample_budget(&mut stream, &first, 0)?;
    let batch_id = batch.id.clone();
    let batch_size = batch.size;
    let mut problems = Vec::with_capacity(batch_size);
    respond(&mut stream, "200 OK", "Queued for run-cli contest import")?;
    problems.push(first);
    ui.info(format!("Received contest problem 1/{batch_size}"));

    while problems.len() < batch_size {
        let position = problems.len() + 1;
        let next = match next_request(listeners, interrupt, deadline, port) {
            Ok(next) => next,
            Err(error) if error.code == 124 => {
                return Err(AppError::with_code(
                    format!(
                        "{}; received {}/{batch_size}; no contest cases were written",
                        error.message,
                        problems.len()
                    ),
                    124,
                ));
            }
            Err(error) => return Err(error),
        };
        let Some((mut stream, problem)) = next else {
            ui.info(format!(
                "Competitive Companion contest import cancelled after {}/{batch_size}; no cases were written",
                problems.len()
            ));
            return Ok(130);
        };
        let compatible = problem
            .batch
            .as_ref()
            .is_some_and(|current| current.size == batch_size && current.id == batch_id);
        if !compatible {
            let current = problem
                .batch
                .as_ref()
                .map(|current| format!("id '{}' size {}", current.id, current.size))
                .unwrap_or_else(|| "missing batch metadata".into());
            let message = format!(
                "contest problem {position}/{batch_size} has incompatible batch metadata ({current}); expected id '{}' size {}",
                batch_id, batch_size
            );
            let _ = respond(&mut stream, "422 Unprocessable Content", &message);
            return Err(AppError::new(message));
        }
        validate_problem(&mut stream, &problem)?;
        sample_bytes = validate_contest_sample_budget(&mut stream, &problem, sample_bytes)?;
        respond(&mut stream, "200 OK", "Queued for run-cli contest import")?;
        ui.info(format!("Received contest problem {position}/{batch_size}"));
        problems.push(problem);
    }

    let mut imported_all = Vec::new();
    let mut imported_problems = Vec::with_capacity(problems.len());
    for (index, problem) in problems.iter().enumerate() {
        let target = contest_source(source, index);
        let samples = samples_for(problem);
        match cases::import_samples(&target, &samples, config) {
            Ok(imported) => {
                imported_all.extend(imported.iter().cloned());
                imported_problems.push((index, target, imported));
            }
            Err(error) => {
                cases::rollback_import(&imported_all);
                return Err(AppError::new(format!(
                    "contest import failed at problem {}/{} ({}): {}; no contest cases were kept",
                    index + 1,
                    batch_size,
                    problem.name,
                    error.message
                )));
            }
        }
    }
    let mut created_sources = Vec::new();
    for (index, target, _) in &imported_problems {
        match create_source_placeholder(target, &problems[*index]) {
            Ok(true) => created_sources.push(target.path.clone()),
            Ok(false) => {}
            Err(error) => {
                cases::rollback_import(&imported_all);
                remove_created_sources(&created_sources);
                return Err(AppError::new(format!(
                    "contest source creation failed for '{}': {}; no contest cases were kept",
                    target.path.display(),
                    error.message
                )));
            }
        }
    }
    for (index, target, imported) in &imported_problems {
        describe_problem(ui, &problems[*index], imported, Some(target));
    }
    ui.success(format!(
        "Imported contest batch '{}' ({batch_size} problem(s), {} sample(s))",
        if batch_id.is_empty() {
            "unnamed"
        } else {
            &batch_id
        },
        imported_all.len()
    ));
    Ok(0)
}

fn next_request(
    listeners: &[TcpListener],
    interrupt: &process::InterruptScope,
    deadline: Instant,
    port: u16,
) -> AppResult<Option<(TcpStream, CompanionProblem)>> {
    loop {
        if interrupt.requested() {
            return Ok(None);
        }
        if Instant::now() >= deadline {
            return Err(AppError::with_code(
                format!("timed out waiting for Competitive Companion on port {port}"),
                124,
            ));
        }
        for listener in listeners {
            match listener.accept() {
                Ok((mut stream, _)) => {
                    stream
                        .set_read_timeout(Some(Duration::from_secs(5)))
                        .map_err(AppError::from)?;
                    stream
                        .set_write_timeout(Some(Duration::from_secs(5)))
                        .map_err(AppError::from)?;
                    let problem = match read_problem(&mut stream) {
                        Ok(problem) => problem,
                        Err(error) => {
                            let _ = respond(&mut stream, error.status, &error.message);
                            return Err(AppError::new(error.message));
                        }
                    };
                    return Ok(Some((stream, problem)));
                }
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => {}
                Err(error) => return Err(AppError::new(format!("listener failed: {error}"))),
            }
        }
        thread::sleep(Duration::from_millis(20));
    }
}

fn validate_problem(stream: &mut TcpStream, problem: &CompanionProblem) -> AppResult<()> {
    if problem.tests.is_empty() {
        let message = format!("'{}' contains no sample tests", problem.name);
        let _ = respond(stream, "422 Unprocessable Content", &message);
        return Err(AppError::new(message));
    }
    Ok(())
}

fn validate_contest_sample_budget(
    stream: &mut TcpStream,
    problem: &CompanionProblem,
    current: usize,
) -> AppResult<usize> {
    let size = problem.tests.iter().fold(0_usize, |total, test| {
        total
            .saturating_add(test.input.len())
            .saturating_add(test.output.len())
    });
    let Some(total) = current.checked_add(size) else {
        let message = "contest sample data is too large; the 64 MiB aggregate limit was exceeded";
        let _ = respond(stream, "413 Content Too Large", message);
        return Err(AppError::new(message));
    };
    if total > MAX_CONTEST_SAMPLE_BYTES {
        let message = format!(
            "contest sample data exceeds the {} MiB aggregate limit",
            MAX_CONTEST_SAMPLE_BYTES / (1024 * 1024)
        );
        let _ = respond(stream, "413 Content Too Large", &message);
        return Err(AppError::new(message));
    }
    Ok(total)
}

fn samples_for(problem: &CompanionProblem) -> Vec<(&[u8], &[u8])> {
    problem
        .tests
        .iter()
        .map(|test| (test.input.as_bytes(), test.output.as_bytes()))
        .collect()
}

fn describe_problem(
    ui: &Ui,
    problem: &CompanionProblem,
    imported: &[cases::ImportedSample],
    target: Option<&SourceSpec>,
) {
    let destination = target
        .map(|target| format!(" for {}", target.path.display()))
        .unwrap_or_default();
    ui.success(format!(
        "Imported {} sample(s) for {}{}",
        imported.len(),
        problem.name,
        destination
    ));
    if !problem.group.is_empty() {
        ui.info(&problem.group);
    }
    if !problem.url.is_empty() {
        ui.info(&problem.url);
    }
    let limits = [
        problem.time_limit.map(|value| format!("{value} ms")),
        problem.memory_limit.map(|value| format!("{value} MiB")),
    ]
    .into_iter()
    .flatten()
    .collect::<Vec<_>>()
    .join(" · ");
    if !limits.is_empty() {
        ui.info(format!("Limits: {limits}"));
    }
    if problem.interactive {
        ui.warning("This problem is interactive; imported samples are non-interactive");
    }
    for sample in imported {
        ui.info(format!(
            "Case #{}: {} · expected {}",
            sample.id,
            sample.input.display(),
            sample.output.display()
        ));
    }
}

fn contest_source(source: &SourceSpec, index: usize) -> SourceSpec {
    if index == 0 {
        return source.clone();
    }
    let stem_name = source
        .stem
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("main");
    let name = if stem_name.chars().count() == 1 && stem_name.as_bytes()[0].is_ascii_alphabetic() {
        let upper = stem_name.as_bytes()[0].is_ascii_uppercase();
        let mut label = contest_label(index);
        if !upper {
            label.make_ascii_lowercase();
        }
        label
    } else {
        format!("{stem_name}-{}", index + 1)
    };
    let parent = source
        .stem
        .parent()
        .unwrap_or_else(|| std::path::Path::new(""));
    let stem = parent.join(&name);
    let extension = source
        .path
        .extension()
        .and_then(|extension| extension.to_str())
        .unwrap_or(source.language.extension());
    let path = stem.with_extension(extension);
    SourceSpec {
        requested: path.clone(),
        path,
        stem,
        language: source.language,
    }
}

fn contest_label(mut index: usize) -> String {
    let mut label = String::new();
    loop {
        label.push((b'A' + (index % 26) as u8) as char);
        if index < 26 {
            break;
        }
        index = index / 26 - 1;
    }
    label.chars().rev().collect()
}

fn create_source_placeholder(source: &SourceSpec, problem: &CompanionProblem) -> AppResult<bool> {
    if source.path.exists() {
        if source.path.is_file() {
            return Ok(false);
        }
        return Err(AppError::new(format!(
            "contest source path '{}' exists but is not a file",
            source.path.display()
        )));
    }
    if let Some(parent) = source
        .path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        fs::create_dir_all(parent)?;
    }
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&source.path)
        .map_err(|error| {
            AppError::new(format!(
                "cannot create contest source '{}': {error}",
                source.path.display()
            ))
        })?;
    let name = problem.name.replace(['\r', '\n'], " ");
    let contents = if source.language.extension() == "py" {
        format!("# Caseflow contest source placeholder\n# Problem: {name}\n")
    } else {
        format!("// Caseflow contest source placeholder\n// Problem: {name}\n")
    };
    if let Err(error) = file
        .write_all(contents.as_bytes())
        .and_then(|_| file.sync_all())
    {
        let _ = fs::remove_file(&source.path);
        return Err(AppError::new(format!(
            "cannot initialize contest source '{}': {error}",
            source.path.display()
        )));
    }
    Ok(true)
}

fn remove_created_sources(paths: &[PathBuf]) {
    for path in paths {
        let _ = fs::remove_file(path);
    }
}

fn bind_loopback(port: u16) -> AppResult<Vec<TcpListener>> {
    let ipv4 =
        TcpListener::bind(SocketAddr::from((Ipv4Addr::LOCALHOST, port))).map_err(|error| {
            AppError::new(format!(
                "cannot listen for Competitive Companion on 127.0.0.1:{port}: {error}"
            ))
        })?;
    ipv4.set_nonblocking(true)?;
    let mut listeners = vec![ipv4];
    if let Ok(ipv6) = TcpListener::bind(SocketAddr::from((Ipv6Addr::LOCALHOST, port))) {
        ipv6.set_nonblocking(true)?;
        listeners.push(ipv6);
    }
    Ok(listeners)
}

fn read_problem(stream: &mut TcpStream) -> Result<CompanionProblem, HttpError> {
    let mut request = Vec::new();
    let mut chunk = [0_u8; 4096];
    let header_end = loop {
        let count = stream.read(&mut chunk).map_err(|error| HttpError {
            status: "400 Bad Request",
            message: format!("could not read Competitive Companion request: {error}"),
        })?;
        if count == 0 {
            return Err(HttpError {
                status: "400 Bad Request",
                message: "Competitive Companion request ended before its headers".into(),
            });
        }
        request.extend_from_slice(&chunk[..count]);
        if let Some(end) = find_header_end(&request) {
            if end > MAX_HEADER_BYTES {
                return Err(HttpError {
                    status: "431 Request Header Fields Too Large",
                    message: "Competitive Companion request headers are too large".into(),
                });
            }
            break end;
        }
        if request.len() > MAX_HEADER_BYTES {
            return Err(HttpError {
                status: "431 Request Header Fields Too Large",
                message: "Competitive Companion request headers are too large".into(),
            });
        }
    };
    let headers = std::str::from_utf8(&request[..header_end]).map_err(|_| HttpError {
        status: "400 Bad Request",
        message: "Competitive Companion request headers are not valid UTF-8".into(),
    })?;
    let mut lines = headers.split("\r\n");
    let request_line = lines.next().unwrap_or_default();
    let mut request_parts = request_line.split_whitespace();
    if request_parts.next() != Some("POST") || request_parts.next() != Some("/") {
        return Err(HttpError {
            status: "405 Method Not Allowed",
            message: "expected a POST request to /".into(),
        });
    }
    let content_length = lines
        .find_map(|line| {
            let (name, value) = line.split_once(':')?;
            name.eq_ignore_ascii_case("content-length")
                .then(|| value.trim())
        })
        .ok_or_else(|| HttpError {
            status: "411 Length Required",
            message: "Competitive Companion request has no Content-Length".into(),
        })?
        .parse::<usize>()
        .map_err(|_| HttpError {
            status: "400 Bad Request",
            message: "Competitive Companion Content-Length is invalid".into(),
        })?;
    if content_length > MAX_BODY_BYTES {
        return Err(HttpError {
            status: "413 Content Too Large",
            message: format!(
                "Competitive Companion payload exceeds the {} MiB limit",
                MAX_BODY_BYTES / (1024 * 1024)
            ),
        });
    }
    let body_start = header_end + 4;
    while request.len().saturating_sub(body_start) < content_length {
        let count = stream.read(&mut chunk).map_err(|error| HttpError {
            status: "400 Bad Request",
            message: format!("could not read Competitive Companion payload: {error}"),
        })?;
        if count == 0 {
            return Err(HttpError {
                status: "400 Bad Request",
                message: "Competitive Companion payload ended early".into(),
            });
        }
        request.extend_from_slice(&chunk[..count]);
    }
    serde_json::from_slice(&request[body_start..body_start + content_length]).map_err(|error| {
        HttpError {
            status: "400 Bad Request",
            message: format!("invalid Competitive Companion JSON: {error}"),
        }
    })
}

fn find_header_end(request: &[u8]) -> Option<usize> {
    request.windows(4).position(|window| window == b"\r\n\r\n")
}

fn respond(stream: &mut TcpStream, status: &str, message: &str) -> AppResult<()> {
    let body = message.as_bytes();
    write!(
        stream,
        "HTTP/1.1 {status}\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Length: {}\r\nAccess-Control-Allow-Origin: *\r\nConnection: close\r\n\r\n",
        body.len()
    )?;
    stream.write_all(body)?;
    stream.flush()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::source::resolve_source;
    use std::path::PathBuf;

    #[test]
    fn contest_sources_follow_problem_labels_for_letter_stems() {
        let source = resolve_source("round/A.cpp").unwrap();
        assert_eq!(
            contest_source(&source, 0).path,
            PathBuf::from("round/A.cpp")
        );
        assert_eq!(
            contest_source(&source, 1).path,
            PathBuf::from("round/B.cpp")
        );
        assert_eq!(
            contest_source(&source, 25).path,
            PathBuf::from("round/Z.cpp")
        );
        assert_eq!(
            contest_source(&source, 26).path,
            PathBuf::from("round/AA.cpp")
        );
    }

    #[test]
    fn contest_sources_add_an_index_for_non_letter_stems() {
        let source = resolve_source("round/main.py").unwrap();
        assert_eq!(
            contest_source(&source, 1).path,
            PathBuf::from("round/main-2.py")
        );
        assert_eq!(
            contest_source(&source, 2).path,
            PathBuf::from("round/main-3.py")
        );
    }

    #[test]
    fn contest_sources_keep_relative_paths_clean() {
        let source = resolve_source("A.cpp").unwrap();
        assert_eq!(contest_source(&source, 1).path, PathBuf::from("B.cpp"));
    }
}
