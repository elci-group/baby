// Copyright (c) 2026 sal
// SPDX-License-Identifier: MIT

use crate::versioning::Version;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Channel {
    Stable,
    Nightly,
    Bleeding,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DiscoverySource {
    ConfigExplicit,
    ConfigScanned,
    FilesystemScanned,
}

#[derive(Debug, Clone)]
pub struct Tool {
    pub name: String,
    pub repo: String,
    pub directory: Option<String>,
    pub recipe: Option<String>,
    pub channel: Option<Channel>,
    pub source: DiscoverySource,
}

#[derive(Debug, Clone)]
pub struct UpdateInfo {
    pub tool: Tool,
    pub installed_version: Option<Version>,
    pub latest_version: Option<Version>,
    pub is_outdated: bool,
    pub status_reason: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UpdateStatus {
    Update,
    Install,
    Current,
    Error,
    Skipped,
}

#[derive(Debug, Clone)]
pub struct ExecutionPlan {
    pub tools: Vec<UpdateInfo>,
    pub dry_run: bool,
    pub parallelism: usize,
    pub strip: bool,
    pub backup: bool,
    pub sudo: bool,
    pub user: bool,
}

#[derive(Debug, Clone)]
pub struct ExecutionResult {
    pub tool_name: String,
    pub status: UpdateStatus,
    pub message: String,
    pub duration_ms: u128,
}

#[derive(Debug)]
pub struct ExecutionReport {
    pub total: usize,
    pub succeeded: usize,
    pub failed: usize,
    pub skipped: usize,
    pub results: Vec<ExecutionResult>,
    pub total_duration_ms: u128,
}

#[derive(Debug, Deserialize, Serialize, Default)]
pub struct BoomConfig {
    pub boom: Option<BoomSection>,
    pub tools: Option<Vec<ToolDeclaration>>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct BoomSection {
    pub channel: Option<String>,
    pub parallelism: Option<usize>,
    pub scan_dirs: Option<Vec<String>>,
    pub strip: Option<bool>,
    pub backup: Option<bool>,
    pub sudo: Option<bool>,
    pub user: Option<bool>,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct ToolDeclaration {
    pub name: String,
    pub repo: String,
    pub dir: Option<String>,
    pub recipe: Option<String>,
    pub channel: Option<String>,
}

impl Tool {
    pub fn new(name: String, repo: String, source: DiscoverySource) -> Self {
        Self {
            name,
            repo,
            directory: None,
            recipe: None,
            channel: None,
            source,
        }
    }

    pub fn with_directory(mut self, directory: Option<String>) -> Self {
        self.directory = directory;
        self
    }

    pub fn with_recipe(mut self, recipe: Option<String>) -> Self {
        self.recipe = recipe;
        self
    }

    pub fn with_channel(mut self, channel: Option<Channel>) -> Self {
        self.channel = channel;
        self
    }
}

impl UpdateInfo {
    pub fn new(tool: Tool) -> Self {
        Self {
            tool,
            installed_version: None,
            latest_version: None,
            is_outdated: false,
            status_reason: "Pending detection".to_string(),
        }
    }

    pub fn status(&self) -> UpdateStatus {
        if self.is_outdated {
            UpdateStatus::Update
        } else if self.installed_version.is_none() && self.latest_version.is_some() {
            UpdateStatus::Install
        } else if self.installed_version.is_some() && self.latest_version.is_some() {
            UpdateStatus::Current
        } else {
            UpdateStatus::Error
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tool_builder() {
        let tool = Tool::new("test".to_string(), "https://example.com/test.git".to_string(), DiscoverySource::ConfigExplicit)
            .with_directory(Some("cmd".to_string()))
            .with_channel(Some(Channel::Stable));

        assert_eq!(tool.name, "test");
        assert_eq!(tool.directory, Some("cmd".to_string()));
        assert_eq!(tool.channel, Some(Channel::Stable));
    }

    #[test]
    fn update_info_status() {
        let tool = Tool::new("test".to_string(), "https://example.com/test.git".to_string(), DiscoverySource::ConfigExplicit);
        let update = UpdateInfo::new(tool);
        assert_eq!(update.status(), UpdateStatus::Error);
    }
}
