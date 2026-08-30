// Copyright (c) 2026 sal
// SPDX-License-Identifier: MIT

use crate::error::{BabyError, ErrorKind, Result};
use form3::table::{Table, TableStyle};
use std::io::{self, Write};
use std::time::Instant;
use tokio::sync::mpsc::UnboundedSender;

use super::types::{
    ExecutionPlan, ExecutionReport, ExecutionResult, ToolEvent, ToolPhase, UpdateInfo, UpdateStatus,
};

struct DryRunRow {
    tool: String,
    installed: String,
    available: String,
    action: String,
}

pub fn show_dry_run(updates: &[UpdateInfo]) -> Result<()> {
    let mut rows = vec![];

    for update in updates {
        let installed = update
            .installed_version
            .as_ref()
            .map(|v| v.to_string())
            .unwrap_or_else(|| "—".to_string());

        let available = update
            .latest_version
            .as_ref()
            .map(|v| v.to_string())
            .unwrap_or_else(|| "—".to_string());

        let action = if update.is_outdated {
            "Update"
        } else {
            "Current"
        };

        rows.push(DryRunRow {
            tool: update.tool.name.clone(),
            installed,
            available,
            action: action.to_string(),
        });
    }

    let mut table = Table::new();
    table.set_style(TableStyle::Ascii);
    table.set_header(vec!["Tool", "Installed", "Available", "Action"]);
    for row in &rows {
        table.add_row(vec![
            row.tool.clone(),
            row.installed.clone(),
            row.available.clone(),
            row.action.clone(),
        ]);
    }
    let mut rendered = table.to_string();
    if rendered.ends_with('\n') {
        rendered.pop();
    }
    println!("\n{}\n", rendered);

    Ok(())
}

pub fn confirm_updates(updates: &[UpdateInfo]) -> Result<bool> {
    let outdated_count = updates.iter().filter(|u| u.is_outdated).count();

    if outdated_count == 0 {
        println!("✅ All tools are already up to date.");
        return Ok(false);
    }

    print!(
        "Found {} tool(s) with updates. Proceed? [y/N] ",
        outdated_count
    );
    io::stdout()
        .flush()
        .map_err(|e| BabyError::io("stdout flush", e))?;

    let mut input = String::new();
    io::stdin()
        .read_line(&mut input)
        .map_err(|e| BabyError::io("stdin read", e))?;

    Ok(input.trim().eq_ignore_ascii_case("y") || input.trim().eq_ignore_ascii_case("yes"))
}

/// Run every tool in `plan.tools` to completion, bounding concurrency by
/// `plan.parallelism`. When `events` is given, each tool reports its
/// `Queued` -> `Building` -> `Done` transitions over the channel as they
/// happen, so a live renderer (see `boom::grid`) can reflect real,
/// out-of-order progress; without a live renderer this argument is `None`
/// and the function behaves exactly as a synchronous batch runner.
pub async fn execute_updates(
    plan: &ExecutionPlan,
    events: Option<UnboundedSender<ToolEvent>>,
) -> Result<ExecutionReport> {
    let start = Instant::now();
    let parallelism = plan.parallelism.max(1);

    for update_info in &plan.tools {
        emit(&events, &update_info.tool.name, ToolPhase::Queued, "queued");
    }

    let mut handles = Vec::with_capacity(parallelism.min(plan.tools.len()));
    let mut results = Vec::with_capacity(plan.tools.len());

    for update_info in plan.tools.iter().cloned() {
        let dry_run = plan.dry_run;
        let tx = events.clone();

        let handle =
            tokio::task::spawn_blocking(move || run_single_update(update_info, dry_run, tx));
        handles.push(handle);

        if handles.len() >= parallelism {
            drain(&mut handles, &mut results).await?;
        }
    }
    drain(&mut handles, &mut results).await?;

    let succeeded = results
        .iter()
        .filter(|r| r.status == UpdateStatus::Update)
        .count();
    let failed = results
        .iter()
        .filter(|r| r.status == UpdateStatus::Error)
        .count();
    let skipped = results
        .iter()
        .filter(|r| matches!(r.status, UpdateStatus::Current | UpdateStatus::Skipped))
        .count();

    Ok(ExecutionReport {
        total: plan.tools.len(),
        succeeded,
        failed,
        skipped,
        results,
        total_duration_ms: start.elapsed().as_millis(),
    })
}

async fn drain(
    handles: &mut Vec<tokio::task::JoinHandle<ExecutionResult>>,
    results: &mut Vec<ExecutionResult>,
) -> Result<()> {
    for handle in handles.drain(..) {
        let result = handle.await.map_err(|e| {
            BabyError::new(ErrorKind::CommandFailed, format!("update task failed: {e}"))
        })?;
        results.push(result);
    }
    Ok(())
}

