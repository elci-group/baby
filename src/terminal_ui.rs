//! Terminal-native installation animation and completion telemetry.

use std::io::{self, IsTerminal, Write};
use std::path::Path;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::thread::{self, JoinHandle};
use std::time::Duration;

/// Data rendered after a successful installation.
pub struct InstallTelemetry<'a> {
    pub project: &'a str,
    pub install_path: &'a Path,
    pub elapsed: Duration,
    pub build_commands: usize,
    pub artifact_bytes: u64,
    pub cleanup_ran: bool,
    pub dry_run: bool,
}

/// A TTY-only crying-baby progress animation.
pub struct InstallAnimation {
    stopped: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
}

impl InstallAnimation {
    /// Start the animation only for interactive, real installations.
    pub fn start(project: &str, enabled: bool) -> Option<Self> {
        if !enabled || !io::stderr().is_terminal() {
            return None;
        }

        let stopped = Arc::new(AtomicBool::new(false));
        let stop_signal = Arc::clone(&stopped);
        let label = project.to_owned();
        let worker = thread::spawn(move || {
            const FRAMES: [&str; 4] = ["👶😭  waaah", "👶💧  waaah", "👶😭  WAAAH", "👶💧  WAAAH"];
            let mut index = 0;
            while !stop_signal.load(Ordering::Relaxed) {
                let mut stderr = io::stderr().lock();
                let _ = write!(
                    stderr,
                    "\r\x1b[2K{}  building and installing {label}…",
                    FRAMES[index]
                );
                let _ = stderr.flush();
                index = (index + 1) % FRAMES.len();
                thread::sleep(Duration::from_millis(140));
            }
        });

        Some(Self {
            stopped,
            worker: Some(worker),
        })
    }

    /// End the crying animation, let the baby sleep to the right, then reveal telemetry.
    pub fn finish(&mut self, telemetry: &InstallTelemetry<'_>) {
        self.stop();
        let mut stderr = io::stderr().lock();
        for offset in (0..=24).step_by(4) {
            let _ = write!(stderr, "\r\x1b[2K{:offset$}👶💤", "");
            let _ = stderr.flush();
            thread::sleep(Duration::from_millis(70));
        }
        let cleanup = if telemetry.cleanup_ran {
            "completed"
        } else {
            "skipped"
        };
        let mode = if telemetry.dry_run {
            "dry run"
        } else {
            "installed"
        };
        let _ = write!(
            stderr,
            "\r\x1b[2K                        😴 zzz\n\
             ┌─ installation complete ─────────────────────────\n\
             │ project:   {} ({mode})\n\
             │ installed: {}\n\
             │ elapsed:   {}\n\
             │ build:     {} command(s)\n\
             │ artifact:  {}\n\
             │ cleanup:   {cleanup}\n\
             └─────────────────────────────────────────────────\n",
            telemetry.project,
            telemetry.install_path.display(),
            format_elapsed(telemetry.elapsed),
            telemetry.build_commands,
            format_bytes(telemetry.artifact_bytes),
        );
        let _ = stderr.flush();
    }

    fn stop(&mut self) {
        self.stopped.store(true, Ordering::Relaxed);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
        let mut stderr = io::stderr().lock();
        let _ = write!(stderr, "\r\x1b[2K");
        let _ = stderr.flush();
    }
}

impl Drop for InstallAnimation {
    fn drop(&mut self) {
        self.stop();
    }
}

fn format_elapsed(elapsed: Duration) -> String {
    format!("{:.2}s", elapsed.as_secs_f64())
}

fn format_bytes(bytes: u64) -> String {
    const UNITS: [&str; 4] = ["B", "KiB", "MiB", "GiB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_bytes_for_install_summary() {
        assert_eq!(format_bytes(999), "999 B");
        assert_eq!(format_bytes(1_536), "1.5 KiB");
    }

    #[test]
    fn formats_elapsed_for_install_summary() {
        assert_eq!(format_elapsed(Duration::from_millis(1_250)), "1.25s");
    }
}
