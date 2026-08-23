// Copyright (c) 2026 sal
// SPDX-License-Identifier: MIT

use crate::error::{BabyError, Result};
use std::io::{self, Write};
use std::time::Instant;
use tabled::{Table, Tabled};

use super::types::{ExecutionPlan, ExecutionReport, ExecutionResult, UpdateInfo, UpdateStatus};

#[derive(Tabled)]
struct DryRunRow {
    #[tabled(rename = "Tool")]
    tool: String,
    #[tabled(rename = "Installed")]
    installed: String,
    #[tabled(rename = "Available")]
    available: String,
    #[tabled(rename = "Action")]
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

    let table = Table::new(rows);
    println!("\n{}\n", table);

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
    io::stdout().flush()?;

    let mut input = String::new();
    io::stdin().read_line(&mut input)?;

    Ok(input.trim().eq_ignore_ascii_case("y") || input.trim().eq_ignore_ascii_case("yes"))
}

pub async fn execute_updates(plan: &ExecutionPlan) -> Result<ExecutionReport> {
    let start = Instant::now();
    let mut results = vec![];
    let mut succeeded = 0;
    let mut failed = 0;
    let mut skipped = 0;

    for update_info in &plan.tools {
        let tool_start = Instant::now();
        let result = if plan.dry_run {
            ExecutionResult {
                tool_name: update_info.tool.name.clone(),
                status: UpdateStatus::Skipped,
                message: "[DRY RUN] Would update this tool".to_string(),
                duration_ms: tool_start.elapsed().as_millis(),
            }
        } else if update_info.is_outdated {
            match build_and_install_tool(&update_info.tool, plan).await {
                Ok(msg) => {
                    succeeded += 1;
                    ExecutionResult {
                        tool_name: update_info.tool.name.clone(),
                        status: UpdateStatus::Update,
                        message: msg,
                        duration_ms: tool_start.elapsed().as_millis(),
                    }
                }
                Err(e) => {
                    failed += 1;
                    ExecutionResult {
                        tool_name: update_info.tool.name.clone(),
                        status: UpdateStatus::Error,
                        message: format!("Failed: {}", e),
                        duration_ms: tool_start.elapsed().as_millis(),
                    }
                }
            }
        } else {
            skipped += 1;
            ExecutionResult {
                tool_name: update_info.tool.name.clone(),
                status: UpdateStatus::Current,
                message: "Already up to date".to_string(),
                duration_ms: tool_start.elapsed().as_millis(),
            }
        };

        results.push(result);
    }

    let total_duration = start.elapsed().as_millis();

    Ok(ExecutionReport {
        total: plan.tools.len(),
        succeeded,
        failed,
        skipped,
        results,
        total_duration_ms: total_duration,
    })
}

async fn build_and_install_tool(
    tool: &crate::boom::types::Tool,
    plan: &ExecutionPlan,
) -> Result<String> {
    log::info!("🔨 Building {}...", tool.name);

    let repo_url = &tool.repo;
    let repo_name = repo_url
        .split('/')
        .last()
        .and_then(|s| s.strip_suffix(".git"))
        .unwrap_or("repo");

    let temp_dir = tempfile::tempdir()
        .map_err(|e| BabyError::io("create temp directory", e))?;

    let clone_status = std::process::Command::new("git")
        .arg("clone")
        .arg("--depth")
        .arg("1")
        .arg(repo_url)
        .arg(temp_dir.path())
        .status()
        .map_err(|e| BabyError::io("git clone", e))?;

    if !clone_status.success() {
        return Err(BabyError::new(
            crate::error::ErrorKind::RecipeInvalid,
            format!("Failed to clone {}", repo_url),
        ));
    }

    let build_status = std::process::Command::new("baby")
        .arg("--recipe")
        .arg(temp_dir.path().join(".baby.toml"))
        .arg("--user")
        .status()
        .map_err(|e| BabyError::io("baby build", e))?;

    if !build_status.success() {
        return Err(BabyError::new(
            crate::error::ErrorKind::RecipeInvalid,
            format!("Failed to build {}", tool.name),
        ));
    }

    Ok(format!("✅ Successfully updated {}", tool.name))
}

pub fn show_execution_report(report: &ExecutionReport) -> Result<()> {
    println!("\n📊 Execution Report");
    println!("  Total:     {}", report.total);
    println!("  Succeeded: {}", report.succeeded);
    println!("  Failed:    {}", report.failed);
    println!("  Skipped:   {}", report.skipped);
    println!("  Duration:  {:.2}s\n", report.total_duration_ms as f64 / 1000.0);

    for result in &report.results {
        let symbol = match result.status {
            UpdateStatus::Update => "✅",
            UpdateStatus::Install => "📦",
            UpdateStatus::Current => "✓",
            UpdateStatus::Error => "❌",
            UpdateStatus::Skipped => "⊘",
        };

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
}
