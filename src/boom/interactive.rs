// Copyright (c) 2026 sal
// SPDX-License-Identifier: MIT

use crate::error::Result;
use std::io::{self, Write};

use super::types::UpdateInfo;

pub fn select_updates_interactive(updates: &[UpdateInfo]) -> Result<Vec<usize>> {
    let mut selected = vec![];

    let outdated: Vec<(usize, &UpdateInfo)> = updates
        .iter()
        .enumerate()
        .filter(|(_, u)| u.is_outdated)
        .collect();

    if outdated.is_empty() {
        println!("✅ No tools need updates.");
        return Ok(vec![]);
    }

    println!("\n📋 Select tools to update:\n");

    for (i, (original_idx, update)) in outdated.iter().enumerate() {
        let current_version = update
            .installed_version
            .as_ref()
            .map(|v| v.to_string())
            .unwrap_or_else(|| "not installed".to_string());

        let new_version = update
            .latest_version
            .as_ref()
            .map(|v| v.to_string())
            .unwrap_or_else(|| "unknown".to_string());

        loop {
            print!(
                "  [{}] {} ({} → {})? [y/n/skip] ",
                i + 1,
                update.tool.name,
                current_version,
                new_version
            );
            io::stdout()
                .flush()
                .map_err(|e| crate::error::BabyError::io("stdout flush", e))?;

            let mut input = String::new();
            io::stdin()
                .read_line(&mut input)
                .map_err(|e| crate::error::BabyError::io("stdin read", e))?;

            let choice = input.trim().to_lowercase();

            if choice == "y" || choice == "yes" {
                selected.push(*original_idx);
                break;
            } else if choice == "n" || choice == "no" {
                break;
            } else if choice == "skip" || choice == "s" {
                break;
            } else {
                println!("    Please enter y, n, or skip.");
            }
        }
    }

    println!("\n📦 Selected {} tool(s) for update.\n", selected.len());

    Ok(selected)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_empty_updates() {
        let updates = vec![];
        let result = select_updates_interactive(&updates);
        assert!(result.is_ok());
        assert!(result.unwrap().is_empty());
    }
}
