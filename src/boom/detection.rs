// Copyright (c) 2026 sal
// SPDX-License-Identifier: MIT

use crate::error::{BabyError, Result};
use crate::versioning::Version;
use std::process::Command;

use super::types::{Channel, Tool, UpdateInfo};

pub async fn detect_updates(tools: &[Tool], parallelism: usize) -> Result<Vec<UpdateInfo>> {
    let mut handles = vec![];

    for tool in tools {
        let tool = tool.clone();
        let handle = tokio::task::spawn_blocking(move || detect_single_update(&tool));
        handles.push(handle);

        if handles.len() >= parallelism {
            for handle in handles.drain(..) {
                let _ = handle.await.map_err(|e| {
                    BabyError::new(
                        crate::error::ErrorKind::ConfigParse,
                        format!("detection task failed: {}", e),
                    )
                })??;
            }
        }
    }

    let mut updates = vec![];
    for handle in handles {
        let update = handle.await.map_err(|e| {
            BabyError::new(
                crate::error::ErrorKind::ConfigParse,
                format!("detection task failed: {}", e),
            )
        })??;
        updates.push(update);
    }

    // Collect remaining results from earlier batches
    Ok(updates)
}

fn detect_single_update(tool: &Tool) -> Result<UpdateInfo> {
    let mut update = UpdateInfo::new(tool.clone());

    update.installed_version = get_installed_version(&tool.name);

    match get_latest_version(&tool.repo, tool.channel) {
        Ok(latest) => {
            update.latest_version = Some(latest.clone());

            if let Some(ref installed) = update.installed_version {
                update.is_outdated = installed < &latest;
                update.status_reason = if update.is_outdated {
                    format!("Update available: {} -> {}", installed, latest)
                } else {
                    format!("Already at latest: {}", latest)
                };
            } else {
                update.is_outdated = true;
                update.status_reason = format!("Not installed (latest: {})", latest);
            }
        }
        Err(e) => {
            update.status_reason = format!("Failed to detect updates: {}", e);
        }
    }

    Ok(update)
}

fn get_installed_version(tool_name: &str) -> Option<Version> {
    let output = Command::new("which").arg(tool_name).output().ok()?;

    if !output.status.success() {
        return None;
    }

    let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let version_output = Command::new(&path).arg("--version").output().ok()?;

    if !version_output.status.success() {
        return None;
    }

    let version_str = String::from_utf8_lossy(&version_output.stdout);

    for word in version_str.split_whitespace() {
        if let Ok(version) = Version::parse(word) {
            return Some(version);
        }
    }

    None
}

fn get_latest_version(repo: &str, channel: Option<Channel>) -> Result<Version> {
    let output = Command::new("git")
        .arg("ls-remote")
        .arg("--tags")
        .arg(repo)
        .output()
        .map_err(|e| BabyError::io(format!("git ls-remote {}", repo), e))?;

    if !output.status.success() {
        return Err(BabyError::new(
            crate::error::ErrorKind::VersionCheck,
            format!("Failed to query remote: {}", repo),
        ));
    }

    let output_str = String::from_utf8_lossy(&output.stdout);
    let mut versions = vec![];

    for line in output_str.lines() {
        if let Some(tag) = line.split_whitespace().nth(1) {
            let tag = tag.trim_start_matches("refs/tags/").trim_end_matches("^{}");

            if let Ok(version) = Version::parse(tag) {
                match channel {
                    Some(Channel::Stable) => {
                        if version.prerelease.is_none() {
                            versions.push(version);
                        }
                    }
                    Some(Channel::Nightly) => {
                        if version.prerelease.is_some() {
                            versions.push(version);
                        }
                    }
                    Some(Channel::Bleeding) => {
                        versions.push(version);
                    }
                    None => {
                        if version.prerelease.is_none() {
                            versions.push(version);
                        }
                    }
                }
            }
        }
    }

    versions.sort();
    versions.pop().ok_or_else(|| {
        BabyError::new(
            crate::error::ErrorKind::VersionCheck,
            format!("No versions found in {}", repo),
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_version_parsing_from_git_output() {
        let line = "abc123def456\trefs/tags/v1.2.3";
        let parts: Vec<&str> = line.split_whitespace().collect();
        let tag = parts[1].trim_start_matches("refs/tags/");

        let version = Version::parse(tag).unwrap();
        assert_eq!(version.major, 1);
        assert_eq!(version.minor, 2);
        assert_eq!(version.patch, 3);
    }
}
