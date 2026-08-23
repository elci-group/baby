// Copyright (c) 2026 sal
// SPDX-License-Identifier: MIT

pub mod types;
pub mod config;

pub use types::*;

use crate::error::Result;

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