fn emit(
    events: &Option<UnboundedSender<ToolEvent>>,
    tool_name: &str,
    phase: ToolPhase,
    detail: &str,
) {
    if let Some(tx) = events {
        let _ = tx.send(ToolEvent {
            tool_name: tool_name.to_string(),
            phase,
            detail: detail.to_string(),
        });
    }
}

/// Run one tool to completion. Blocking (spawns `git`/`baby` child
/// processes synchronously) — always called via `spawn_blocking`, never
/// directly on an async task, mirroring `detection::detect_single_update`.
fn run_single_update(
    update_info: UpdateInfo,
    dry_run: bool,
    events: Option<UnboundedSender<ToolEvent>>,
) -> ExecutionResult {
    let tool_start = Instant::now();
    let name = &update_info.tool.name;

    if dry_run {
        emit(
            &events,
            name,
            ToolPhase::Done(UpdateStatus::Skipped),
            "dry run",
        );
        return ExecutionResult {
            tool_name: name.clone(),
            status: UpdateStatus::Skipped,
            message: "[DRY RUN] Would update this tool".to_string(),
            duration_ms: tool_start.elapsed().as_millis(),
        };
    }

    if !update_info.is_outdated {
        emit(
            &events,
            name,
            ToolPhase::Done(UpdateStatus::Current),
            "already up to date",
        );
        return ExecutionResult {
            tool_name: name.clone(),
            status: UpdateStatus::Current,
            message: "Already up to date".to_string(),
            duration_ms: tool_start.elapsed().as_millis(),
        };
    }

    emit(&events, name, ToolPhase::Building, "building");
    match build_and_install_tool(&update_info.tool) {
        Ok(msg) => {
            emit(&events, name, ToolPhase::Done(UpdateStatus::Update), &msg);
            ExecutionResult {
                tool_name: name.clone(),
                status: UpdateStatus::Update,
                message: msg,
                duration_ms: tool_start.elapsed().as_millis(),
            }
        }
        Err(e) => {
            let message = format!("Failed: {}", e);
            emit(
                &events,
                name,
                ToolPhase::Done(UpdateStatus::Error),
                &message,
            );
            ExecutionResult {
                tool_name: name.clone(),
                status: UpdateStatus::Error,
                message,
                duration_ms: tool_start.elapsed().as_millis(),
            }
        }
    }
}

/// Known, deterministic failure causes pulled from a failed command's
/// stderr. Returns `None` for anything unrecognized so the caller can
/// fall back to the generic error — classification only ever adds
/// specificity, it never replaces or hides the raw message.
fn classify_command_failure(stderr: &str) -> Option<ErrorKind> {
    let s = stderr;
    if s.contains("Permission denied (publickey)") {
        Some(ErrorKind::GitAuthFailed)
    } else if s.contains("Could not resolve host") {
        Some(ErrorKind::GitNetworkUnreachable)
    } else if s.contains("repository not found") || s.contains("Repository not found") {
        Some(ErrorKind::GitRepoNotFound)
    } else if s.contains("No space left on device") {
        Some(ErrorKind::DiskFull)
    } else if s.contains("linker `cc` not found") || s.contains("error: linking with") {
        Some(ErrorKind::BuildToolchainMissing)
    } else if s.contains("Permission denied") {
        Some(ErrorKind::InstallPermissionDenied)
    } else {
        None
    }
}

/// Run `cmd`, returning its stderr as a `String` on failure so callers can
/// classify it with [`classify_command_failure`].
fn run_capturing_stderr(cmd: &mut std::process::Command, context: &str) -> Result<()> {
    let output = cmd
        .output()
        .map_err(|e| BabyError::io(context.to_string(), e))?;

    if output.status.success() {
        return Ok(());
    }

    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    let kind = classify_command_failure(&stderr).unwrap_or(ErrorKind::CommandFailed);
    let detail = if stderr.trim().is_empty() {
        format!(
            "{context}: exited with status {}",
            output
                .status
                .code()
                .map(|c| c.to_string())
                .unwrap_or_else(|| "unknown".to_string())
        )
    } else {
        format!("{context}: {}", stderr.trim())
    };

    Err(BabyError::new(kind, format!("{kind}; {detail}")))
}

/// The caller (`run_single_update`) already emits a `Building` transition
/// covering this, both to the live grid and to the non-TTY log fallback —
/// this function logs nothing on its own so those two views stay the only
/// source of "building" notifications instead of a third, redundant one.
fn build_and_install_tool(tool: &crate::boom::types::Tool) -> Result<String> {
    let repo_url = &tool.repo;

    let temp_dir = tempfile::tempdir().map_err(|e| BabyError::io("create temp directory", e))?;

    run_capturing_stderr(
        std::process::Command::new("git")
            .arg("clone")
            .arg("--depth")
            .arg("1")
            .arg(repo_url)
            .arg(temp_dir.path()),
        &format!("git clone {}", repo_url),
    )?;

    run_capturing_stderr(
        std::process::Command::new("baby")
            .arg("--recipe")
            .arg(temp_dir.path().join(".baby.toml"))
            .arg("--user"),
        &format!("build {}", tool.name),
    )?;

    Ok(format!("✅ Successfully updated {}", tool.name))
}

