use crate::config::ColorPolicy;
use crate::model::{BuildMode, RunReport, SourceSpec};
use crate::theme;
use std::io::{self, IsTerminal};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

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
        let width = terminal_width().min(88);
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
                theme::ANSI_SUCCESS,
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
        self.header("RESULT", "");
        self.field("Status", status);
        self.field("Wall", format!("{:.3} s", report.wall_time.as_secs_f64()));
        self.field(
            "CPU",
            format!(
                "{:.3} s user · {:.3} s system · {:.0}%",
                report.user_time.as_secs_f64(),
                report.system_time.as_secs_f64(),
                cpu
            ),
        );
        self.field("Memory", format_memory(report.peak_memory_kib));
    }

    pub fn welcome(&self, source: &SourceSpec, mode: BuildMode, mouse: bool) {
        let width = terminal_width().min(64);
        if width < 28 {
            eprintln!("run-cli v{}", env!("CARGO_PKG_VERSION"));
            eprintln!(
                "{}",
                truncate_width(&source.path.display().to_string(), width)
            );
            eprintln!(
                "{} · {} · mouse:{}",
                source.language,
                mode,
                if mouse { "on" } else { "off" }
            );
            eprintln!("/help · Tab complete · Ctrl-D exit\n");
            return;
        }
        print_welcome_top(width, self.color);
        print_box_row(&source.path.display().to_string(), width);
        print_box_row(
            &format!(
                "{}  ·  {}  ·  mouse:{}",
                source.language,
                mode,
                if mouse { "on" } else { "off" }
            ),
            width,
        );
        let line = "─".repeat(width - 2);
        eprintln!("╰{line}╯");
        if self.color {
            eprintln!("  \x1b[2m/help commands  ·  Tab complete  ·  Ctrl-D exit\x1b[0m\n");
        } else {
            eprintln!("  /help commands  ·  Tab complete  ·  Ctrl-D exit\n");
        }
    }
}

fn print_welcome_top(width: usize, color: bool) {
    let (start, title, fill, end) = welcome_top_parts(width);
    if color {
        eprintln!(
            "{start}{}{title}{}{fill}{end}",
            theme::ANSI_PRIMARY_BOLD,
            theme::ANSI_RESET
        );
    } else {
        eprintln!("{start}{title}{fill}{end}");
    }
}

fn welcome_top_parts(width: usize) -> (&'static str, String, String, &'static str) {
    let title = format!(" run-cli v{} ", env!("CARGO_PKG_VERSION"));
    let fill = "─".repeat(width.saturating_sub(UnicodeWidthStr::width(title.as_str()) + 3));
    ("╭─", title, fill, "╮")
}

fn terminal_width() -> usize {
    crossterm::terminal::size()
        .map(|(columns, _)| columns.max(1) as usize)
        .unwrap_or(72)
}

fn print_box_row(value: &str, width: usize) {
    let content_width = width.saturating_sub(4);
    let value = truncate_width(value, content_width);
    let padding = content_width.saturating_sub(UnicodeWidthStr::width(value.as_str()));
    eprintln!("│ {value}{} │", " ".repeat(padding));
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

    #[test]
    fn welcome_border_matches_requested_width() {
        let (start, title, fill, end) = welcome_top_parts(48);
        let line = format!("{start}{title}{fill}{end}");
        assert_eq!(UnicodeWidthStr::width(line.as_str()), 48);
    }
}
