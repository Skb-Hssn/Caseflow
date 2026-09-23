use crate::config::ColorPolicy;
use crate::model::RunReport;
use crate::terminal;
use crate::theme;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

#[derive(Debug, Clone)]
pub struct Ui {
    color: bool,
    interactive: bool,
}

impl Ui {
    pub fn new(policy: ColorPolicy) -> Self {
        let interactive = terminal::stderr_is_terminal();
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

    pub fn color_enabled(&self) -> bool {
        self.color
    }

    pub fn header(&self, title: &str, detail: impl AsRef<str>) {
        if !self.interactive {
            return;
        }
        let detail = detail.as_ref();
        let color = if self.color {
            theme::ANSI_PRIMARY_BOLD
        } else {
            ""
        };
        let reset = if self.color { theme::ANSI_RESET } else { "" };
        let width = terminal_width().min(110);
        let label = if detail.is_empty() {
            format!("━━ {title} ")
        } else {
            format!("━━ {title} · {detail} ")
        };
        let mut line = truncate_width(&label, width);
        line.push_str(&"━".repeat(width.saturating_sub(UnicodeWidthStr::width(line.as_str()))));
        eprintln!("{color}{line}{reset}");
    }

    pub fn info(&self, message: impl AsRef<str>) {
        eprintln!("  • {}", message.as_ref());
    }

    pub fn success(&self, message: impl AsRef<str>) {
        if self.color {
            eprintln!(
                "  {}✓ {}{}",
                theme::ANSI_SUCCESS_BOLD,
                message.as_ref(),
                theme::ANSI_RESET
            );
        } else {
            eprintln!("  ✓ {}", message.as_ref());
        }
    }

    pub fn warning(&self, message: impl AsRef<str>) {
        if self.color {
            eprintln!(
                "  {}! {}{}",
                theme::ANSI_WARNING,
                message.as_ref(),
                theme::ANSI_RESET
            );
        } else {
            eprintln!("  ! {}", message.as_ref());
        }
    }

    pub fn failure(&self, message: impl AsRef<str>) {
        if self.color {
            eprintln!(
                "  {}✗ {}{}",
                theme::ANSI_DANGER_BOLD,
                message.as_ref(),
                theme::ANSI_RESET
            );
        } else {
            eprintln!("  ✗ {}", message.as_ref());
        }
    }

    pub fn error(&self, message: impl AsRef<str>) {
        if self.color {
            eprintln!(
                "{}✗ Error{}  {}",
                theme::ANSI_DANGER_BOLD,
                theme::ANSI_RESET,
                message.as_ref()
            );
        } else {
            eprintln!("✗ Error  {}", message.as_ref());
        }
    }

    pub fn field(&self, label: &str, value: impl AsRef<str>) {
        let label = format!("{label:<10}");
        if self.color {
            eprintln!("  \x1b[2m{label}\x1b[0m{}", value.as_ref());
        } else {
            eprintln!("  {label}{}", value.as_ref());
        }
    }

    pub fn section_title(&self, title: &str) {
        if self.color {
            let color = if matches!(title, "Input" | "Output" | "Expected Output") {
                theme::ANSI_ACCENT_ORANGE_BOLD
            } else {
                theme::ANSI_PRIMARY_BOLD
            };
            eprintln!("{color}{title}{}", theme::ANSI_RESET);
        } else {
            eprintln!("{title}");
        }
    }

    pub fn case_report(&self, report: &RunReport) {
        if self.interactive {
            let width = terminal_width().min(110);
            if self.color {
                eprintln!("\x1b[38;5;103;2m{}{}", "─".repeat(width), theme::ANSI_RESET);
            } else {
                eprintln!("{}", "─".repeat(width));
            }
            let summary = report_summary(report);
            if self.color {
                let (result_color, marker) = if report.timed_out || report.exit_code != 0 {
                    (theme::ANSI_DANGER_BOLD, '✗')
                } else {
                    (theme::ANSI_SUCCESS_BOLD, '✓')
                };
                let result = summary
                    .strip_prefix(&format!("{marker} "))
                    .unwrap_or(&summary);
                let (status, metrics) = result
                    .split_once("  ·  ")
                    .map_or((result, ""), |(status, metrics)| (status, metrics));
                eprint!(
                    "\x1b[38;5;103mSystem status  ·  {result_color}{marker} {status}{}",
                    theme::ANSI_RESET
                );
                if !metrics.is_empty() {
                    eprint!("\x1b[38;5;103m  ·  {metrics}{}", theme::ANSI_RESET);
                }
                eprintln!();
            } else {
                eprintln!("System status  ·  {summary}");
            }
        } else {
            self.report(report);
        }
        if self.interactive {
            eprintln!();
            eprintln!();
        }
    }

    pub fn report(&self, report: &RunReport) {
        eprintln!("  {}", report_summary(report));
    }

    pub fn case_summary(&self, passed: &[u64], failed: &[u64], unjudged: &[u64]) {
        let judged = passed.len() + failed.len();
        let prefix = format!("Test summary  ·  {}/{} passed", passed.len(), judged);
        let mut badges = passed
            .iter()
            .map(|id| (*id, theme::ANSI_SUCCESS))
            .chain(failed.iter().map(|id| (*id, theme::ANSI_DANGER_BOLD)))
            .chain(unjudged.iter().map(|id| (*id, "\x1b[38;5;103;2m")))
            .collect::<Vec<_>>();
        badges.sort_unstable_by_key(|(id, _)| *id);
        if self.color {
            eprint!("\x1b[1m{prefix}{}  ·  ", theme::ANSI_RESET);
            for (index, (id, color)) in badges.iter().enumerate() {
                if index > 0 {
                    eprint!(" ");
                }
                eprint!("{color}[{id}]{}", theme::ANSI_RESET);
            }
            eprintln!();
        } else {
            let badges = badges
                .iter()
                .map(|(id, _)| format!("[{id}]"))
                .collect::<Vec<_>>()
                .join(" ");
            eprintln!("{prefix}  ·  {badges}");
        }
    }
}

fn report_summary(report: &RunReport) -> String {
    let status = if report.timed_out {
        "Timed out".to_string()
    } else if report.exit_code == 0 {
        "Success (exit 0)".to_string()
    } else {
        format!("Failed (exit {})", report.exit_code)
    };
    let marker = if report.timed_out || report.exit_code != 0 {
        "✗"
    } else {
        "✓"
    };
    format!(
        "{marker} {status}  ·  {:.3}s  ·  {}",
        report.wall_time.as_secs_f64(),
        format_memory(report.peak_memory_kib)
    )
}

fn format_memory(kib: i64) -> String {
    if kib < 0 {
        "memory ?".into()
    } else if kib >= 1_048_576 {
        format!("{:.1} GiB", kib as f64 / 1_048_576.0)
    } else if kib >= 1024 {
        format!("{:.1} MiB", kib as f64 / 1024.0)
    } else {
        format!("{kib} KiB")
    }
}

fn terminal_width() -> usize {
    crossterm::terminal::size()
        .map(|(columns, _)| columns.max(1) as usize)
        .unwrap_or(72)
}

fn truncate_width(value: &str, width: usize) -> String {
    if UnicodeWidthStr::width(value) <= width {
        return value.to_string();
    }
    let target = width.saturating_sub(1);
    let mut used = 0;
    let mut result = String::new();
    for character in value.chars() {
        let character_width = character.width().unwrap_or(0);
        if used + character_width > target {
            break;
        }
        result.push(character);
        used += character_width;
    }
    result.push('…');
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn formats_memory_units() {
        assert_eq!(format_memory(512), "512 KiB");
        assert_eq!(format_memory(1536), "1.5 MiB");
    }

    #[test]
    fn report_summary_is_compact_and_single_line() {
        let report = RunReport {
            exit_code: 0,
            interrupted: false,
            wall_time: Duration::from_millis(10),
            peak_memory_kib: 4096,
            timed_out: false,
        };
        let summary = report_summary(&report);
        assert!(summary.starts_with("✓ Success (exit 0)  ·  0.010s"));
        assert!(summary.ends_with("4.0 MiB"));
        assert!(!summary.contains("CPU"));
        assert!(!summary.contains('\n'));
    }
}
