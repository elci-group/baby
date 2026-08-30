// Copyright (c) 2026 sal
// SPDX-License-Identifier: MIT

//! Live progress reporting for [`super::execution::execute_updates`].
//!
//! On an interactive terminal, renders a three-zone grid — Requirements
//! (queued/building) | Compiled | Errors — redrawn in place as
//! [`super::types::ToolEvent`]s arrive, so packages visibly move from the
//! queue into whichever outcome zone they settle in, using horizontal
//! space (three columns) instead of one line per tool. The block is
//! height-bounded regardless of tool count: each zone shows at most
//! [`MAX_ROWS_PER_ZONE`] entries and collapses the rest to a single
//! "+N more" line.
//!
//! Off a terminal (piped output, CI, `NO_COLOR` scripts) the same events
//! are instead logged one line per transition via `log::*!`, so nothing
//! about this feature depends on a live terminal to be legible — see
//! [`run_log_fallback`]. Either way, [`ProgressReporter::finish`] must be
//! awaited before printing the final summary table
//! (`execution::show_execution_report`), which remains the single,
//! persisted record of every tool's outcome.

use std::io::{self, IsTerminal, Write};
use std::time::Duration;

use form3::anim::Spinner;
use form3::table::{ColumnConstraint, Table, TableStyle};
use tokio::sync::mpsc::{self, UnboundedReceiver, UnboundedSender};

use crate::logger::{RENDER, erase_drawn, replace_drawn};

use super::types::{ToolEvent, ToolPhase, UpdateStatus};

const FRAME_INTERVAL: Duration = Duration::from_millis(160);
const MAX_ROWS_PER_ZONE: usize = 8;

/// Consumes [`ToolEvent`]s for one `execute_updates` run, either through a
/// live redrawn grid or a plain log fallback. Own the returned
/// [`UnboundedSender`] into `execute_updates`'s `events` parameter, then
/// `.await` [`finish`](Self::finish) once it returns and before printing
/// the final summary.
pub struct ProgressReporter {
    handle: tokio::task::JoinHandle<()>,
}

impl ProgressReporter {
    pub fn start(total: usize) -> (Self, UnboundedSender<ToolEvent>) {
        let (tx, rx) = mpsc::unbounded_channel();
        let handle = if io::stderr().is_terminal() {
            tokio::spawn(run_grid(rx, total))
        } else {
            tokio::spawn(run_log_fallback(rx))
        };
        (Self { handle }, tx)
    }

