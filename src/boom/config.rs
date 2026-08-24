// Copyright (c) 2026 sal
// SPDX-License-Identifier: MIT

use crate::error::{BabyError, Result};
use std::fs;
use std::path::{Path, PathBuf};

use super::types::{BoomConfig, Channel};

pub fn parse_boom_config(path: &Path) -> Result<BoomConfig> {
    let content = fs::read_to_string(path)
        .map_err(|e| BabyError::io(format!("read {}", path.display()), e))?;
    toml::from_str(&content).map_err(|e| BabyError::config_parse(path.display().to_string(), e))
}

pub fn find_boom_config(root: &Path) -> Option<PathBuf> {
    let local = root.join(".boom.toml");
    if local.exists() {
        return Some(local);
    }

    let config_dir = crate::xdg_config_dir().join("boom").join("boom.toml");
    if config_dir.exists() {
        return Some(config_dir);
    }

    None
}

pub fn resolve_channel(tool_channel: Option<&str>, config_channel: Option<&str>) -> Channel {
    if let Some(ch) = tool_channel {
        match ch {
            "nightly" => Channel::Nightly,
            "bleeding" => Channel::Bleeding,
            _ => Channel::Stable,
        }
    } else if let Some(ch) = config_channel {
        match ch {
            "nightly" => Channel::Nightly,
            "bleeding" => Channel::Bleeding,
            _ => Channel::Stable,
        }
    } else {
        Channel::Stable
    }
}

pub fn get_scan_dirs(config: &BoomConfig, project_root: &Path) -> Vec<PathBuf> {
    let mut dirs = vec![project_root.to_path_buf()];

    if let Some(ref boom) = config.boom {
        if let Some(ref scan_dirs) = boom.scan_dirs {
            for dir in scan_dirs {
                let path = project_root.join(dir);
                if path.exists() {
                    dirs.push(path);
                }
            }
        }
    }

    dirs
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_channel_precedence() {
        assert_eq!(
            resolve_channel(Some("nightly"), Some("stable")),
            Channel::Nightly
        );
        assert_eq!(resolve_channel(None, Some("bleeding")), Channel::Bleeding);
        assert_eq!(resolve_channel(None, None), Channel::Stable);
    }

    #[test]
    fn parse_example_config() {
        let toml_str = r#"
[boom]
channel = "stable"
scan_dirs = ["tools"]

[[tools]]
name = "test"
repo = "https://example.com/test.git"
"#;

        let config: BoomConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(
            config.boom.as_ref().unwrap().channel,
            Some("stable".to_string())
        );
        assert_eq!(config.tools.as_ref().unwrap()[0].name, "test");
    }
}
