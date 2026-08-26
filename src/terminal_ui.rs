//! Terminal-native installation animation and completion telemetry.
//!
//! Renders a small full-colour pixel-art baby (via 24-bit ANSI foreground
//! codes) plus a live "current stage · elapsed" line beneath it, so a user
//! watching an install can see exactly what's happening and for how long —
//! not just a single generic spinner. The block is redrawn in place on a
//! background thread; any `log::*!` call (see `logger::RENDER`) erases it
//! first, and callers `pause()`/`resume()` it around child processes that
//! inherit the terminal directly (e.g. `cargo build`), so the art never
//! races with output it doesn't control.

use std::io::{self, IsTerminal, Write};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use crate::logger::{RENDER, erase_drawn};

const BABY_WIDTH: usize = 10;
const BABY_HEIGHT: usize = 7;
const FRAME_INTERVAL: Duration = Duration::from_millis(160);

/// Four crying-baby frames cycled while a build/install is in progress:
/// mouth alternates open/closed and a tear blinks between cheeks.
const CRY_FRAMES: [[&str; BABY_HEIGHT]; 4] = [
    [
        "..hhhhhh..",
        "..hssssh..",
        "..skssks..",
        "..tsossc..",
        "...ssss...",
        "..bbbbbb..",
        ".bbbbbbbb.",
    ],
    [
        "..hhhhhh..",
        "..hssssh..",
        "..skssks..",
        "..csmssc..",
        "...ssss...",
        "..bbbbbb..",
        ".bbbbbbbb.",
    ],
    [
        "..hhhhhh..",
        "..hssssh..",
        "..skssks..",
        "..csosst..",
        "...ssss...",
        "..bbbbbb..",
        ".bbbbbbbb.",
    ],
    [
        "..hhhhhh..",
        "..hssssh..",
        "..skssks..",
        "..csossc..",
        "...ssss...",
        "..bbbbbb..",
        "bbbbbbbb..",
    ],
];

/// Content, sleepy baby shown once installation has finished.
const SLEEP_FRAME: [&str; BABY_HEIGHT] = [
    "..hhhhhh..",
    "..hssssh..",
    "..skssks..",
    "..csmssc..",
    "...ssss...",
    "..bbbbbb..",
    ".bbbbbbbb.",
];

/// Map a pixel-art legend character to its 24-bit colour. `None` means the
/// pixel is transparent (background shows through).
fn pixel_color(ch: char) -> Option<(u8, u8, u8)> {
    match ch {
        'h' => Some((74, 47, 28)),    // hair
        's' => Some((246, 201, 160)), // skin
        'k' => Some((42, 33, 24)),    // eyes
        'c' => Some((242, 143, 160)), // blush
        'o' => Some((122, 32, 32)),   // open (crying) mouth
        'm' => Some((201, 122, 99)),  // closed mouth
        't' => Some((95, 184, 232)),  // tear
        'b' => Some((127, 209, 224)), // onesie
        _ => None,
    }
}

fn render_pixel_row(row: &str, out: &mut String) {
    for ch in row.chars() {
        match pixel_color(ch) {
            Some((r, g, b)) => {
                out.push_str(&format!("\x1b[38;2;{r};{g};{b}m██\x1b[0m"));
            }
            None => out.push_str("  "),
        }
    }
}

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

struct StageState {
    label: String,
    started: Instant,
}

/// A TTY-only, full-colour animated-baby progress display with a live
/// stage/elapsed status line.
pub struct InstallAnimation {
    stopped: Arc<AtomicBool>,
    paused: Arc<AtomicBool>,
    stage: Arc<Mutex<StageState>>,
    worker: Option<JoinHandle<()>>,
    started: Instant,
}

impl InstallAnimation {
    /// Start the animation only for interactive, real installations.
    pub fn start(project: &str, enabled: bool) -> Option<Self> {
        if !enabled || !io::stderr().is_terminal() {
            return None;
        }

        let started = Instant::now();
        let stopped = Arc::new(AtomicBool::new(false));
        let paused = Arc::new(AtomicBool::new(false));
        let stage = Arc::new(Mutex::new(StageState {
            label: format!("resolving {project}"),
            started,
        }));

        let stop_signal = Arc::clone(&stopped);
        let pause_signal = Arc::clone(&paused);
        let stage_signal = Arc::clone(&stage);
        let label = project.to_owned();

        let worker = thread::spawn(move || {
            let mut frame_idx = 0usize;
            while !stop_signal.load(Ordering::Relaxed) {
                if !pause_signal.load(Ordering::Relaxed) {
                    draw_frame(&stage_signal, &label, started, frame_idx);
                    frame_idx = (frame_idx + 1) % CRY_FRAMES.len();
                }
                thread::sleep(FRAME_INTERVAL);
            }
        });

        Some(Self {
            stopped,
            paused,
            stage,
            worker: Some(worker),
            started,
        })
    }

