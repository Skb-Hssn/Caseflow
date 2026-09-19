use crate::config::ColorPolicy;
use crate::model::{BuildMode, RunReport, SourceSpec};
use std::io::{self, IsTerminal};

#[derive(Debug, Clone)]
pub struct Ui {
    color: bool,
    interactive: bool,
}

impl Ui {
    pub fn new(policy: ColorPolicy) -> Self {
        let interactive = io::stderr().is_terminal();
        let color = match policy {
            ColorPolicy::Always => true,
            ColorPolicy::Never => false,
            ColorPolicy::Auto => interactive,
        };
        Self { color, interactive }
    }

    pub fn interactive(&self) -> bool {
        self.interactive
    }

    pub fn header(&self, title: &str, detail: impl AsRef<str>) {
        if !self.interactive {
            return;
        }
        let detail = detail.as_ref();
        let color = if self.color { "\x1b[36;1m" } else { "" };
        let reset = if self.color { "\x1b[0m" } else { "" };
        if detail.is_empty() {
            eprintln!("{color}━━ {title} ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━{reset}");
        } else {
            eprintln!("{color}━━ {title} · {detail} ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━{reset}");
        }
    }

    pub fn info(&self, message: impl AsRef<str>) {
        eprintln!("  {}", message.as_ref());
    }

    pub fn success(&self, message: impl AsRef<str>) {
        if self.color {
            eprintln!("  \x1b[32m✓ {}\x1b[0m", message.as_ref());
        } else {
            eprintln!("  ✓ {}", message.as_ref());
        }
    }

    pub fn warning(&self, message: impl AsRef<str>) {
        if self.color {
            eprintln!("  \x1b[33m{}\x1b[0m", message.as_ref());
        } else {
            eprintln!("  {}", message.as_ref());
        }
    }

    pub fn error(&self, message: impl AsRef<str>) {
        if self.color {
            eprintln!("\x1b[31;1mError:\x1b[0m {}", message.as_ref());
        } else {
            eprintln!("Error: {}", message.as_ref());
        }
    }

    pub fn report(&self, report: &RunReport) {
        let cpu_seconds = report.user_time.as_secs_f64() + report.system_time.as_secs_f64();
        let cpu = if report.wall_time.is_zero() {
            0.0
        } else {
            cpu_seconds / report.wall_time.as_secs_f64() * 100.0
        };
        let status = if report.timed_out {
            "Timed out".to_string()
        } else if report.exit_code == 0 {
            "Success (exit 0)".to_string()
        } else {
            format!("Failed (exit {})", report.exit_code)
        };
        self.header("RESOURCE USAGE", "");
        eprintln!("  Status       {status}");
        eprintln!("  Wall time    {:.3} s", report.wall_time.as_secs_f64());
        eprintln!(
            "  CPU time     {:.3} s user · {:.3} s system · {:.0}%",
            report.user_time.as_secs_f64(),
            report.system_time.as_secs_f64(),
            cpu
        );
        eprintln!("  Peak memory  {}", format_memory(report.peak_memory_kib));
    }

    pub fn welcome(&self, source: &SourceSpec, mode: BuildMode, mouse: bool) {
        let width = 64;
        let line = "─".repeat(width - 2);
        eprintln!("╭{line}╮");
        eprintln!("│  run-cli {:<52}│", env!("CARGO_PKG_VERSION"));
        eprintln!(
            "│  source: {:<52}│",
            truncate(&source.path.display().to_string(), 52)
        );
        eprintln!("│  language: {:<50}│", source.language.to_string());
        eprintln!("│  mode: {:<54}│", mode.to_string());
        eprintln!("│  mouse: {:<53}│", if mouse { "on" } else { "off" });
        eprintln!("╰{line}╯");
        eprintln!("  Type /help for commands. File arguments support Tab completion.\n");
    }
}

fn truncate(value: &str, width: usize) -> String {
    let count = value.chars().count();
    if count <= width {
        return value.to_string();
    }
    let mut result = value
        .chars()
        .take(width.saturating_sub(1))
        .collect::<String>();
    result.push('…');
    result
}

pub fn format_memory(kib: i64) -> String {
    if kib < 0 {
        return "?".into();
    }
    if kib >= 1_048_576 {
        format!("{:.1} GiB ({} KiB)", kib as f64 / 1_048_576.0, kib)
    } else if kib >= 1024 {
        format!("{:.1} MiB ({} KiB)", kib as f64 / 1024.0, kib)
    } else {
        format!("{kib} KiB")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_memory_units() {
        assert_eq!(format_memory(512), "512 KiB");
        assert_eq!(format_memory(1536), "1.5 MiB (1536 KiB)");
    }
}
