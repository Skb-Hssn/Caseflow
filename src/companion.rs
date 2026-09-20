use crate::cases;
use crate::config::Config;
use crate::error::{AppError, AppResult};
use crate::model::SourceSpec;
use crate::process;
use crate::ui::Ui;
use serde::Deserialize;
use std::io::{self, Read, Write};
use std::net::{Ipv4Addr, Ipv6Addr, SocketAddr, TcpListener, TcpStream};
use std::thread;
use std::time::{Duration, Instant};

const MAX_HEADER_BYTES: usize = 64 * 1024;
const MAX_BODY_BYTES: usize = 8 * 1024 * 1024;

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
) -> AppResult<i32> {
    let listeners = bind_loopback(port)?;
    let interrupt = process::InterruptScope::install();
    ui.info(format!(
        "Listening for Competitive Companion on http://localhost:{port}/"
    ));
    ui.info("Open a problem page and click the green + button · Ctrl-C cancels");
    let deadline = Instant::now() + wait;

    loop {
        if interrupt.requested() {
            ui.info("Competitive Companion import cancelled");
            return Ok(130);
        }
        if Instant::now() >= deadline {
            return Err(AppError::with_code(
                format!("timed out waiting for Competitive Companion on port {port}"),
                124,
            ));
        }
        for listener in &listeners {
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
                    if problem.batch.as_ref().is_some_and(|batch| batch.size > 1) {
                        let batch = problem.batch.as_ref().unwrap();
                        let suffix = if batch.id.is_empty() {
                            String::new()
                        } else {
                            format!(" ({})", batch.id)
                        };
                        let message = format!(
                            "contest batch of {} problems{suffix} is not supported for one active source; send a single problem page",
                            batch.size
                        );
                        let _ = respond(&mut stream, "422 Unprocessable Content", &message);
                        return Err(AppError::new(message));
                    }
                    if problem.tests.is_empty() {
                        let message = format!("'{}' contains no sample tests", problem.name);
                        let _ = respond(&mut stream, "422 Unprocessable Content", &message);
                        return Err(AppError::new(message));
                    }
                    let samples = problem
                        .tests
                        .iter()
                        .map(|test| (test.input.as_bytes(), test.output.as_bytes()))
                        .collect::<Vec<_>>();
                    let imported = match cases::import_samples(source, &samples, config) {
                        Ok(imported) => imported,
                        Err(error) => {
                            let _ =
                                respond(&mut stream, "500 Internal Server Error", &error.message);
                            return Err(error);
                        }
                    };
                    respond(&mut stream, "200 OK", "Imported by run-cli")?;
                    ui.success(format!(
                        "Imported {} sample(s) for {}",
                        imported.len(),
                        problem.name
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
                        ui.warning(
                            "This problem is interactive; imported samples are non-interactive",
                        );
                    }
                    for sample in imported {
                        ui.info(format!(
                            "Case #{}: {} · expected {}",
                            sample.id,
                            sample.input.display(),
                            sample.output.display()
                        ));
                    }
                    return Ok(0);
                }
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => {}
                Err(error) => return Err(AppError::new(format!("listener failed: {error}"))),
            }
        }
        thread::sleep(Duration::from_millis(20));
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
