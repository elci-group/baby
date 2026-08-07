//! Structured error types for `baby`.
//!
//! All library code returns [`BabyError`] via the [`Result`] alias. Binaries
//! convert it to a log message and a non-zero exit code at the `main()` edge.

use std::fmt;
use std::io;

/// Classification of every error the library can produce.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorKind {
    /// An I/O operation failed.
    Io,
    /// An external command returned a non-zero status.
    CommandFailed,
    /// A configuration file could not be parsed.
    ConfigParse,
    /// The project name could not be inferred from the current directory.
    ProjectNameInference,
    /// The `HOME` environment variable is required but missing.
    HomeNotSet,
    /// No PID file was found.
    PidNotFound,
    /// The process referenced by a PID file is not alive.
    ProcessNotAlive,
    /// The daemon encountered a fatal problem.
    Daemon,
    /// Filesystem watching failed.
    Watch,
    /// Man-page generation failed.
    ManPage,
}

impl fmt::Display for ErrorKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ErrorKind::Io => write!(f, "I/O error"),
            ErrorKind::CommandFailed => write!(f, "command failed"),
            ErrorKind::ConfigParse => write!(f, "config parse error"),
            ErrorKind::ProjectNameInference => write!(f, "project name inference error"),
            ErrorKind::HomeNotSet => write!(f, "HOME not set"),
            ErrorKind::PidNotFound => write!(f, "PID file not found"),
            ErrorKind::ProcessNotAlive => write!(f, "process not alive"),
            ErrorKind::Daemon => write!(f, "daemon error"),
            ErrorKind::Watch => write!(f, "watch error"),
            ErrorKind::ManPage => write!(f, "man page error"),
        }
    }
}

/// The error type used throughout the `baby` crate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BabyError {
    kind: ErrorKind,
    message: String,
}

impl BabyError {
    /// Create a new error with the given classification and message.
    pub fn new(kind: ErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
        }
    }

    /// I/O error with human-readable context.
    pub fn io(context: impl Into<String>, source: io::Error) -> Self {
        Self {
            kind: ErrorKind::Io,
            message: format!("{}: {}", context.into(), source),
        }
    }

    /// External command exited unsuccessfully.
    pub fn command_failed(command: impl Into<String>, status: Option<i32>) -> Self {
        let msg = match status {
            Some(code) => format!("{} exited with status {}", command.into(), code),
            None => format!("{} was terminated", command.into()),
        };
        Self {
            kind: ErrorKind::CommandFailed,
            message: msg,
        }
    }

    /// Configuration file parse error.
    pub fn config_parse(path: impl Into<String>, source: toml::de::Error) -> Self {
        Self {
            kind: ErrorKind::ConfigParse,
            message: format!("failed to parse {}: {}", path.into(), source),
        }
    }

    /// Project name could not be inferred.
    pub fn project_name() -> Self {
        Self {
            kind: ErrorKind::ProjectNameInference,
            message: "cannot infer project name from current directory".into(),
        }
    }

    /// HOME environment variable is missing.
    pub fn home_not_set() -> Self {
        Self {
            kind: ErrorKind::HomeNotSet,
            message: "HOME environment variable is not set".into(),
        }
    }

    /// PID file is missing.
    pub fn pid_not_found() -> Self {
        Self {
            kind: ErrorKind::PidNotFound,
            message: "birthd is not running".into(),
        }
    }

    /// Process referenced by PID is not alive.
    pub fn process_not_alive(pid: u32) -> Self {
        Self {
            kind: ErrorKind::ProcessNotAlive,
            message: format!("process {} is not alive", pid),
        }
    }

    /// Daemon error.
    pub fn daemon(message: impl Into<String>) -> Self {
        Self {
            kind: ErrorKind::Daemon,
            message: message.into(),
        }
    }

    /// Filesystem watcher error.
    pub fn watch(source: notify::Error) -> Self {
        Self {
            kind: ErrorKind::Watch,
            message: source.to_string(),
        }
    }

    /// Man-page generation error.
    pub fn man_page(source: clap_mangen::Error) -> Self {
        Self {
            kind: ErrorKind::ManPage,
            message: source.to_string(),
        }
    }

    /// Return the error classification.
    pub fn kind(&self) -> ErrorKind {
        self.kind
    }

    /// Return the detailed message.
    pub fn message(&self) -> &str {
        &self.message
    }
}

impl fmt::Display for BabyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for BabyError {}

/// Convenient alias for results returned by `baby`.
pub type Result<T> = std::result::Result<T, BabyError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_kind_display() {
        assert_eq!(ErrorKind::Io.to_string(), "I/O error");
    }

    #[test]
    fn project_name_error() {
        let err = BabyError::project_name();
        assert_eq!(err.kind(), ErrorKind::ProjectNameInference);
        assert!(err.message().contains("project name"));
    }

    #[test]
    fn command_failed_with_code() {
        let err = BabyError::command_failed("cargo build", Some(101));
        assert_eq!(err.kind(), ErrorKind::CommandFailed);
        assert!(err.message().contains("status 101"));
    }

    #[test]
    fn command_failed_without_code() {
        let err = BabyError::command_failed("cargo build", None);
        assert!(err.message().contains("terminated"));
    }
}
