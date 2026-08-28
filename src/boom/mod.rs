// Copyright (c) 2026 sal
// SPDX-License-Identifier: MIT

pub mod config;
pub mod detection;
pub mod discovery;
pub mod execution;
mod grid;
pub mod interactive;
pub mod types;

pub use types::*;

use crate::error::Result;

pub async fn init_boom_config() -> crate::error::Result<()> {
    let cwd = std::env::current_dir()
        .map_err(|e| crate::error::BabyError::io("get current directory", e))?;
    let config_path = cwd.join(".boom.toml");

    if config_path.exists() {
        log::warn!(".boom.toml already exists at {}", config_path.display());
        return Ok(());
    }

    log::info!("📝 Initializing .boom.toml...");

    let tools = discovery::discover_tools(&cwd).await?;
    let config = generate_boom_config(&tools)?;

    std::fs::write(&config_path, config)
        .map_err(|e| crate::error::BabyError::io(config_path.display().to_string(), e))?;

    log::info!("✅ Created .boom.toml with {} tool(s)", tools.len());
    println!("\n📋 Generated .boom.toml configuration.");
    println!("Edit the file to customize tool discovery and channels.\n");

    Ok(())
}

pub async fn run_boom(
    dry_run: bool,
    yes: bool,
    interactive: bool,
    parallelism: Option<usize>,
    filter: Option<Vec<String>>,
) -> Result<()> {
    log::info!("🧨 boom: discovering tools...");

    let cwd = std::env::current_dir()
        .map_err(|e| crate::error::BabyError::io("get current directory", e))?;

    let mut tools = discovery::discover_tools(&cwd).await?;
    if let Some(ref names) = filter {
        tools.retain(|t| names.iter().any(|n| n == &t.name));
    }

    if tools.is_empty() {
        log::warn!("no tools discovered; create a .boom.toml or run `baby boom --init`");
        return Ok(());
    }

    let boom_config = match config::find_boom_config(&cwd) {
        Some(path) => config::parse_boom_config(&path)?,
        None => types::BoomConfig::default(),
    };
    let boom_section = boom_config.boom.as_ref();

    let parallelism = parallelism
        .or_else(|| boom_section.and_then(|b| b.parallelism))
        .unwrap_or(4)
        .max(1);

    log::info!("🔎 boom: checking {} tool(s) for updates...", tools.len());
    let updates = detection::detect_updates(&tools, parallelism).await?;

    execution::show_dry_run(&updates)?;

    let selected_indices = if interactive {
        interactive::select_updates_interactive(&updates)?
    } else {
        updates
            .iter()
            .enumerate()
            .filter(|(_, u)| u.is_outdated)
            .map(|(i, _)| i)
            .collect()
    };

    if selected_indices.is_empty() {
        log::info!("✅ nothing to update.");
        return Ok(());
    }

    let selected: Vec<types::UpdateInfo> = selected_indices
        .into_iter()
        .map(|i| updates[i].clone())
        .collect();

    if !yes && !dry_run && !execution::confirm_updates(&selected)? {
        log::info!("aborted.");
        return Ok(());
    }

    let plan = types::ExecutionPlan {
        tools: selected,
        dry_run,
        parallelism,
        strip: boom_section.and_then(|b| b.strip).unwrap_or(false),
        backup: boom_section.and_then(|b| b.backup).unwrap_or(false),
        sudo: boom_section.and_then(|b| b.sudo).unwrap_or(false),
        user: boom_section.and_then(|b| b.user).unwrap_or(false),
    };

    let (reporter, tx) = grid::ProgressReporter::start(plan.tools.len());
    let report = execution::execute_updates(&plan, Some(tx)).await?;
    reporter.finish().await;
    execution::show_execution_report(&report)?;

    if report.failed > 0 {
        return Err(crate::error::BabyError::new(
            crate::error::ErrorKind::CommandFailed,
            format!(
                "{} of {} tool(s) failed to update",
                report.failed, report.total
            ),
        ));
    }

    Ok(())
}

fn generate_boom_config(tools: &[types::Tool]) -> Result<String> {
    let mut config = String::from(
        r#"# boom configuration
# Managed tools to keep up to date

[boom]
# Channel: stable (default), nightly, or bleeding
channel = "stable"

# Directories to scan for .baby.toml files (relative to this file)
scan_dirs = [".", "tools"]

# Parallel workers for simultaneous updates
# parallelism = 4

"#,
    );

    if !tools.is_empty() {
        config.push_str("# Discovered tools:\n");
        for tool in tools {
            config.push_str(&format!(
                "\n# [[tools]]\n# name = \"{}\"\n# repo = \"{}\"\n",
                tool.name, tool.repo
            ));
        }
    }

    Ok(config)
}