pub fn show_execution_report(report: &ExecutionReport) -> Result<()> {
    println!("\n📊 Execution Report");
    println!("  Total:     {}", report.total);
    println!("  Succeeded: {}", report.succeeded);
    println!("  Failed:    {}", report.failed);
    println!("  Skipped:   {}", report.skipped);
    println!(
        "  Duration:  {:.2}s\n",
        report.total_duration_ms as f64 / 1000.0
    );

    for result in &report.results {
        let symbol = result.status.symbol();

        println!(
            "  {} {} - {} ({}ms)",
            symbol, result.tool_name, result.message, result.duration_ms
        );
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_dry_run_row() {
        let row = DryRunRow {
            tool: "test".to_string(),
            installed: "1.0.0".to_string(),
            available: "2.0.0".to_string(),
            action: "Update".to_string(),
        };

        assert_eq!(row.tool, "test");
        assert_eq!(row.action, "Update");
    }

    #[test]
    fn classifies_known_git_and_build_failures() {
        assert_eq!(
            classify_command_failure(
                "git@github.com: Permission denied (publickey).\nfatal: Could not read from remote repository."
            ),
            Some(ErrorKind::GitAuthFailed)
        );
        assert_eq!(
            classify_command_failure(
                "fatal: unable to access 'https://x/y.git/': Could not resolve host: x"
            ),
            Some(ErrorKind::GitNetworkUnreachable)
        );
        assert_eq!(
            classify_command_failure(
                "remote: Repository not found.\nfatal: repository 'https://x/y.git/' not found"
            ),
            Some(ErrorKind::GitRepoNotFound)
        );
        assert_eq!(
            classify_command_failure("error: could not write to file: No space left on device"),
            Some(ErrorKind::DiskFull)
        );
        assert_eq!(
            classify_command_failure(
                "error: linking with `cc` failed: exit status: 1\n= note: linker `cc` not found"
            ),
            Some(ErrorKind::BuildToolchainMissing)
        );
        assert_eq!(
            classify_command_failure(
                "cp: cannot create regular file '/usr/local/bin/x': Permission denied"
            ),
            Some(ErrorKind::InstallPermissionDenied)
        );
    }

    #[test]
    fn unrecognized_failure_classifies_as_none() {
        assert_eq!(
            classify_command_failure("something unexpected exploded"),
            None
        );
    }

    fn outdated_tool(name: &str) -> UpdateInfo {
        let tool = crate::boom::types::Tool::new(
            name.to_string(),
            format!("https://example.com/{name}.git"),
            crate::boom::types::DiscoverySource::ConfigExplicit,
        );
        let mut info = UpdateInfo::new(tool);
        info.is_outdated = true;
        info
    }

    fn current_tool(name: &str) -> UpdateInfo {
        let tool = crate::boom::types::Tool::new(
            name.to_string(),
            format!("https://example.com/{name}.git"),
            crate::boom::types::DiscoverySource::ConfigExplicit,
        );
        UpdateInfo::new(tool)
    }

    #[tokio::test]
    async fn dry_run_reports_every_tool_as_skipped_without_touching_the_network() {
        let plan = ExecutionPlan {
            tools: vec![outdated_tool("a"), outdated_tool("b"), current_tool("c")],
            dry_run: true,
            parallelism: 2,
            strip: false,
            backup: false,
            sudo: false,
            user: false,
        };

        let report = execute_updates(&plan, None).await.unwrap();

        assert_eq!(report.total, 3);
        assert_eq!(report.skipped, 3);
        assert_eq!(report.succeeded, 0);
        assert_eq!(report.failed, 0);
        assert!(
            report
                .results
                .iter()
                .all(|r| r.status == UpdateStatus::Skipped)
        );
    }

    #[tokio::test]
    async fn dry_run_emits_a_queued_and_done_event_per_tool() {
        let plan = ExecutionPlan {
            tools: vec![outdated_tool("a"), outdated_tool("b")],
            dry_run: true,
            parallelism: 2,
            strip: false,
            backup: false,
            sudo: false,
            user: false,
        };

        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let report = execute_updates(&plan, Some(tx)).await.unwrap();
        assert_eq!(report.total, 2);

        let mut events = vec![];
        while let Ok(event) = rx.try_recv() {
            events.push(event);
        }

        assert_eq!(events.len(), 4, "expected a Queued and Done event per tool");
        assert_eq!(
            events
                .iter()
                .filter(|e| e.phase == ToolPhase::Queued)
                .count(),
            2
        );
        assert!(events.iter().any(|e| e.tool_name == "a"));
        assert!(events.iter().any(|e| e.tool_name == "b"));
    }
}
