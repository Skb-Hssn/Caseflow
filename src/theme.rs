use crossterm::style::Color;

// A restrained slate-and-blue palette. Semantic colors are reserved for
// success, warning, and error states instead of decorating every control.
pub const PRIMARY: Color = Color::AnsiValue(75);
pub const PRIMARY_SOFT: Color = Color::AnsiValue(110);
pub const MUTED: Color = Color::AnsiValue(245);
pub const SURFACE: Color = Color::AnsiValue(238);
pub const ON_ACCENT: Color = Color::AnsiValue(231);
pub const SUCCESS: Color = Color::AnsiValue(78);
pub const WARNING: Color = Color::AnsiValue(180);
pub const DANGER: Color = Color::AnsiValue(203);

pub const ANSI_PRIMARY_BOLD: &str = "\x1b[38;5;75;1m";
pub const ANSI_SUCCESS: &str = "\x1b[38;5;78m";
pub const ANSI_WARNING: &str = "\x1b[38;5;180m";
pub const ANSI_DANGER_BOLD: &str = "\x1b[38;5;203;1m";
pub const ANSI_RESET: &str = "\x1b[0m";
