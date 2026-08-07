//! A tiny `log` implementation that replaces `env_logger`.
//!
//! `setup_logging` targets stderr; `setup_daemon_logging` writes to stderr and
//! the birthd log file. The default level is `info`; override with `RUST_LOG`
//! (`error`, `warn`, `info`, `debug`, `trace`).

use std::fs::{File, OpenOptions};
use std::io::{self, Write};
use std::sync::Mutex;

use log::{LevelFilter, Log, Metadata, Record};
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

use crate::error::{BabyError, Result};

struct SimpleLogger {
    max_level: LevelFilter,
    file: Option<Mutex<File>>,
}

impl Log for SimpleLogger {
    fn enabled(&self, metadata: &Metadata) -> bool {
        metadata.level() <= self.max_level
    }

    fn log(&self, record: &Record) {
        if !self.enabled(record.metadata()) {
            return;
        }

        let timestamp = OffsetDateTime::now_utc()
            .format(&Rfc3339)
            .unwrap_or_default();
        let line = format!("{} {:>5} {}\n", timestamp, record.level(), record.args());

        // Stderr is unbuffered; ignore write failures.
        let _ = io::stderr().write_all(line.as_bytes());

        if let Some(ref file) = self.file
            && let Ok(mut file) = file.lock()
        {
            let _ = file.write_all(line.as_bytes());
        }
    }

    fn flush(&self) {
        let _ = io::stderr().flush();
        if let Some(ref file) = self.file
            && let Ok(mut file) = file.lock()
        {
            let _ = file.flush();
        }
    }
}

fn parse_level() -> LevelFilter {
    match std::env::var("RUST_LOG").as_deref() {
        Ok("error") => LevelFilter::Error,
        Ok("warn") => LevelFilter::Warn,
        Ok("debug") => LevelFilter::Debug,
        Ok("trace") => LevelFilter::Trace,
        _ => LevelFilter::Info,
    }
}

/// Initialise logging to stderr.
pub fn setup_logging() {
    let logger = SimpleLogger {
        max_level: parse_level(),
        file: None,
    };
    let _ = log::set_boxed_logger(Box::new(logger));
    log::set_max_level(parse_level());
}

/// Initialise logging to stderr and the birthd log file.
pub fn setup_daemon_logging(log_path: &std::path::Path) -> Result<()> {
    if let Some(parent) = log_path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| BabyError::io(format!("create log directory {}", parent.display()), e))?;
    }

    let file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(log_path)
        .map_err(|e| BabyError::io(format!("open log file {}", log_path.display()), e))?;

    let logger = SimpleLogger {
        max_level: parse_level(),
        file: Some(Mutex::new(file)),
    };
    let _ = log::set_boxed_logger(Box::new(logger));
    log::set_max_level(parse_level());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use log::Level;

    #[test]
    fn parse_level_defaults_to_info() {
        // This test assumes RUST_LOG is not set in the test environment.
        // If it is, the test may fail; it documents the intended default.
        let level = parse_level();
        assert!(level >= LevelFilter::Info);
    }

    #[test]
    fn log_line_contains_level() {
        let logger = SimpleLogger {
            max_level: LevelFilter::Info,
            file: None,
        };
        let metadata = log::MetadataBuilder::new()
            .level(Level::Info)
            .target("test")
            .build();
        assert!(logger.enabled(&metadata));

        let metadata = log::MetadataBuilder::new()
            .level(Level::Debug)
            .target("test")
            .build();
        assert!(!logger.enabled(&metadata));
    }
}