    /// Update the live stage label shown beneath the baby (e.g. "building",
    /// "installing binary"). Resets the per-stage elapsed clock.
    pub fn set_stage(&self, label: &str) {
        if let Ok(mut stage) = self.stage.lock() {
            stage.label = label.to_owned();
            stage.started = Instant::now();
        }
    }

    /// Suspend drawing and erase the current block so a child process that
    /// inherits the terminal directly (e.g. `cargo build`) can stream its
    /// own output without the animation racing it for the same lines.
    pub fn pause(&self) {
        self.paused.store(true, Ordering::Relaxed);
        let mut render = RENDER.lock().unwrap_or_else(|e| e.into_inner());
        let mut stderr = io::stderr();
        erase_drawn(&mut stderr, render.drawn_lines);
        render.drawn_lines = 0;
        let _ = stderr.flush();
    }

    /// Resume drawing after [`pause`](Self::pause).
    pub fn resume(&self) {
        self.paused.store(false, Ordering::Relaxed);
    }

    /// Stop the animation, show a sleeping baby, then reveal telemetry.
    pub fn finish(&mut self, telemetry: &InstallTelemetry<'_>) {
        self.stop();

        let mut block = String::new();
        for row in SLEEP_FRAME {
            block.push_str("  ");
            render_pixel_row(row, &mut block);
            block.push_str("  😴\n");
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
        block.push_str(&format!(
            "\n┌─ installation complete ─────────────────────────\n\
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
        ));

        let mut render = RENDER.lock().unwrap_or_else(|e| e.into_inner());
        let mut stderr = io::stderr();
        erase_drawn(&mut stderr, render.drawn_lines);
        render.drawn_lines = 0;
        let _ = stderr.write_all(block.as_bytes());
        let _ = stderr.flush();
    }

    fn stop(&mut self) {
        self.stopped.store(true, Ordering::Relaxed);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
        let mut render = RENDER.lock().unwrap_or_else(|e| e.into_inner());
        let mut stderr = io::stderr();
        erase_drawn(&mut stderr, render.drawn_lines);
        render.drawn_lines = 0;
        let _ = stderr.flush();
    }
}

impl Drop for InstallAnimation {
    fn drop(&mut self) {
        self.stop();
    }
}

/// Erase the previous block (if any), draw the current frame plus a live
/// "stage · elapsed" status line, and record how many lines were drawn so
/// the next erase (by this thread or a log write) clears exactly that much.
fn draw_frame(stage: &Arc<Mutex<StageState>>, project: &str, started: Instant, frame_idx: usize) {
    let (label, stage_elapsed) = match stage.lock() {
        Ok(stage) => (stage.label.clone(), stage.started.elapsed()),
        Err(_) => (format!("installing {project}"), Duration::ZERO),
    };

    let mut block = String::new();
    for row in CRY_FRAMES[frame_idx] {
        block.push_str("  ");
        render_pixel_row(row, &mut block);
        block.push('\n');
    }
    block.push_str(&format!(
        "  \x1b[2m{label}… {:.1}s stage · {:.1}s total\x1b[0m\n",
        stage_elapsed.as_secs_f64(),
        started.elapsed().as_secs_f64()
    ));

    let mut render = RENDER.lock().unwrap_or_else(|e| e.into_inner());
    let mut stderr = io::stderr();
    erase_drawn(&mut stderr, render.drawn_lines);
    let _ = stderr.write_all(block.as_bytes());
    let _ = stderr.flush();
    render.drawn_lines = BABY_HEIGHT + 1;
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

    #[test]
    fn every_pixel_art_row_has_uniform_width() {
        for frame in CRY_FRAMES {
            for row in frame {
                assert_eq!(row.chars().count(), BABY_WIDTH);
            }
        }
        for row in SLEEP_FRAME {
            assert_eq!(row.chars().count(), BABY_WIDTH);
        }
    }

    #[test]
    fn every_frame_has_uniform_height() {
        for frame in CRY_FRAMES {
            assert_eq!(frame.len(), BABY_HEIGHT);
        }
        assert_eq!(SLEEP_FRAME.len(), BABY_HEIGHT);
    }

    #[test]
    fn every_pixel_legend_character_is_mapped_or_transparent() {
        for frame in CRY_FRAMES {
            for row in frame {
                for ch in row.chars() {
                    assert!(
                        ch == '.' || pixel_color(ch).is_some(),
                        "unmapped pixel legend character: {ch:?}"
                    );
                }
            }
        }
    }
}
