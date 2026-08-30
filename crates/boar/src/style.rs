// BOAR terminal styling helpers backed by the local form3 (3form) crate.
//
// All helpers respect NO_COLOR / TERM=dumb via form3::term::TermInfo and
// downgrade to plain text when stdout/stderr is not a TTY.

use form3::compat::Colorize;
use form3::term::{ColorSupport, TermInfo};
use std::io::{IsTerminal, stderr, stdout};

/// Color support appropriate for stdout.
pub fn stdout_support() -> ColorSupport {
    if stdout().is_terminal() {
        TermInfo::detect().color_support
    } else {
        ColorSupport::NoColor
    }
}

/// Color support appropriate for stderr.
pub fn stderr_support() -> ColorSupport {
    if stderr().is_terminal() {
        TermInfo::detect().color_support
    } else {
        ColorSupport::NoColor
    }
}

pub fn title(text: impl Into<String>, support: ColorSupport) -> String {
    text.into()
        .cyan()
        .bold()
        .with_color_support(support)
        .to_string()
}

pub fn heading(text: impl Into<String>, support: ColorSupport) -> String {
    text.into().bold().with_color_support(support).to_string()
}

pub fn label(text: impl Into<String>, support: ColorSupport) -> String {
    text.into()
        .cyan()
        .bold()
        .with_color_support(support)
        .to_string()
}

pub fn command(text: impl Into<String>, support: ColorSupport) -> String {
    text.into().yellow().with_color_support(support).to_string()
}

pub fn option(text: impl Into<String>, support: ColorSupport) -> String {
    text.into().green().with_color_support(support).to_string()
}

pub fn path(text: impl Into<String>, support: ColorSupport) -> String {
    text.into().blue().with_color_support(support).to_string()
}

pub fn value(text: impl Into<String>, support: ColorSupport) -> String {
    text.into()
        .bright_white()
        .bold()
        .with_color_support(support)
        .to_string()
}

pub fn dim(text: impl Into<String>, support: ColorSupport) -> String {
    text.into().dimmed().with_color_support(support).to_string()
}

pub fn error(text: impl Into<String>, support: ColorSupport) -> String {
    text.into()
        .red()
        .bold()
        .with_color_support(support)
        .to_string()
}

pub fn warn(text: impl Into<String>, support: ColorSupport) -> String {
    text.into()
        .yellow()
        .bold()
        .with_color_support(support)
        .to_string()
}

pub fn ok(text: impl Into<String>, support: ColorSupport) -> String {
    text.into()
        .green()
        .bold()
        .with_color_support(support)
        .to_string()
}