    /// Wait for the reporter to drain and settle. Safe to call even if the
    /// paired sender was dropped without sending anything.
    pub async fn finish(self) {
        let _ = self.handle.await;
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ZoneEntry {
    Queued,
    Building,
    Compiled(UpdateStatus),
    Error,
}

struct GridState {
    tools: Vec<(String, ZoneEntry)>,
}

impl GridState {
    fn new(total: usize) -> Self {
        Self {
            tools: Vec::with_capacity(total),
        }
    }

    fn apply(&mut self, event: ToolEvent) {
        let entry = match event.phase {
            ToolPhase::Queued => ZoneEntry::Queued,
            ToolPhase::Building => ZoneEntry::Building,
            ToolPhase::Done(UpdateStatus::Error) => ZoneEntry::Error,
            ToolPhase::Done(status) => ZoneEntry::Compiled(status),
        };
        if let Some(slot) = self
            .tools
            .iter_mut()
            .find(|(name, _)| *name == event.tool_name)
        {
            slot.1 = entry;
        } else {
            self.tools.push((event.tool_name, entry));
        }
    }

    fn zone(&self, want: impl Fn(ZoneEntry) -> bool) -> Vec<(&str, ZoneEntry)> {
        self.tools
            .iter()
            .filter(|(_, entry)| want(*entry))
            .map(|(name, entry)| (name.as_str(), *entry))
            .collect()
    }
}

/// Cap `items` to `cap` labeled entries, collapsing the remainder into a
/// single "+N more" line — this, not column count, is what keeps the grid
/// vertically bounded regardless of how many tools are in flight.
fn capped_labels<T>(items: &[T], cap: usize, label: impl Fn(&T) -> String) -> Vec<String> {
    if items.len() <= cap {
        items.iter().map(label).collect()
    } else {
        let mut out: Vec<String> = items[..cap].iter().map(label).collect();
        out.push(format!("… +{} more", items.len() - cap));
        out
    }
}

fn terminal_size() -> (u16, u16) {
    let mut ws: libc::winsize = unsafe { std::mem::zeroed() };
    let ok = unsafe { libc::ioctl(libc::STDERR_FILENO, libc::TIOCGWINSZ, &mut ws) };
    if ok == 0 && ws.ws_col > 0 && ws.ws_row > 0 {
        (ws.ws_col, ws.ws_row)
    } else {
        (80, 24)
    }
}

fn draw(state: &GridState, frame: usize) {
    let spinner = Spinner::default();
    let (cols, rows) = terminal_size();
    let col_width = ((cols.saturating_sub(10)) / 3).clamp(8, 40);
    let row_cap = ((rows as usize).saturating_sub(8)).clamp(3, MAX_ROWS_PER_ZONE);

    let requirements = state.zone(|e| matches!(e, ZoneEntry::Queued | ZoneEntry::Building));
    let compiled = state.zone(|e| matches!(e, ZoneEntry::Compiled(_)));
    let errors = state.zone(|e| matches!(e, ZoneEntry::Error));

    let req_lines = capped_labels(&requirements, row_cap, |(name, entry)| match entry {
        ZoneEntry::Building => format!("{} {name}", spinner.frame(frame as u64)),
        _ => format!("· {name}"),
    });
    let compiled_lines = capped_labels(&compiled, row_cap, |(name, entry)| match entry {
        ZoneEntry::Compiled(status) => format!("{} {name}", status.symbol()),
        _ => name.to_string(),
    });
    let error_lines = capped_labels(&errors, row_cap, |(name, _)| format!("✗ {name}"));

    let mut table = Table::new();
    table.set_style(TableStyle::Ascii);
    table.set_header(vec![
        format!("Requirements ({})", requirements.len()),
        format!("Compiled ({})", compiled.len()),
        format!("Errors ({})", errors.len()),
    ]);
    table.set_constraints(vec![ColumnConstraint::MaxWidth(col_width); 3]);

    let row_count = req_lines
        .len()
        .max(compiled_lines.len())
        .max(error_lines.len());
    for i in 0..row_count {
        table.add_row(vec![
            req_lines.get(i).cloned().unwrap_or_default(),
            compiled_lines.get(i).cloned().unwrap_or_default(),
            error_lines.get(i).cloned().unwrap_or_default(),
        ]);
    }

    let rendered = table.to_string();
    let lines = rendered.lines().count();

    let mut render = RENDER.lock().unwrap_or_else(|e| e.into_inner());
    let mut stderr = io::stderr();
    replace_drawn(&mut stderr, render.drawn_lines, &rendered);
    let _ = stderr.flush();
    render.drawn_lines = lines;
}

async fn run_grid(mut rx: UnboundedReceiver<ToolEvent>, total: usize) {
    let mut state = GridState::new(total);
    let mut ticker = tokio::time::interval(FRAME_INTERVAL);
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    let mut frame = 0usize;

    draw(&state, frame);
    loop {
        tokio::select! {
            event = rx.recv() => {
                match event {
                    Some(event) => state.apply(event),
                    None => break,
                }
            }
            _ = ticker.tick() => {
                frame = frame.wrapping_add(1);
            }
        }
        draw(&state, frame);
    }

    let mut render = RENDER.lock().unwrap_or_else(|e| e.into_inner());
    let mut stderr = io::stderr();
    erase_drawn(&mut stderr, render.drawn_lines);
    render.drawn_lines = 0;
    let _ = stderr.flush();
}

/// Non-TTY fallback: one deterministic log line per transition, so piped
/// output and CI logs stay legible without any live-terminal redraw.
async fn run_log_fallback(mut rx: UnboundedReceiver<ToolEvent>) {
    while let Some(event) = rx.recv().await {
        match event.phase {
            ToolPhase::Queued => log::info!("· {} queued", event.tool_name),
            ToolPhase::Building => log::info!("🔨 {} building...", event.tool_name),
            ToolPhase::Done(UpdateStatus::Error) => {
                log::error!("❌ {}: {}", event.tool_name, event.detail)
            }
            ToolPhase::Done(status) => {
                log::info!("{} {}: {}", status.symbol(), event.tool_name, event.detail)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capped_labels_collapses_overflow_into_one_line() {
        let items = vec!["a", "b", "c", "d", "e"];
        let labels = capped_labels(&items, 3, |s| s.to_string());
        assert_eq!(labels, vec!["a", "b", "c", "… +2 more"]);
    }

    #[test]
    fn capped_labels_passes_through_when_under_cap() {
        let items = vec!["a", "b"];
        let labels = capped_labels(&items, 3, |s| s.to_string());
        assert_eq!(labels, vec!["a", "b"]);
    }

    #[test]
    fn grid_state_moves_tool_between_zones_on_new_events() {
        let mut state = GridState::new(1);
        state.apply(ToolEvent {
            tool_name: "amber".to_string(),
            phase: ToolPhase::Queued,
            detail: "queued".to_string(),
        });
        assert_eq!(state.zone(|e| matches!(e, ZoneEntry::Queued)).len(), 1);
        assert_eq!(state.zone(|e| matches!(e, ZoneEntry::Compiled(_))).len(), 0);

        state.apply(ToolEvent {
            tool_name: "amber".to_string(),
            phase: ToolPhase::Done(UpdateStatus::Update),
            detail: "done".to_string(),
        });
        assert_eq!(state.zone(|e| matches!(e, ZoneEntry::Queued)).len(), 0);
        assert_eq!(state.zone(|e| matches!(e, ZoneEntry::Compiled(_))).len(), 1);
    }

    #[tokio::test]
    async fn progress_reporter_finishes_when_sender_drops_immediately() {
        let (reporter, tx) = ProgressReporter::start(0);
        drop(tx);
        reporter.finish().await;
    }
}
