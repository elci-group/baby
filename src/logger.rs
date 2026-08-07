//! A tiny `log` implementation that replaces `env_logger`.
//!
//! `setup_logging` targets stderr; `setup_daemon_logging` writes to stderr and
//! the birthd log file. The default level is `info`; override with `RUST_LOG`
//! (`error`, `warn`, `info`, `debug`, `trace`).

use std::fs::{File, OpenOptions};
use std::io::{self, Write};
use std::sync::Mutex;
use std::time::SystemTime;

use log::{LevelFilter, Log, Metadata, Record};

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

        let timestamp = format_timestamp(SystemTime::now());
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

/// Format a [`SystemTime`] as `YYYY-MM-DDTHH:MM:SSZ` in UTC using only `std`.
fn format_timestamp(st: SystemTime) -> String {
    let duration = st
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default();
    let secs = duration.as_secs();
    let (year, month, day, hour, minute, second) = secs_to_utc(secs);
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z",
        year, month, day, hour, minute, second
    )
}

fn is_leap_year(year: u32) -> bool {
    year.is_multiple_of(4) && (!year.is_multiple_of(100) || year.is_multiple_of(400))
}

fn secs_to_utc(mut secs: u64) -> (u32, u32, u32, u32, u32, u32) {
    const SECS_PER_DAY: u64 = 86_400;
    let ss = secs % 60;
    secs /= 60;
    let mm = secs % 60;
    let hh = (secs / 60) % 24;
    let mut days = secs / 60 / 24;

    let mut year = 1970u32;
    loop {
        let days_in_year = if is_leap_year(year) { 366 } else { 365 };
        if days < days_in_year as u64 {
            break;
        }
        days -= days_in_year as u64;
        year += 1;
    }

    let days_in_month: [u32; 12] = [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
    let mut month = 1u32;
    loop {
        let dim = if month == 2 && is_leap_year(year) {
            29
        } else {
            days_in_month[(month - 1) as usize]
        };
        if days < dim as u64 {
            break;
        }
        days -= dim as u64;
        month += 1;
    }

    (
        year,
        month,
        (days + 1) as u32,
        hh as u32,
        mm as u32,
        ss as u32,
    )
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

    #[test]
    fn format_unix_epoch() {
        assert_eq!(
            format_timestamp(SystemTime::UNIX_EPOCH),
            "1970-01-01T00:00:00Z"
        );
    }

    #[test]
    fn format_known_datetime() {
        // 2024-03-15 12:30:45 UTC = 1710505845 seconds since epoch.
        let st = SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_710_505_845);
        assert_eq!(format_timestamp(st), "2024-03-15T12:30:45Z");
    }
}
