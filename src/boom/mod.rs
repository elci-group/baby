// Copyright (c) 2026 sal
// SPDX-License-Identifier: MIT

pub mod types;
pub mod config;
pub mod discovery;
pub mod detection;
pub mod execution;
pub mod interactive;

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
    _dry_run: bool,
    _yes: bool,
    _interactive: bool,
    _parallelism: Option<usize>,
    _filter: Option<Vec<String>>,
) -> Result<()> {
    log::info!("🧨 boom: discovering tools...");
    log::warn!("boom command is under development");
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
