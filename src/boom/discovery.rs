// Copyright (c) 2026 sal
// SPDX-License-Identifier: MIT

use crate::error::Result;
use std::collections::HashMap;
use std::fs;
use std::path::Path;

use super::config;
use super::types::{Channel, DiscoverySource, Tool};

pub async fn discover_tools(root: &Path) -> Result<Vec<Tool>> {
    let mut tools: HashMap<(String, String), Tool> = HashMap::new();

    let config_path = config::find_boom_config(root);
    let boom_config = if let Some(path) = config_path {
        config::parse_boom_config(&path)?
    } else {
        Default::default()
    };

    if let Some(ref tool_decls) = boom_config.tools {
        for decl in tool_decls {
            let channel = config::resolve_channel(
                decl.channel.as_deref(),
                boom_config.boom.as_ref().and_then(|b| b.channel.as_deref()),
            );

            let tool = Tool::new(
                decl.name.clone(),
                decl.repo.clone(),
                DiscoverySource::ConfigExplicit,
            )
            .with_directory(decl.dir.clone())
            .with_recipe(decl.recipe.clone())
            .with_channel(Some(channel));

            tools.insert((tool.name.clone(), tool.repo.clone()), tool);
        }
    }

    let scan_dirs = config::get_scan_dirs(&boom_config, root);

    for dir in scan_dirs {
        scan_directory(&dir, &mut tools)?;
    }

    Ok(tools.into_values().collect())
}

fn scan_directory(dir: &Path, tools: &mut HashMap<(String, String), Tool>) -> Result<()> {
    if !dir.is_dir() {
        return Ok(());
    }

    let entries =
        fs::read_dir(dir).map_err(|e| crate::error::BabyError::io(dir.display().to_string(), e))?;

    for entry_result in entries {
        let entry =
            entry_result.map_err(|e| crate::error::BabyError::io(dir.display().to_string(), e))?;
        let path = entry.path();

        if entry.file_name() == ".baby.toml" {
            if let Some(parent) = path.parent() {
                if let Ok(git_config_path) = find_git_config(parent) {
                    if let Ok(repo_url) = extract_repo_url(&git_config_path) {
                        let tool_name = parent
                            .file_name()
                            .and_then(|n| n.to_str())
                            .unwrap_or("unknown")
                            .to_string();

                        let key = (tool_name.clone(), repo_url.clone());
                        if !tools.contains_key(&key) {
                            let tool =
                                Tool::new(tool_name, repo_url, DiscoverySource::FilesystemScanned)
                                    .with_channel(Some(Channel::Stable));
                            tools.insert(key, tool);
                        }
                    }
                }
            }
        } else if path.is_dir() && entry.file_name() != ".git" {
            scan_directory(&path, tools)?;
        }
    }

    Ok(())
}

fn find_git_config(dir: &Path) -> Result<std::path::PathBuf> {
    let git_dir = dir.join(".git");
    if git_dir.exists() {
        Ok(git_dir.join("config"))
    } else {
        Err(crate::error::BabyError::new(
            crate::error::ErrorKind::ConfigParse,
            "no .git/config found",
        ))
    }
}

fn extract_repo_url(git_config: &Path) -> Result<String> {
    let content = fs::read_to_string(git_config)
        .map_err(|e| crate::error::BabyError::io(git_config.display().to_string(), e))?;

    for line in content.lines() {
        if line.contains("url =") {
            if let Some(url) = line.split('=').nth(1) {
                return Ok(url.trim().to_string());
            }
        }
    }

    Err(crate::error::BabyError::new(
        crate::error::ErrorKind::ConfigParse,
        "no remote URL found in .git/config",
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_repo_url() {
        let git_config = r#"[core]
    repositoryformatversion = 0
[remote "origin"]
    url = https://github.com/example/repo.git
    fetch = +refs/heads/*:refs/remotes/origin/*
"#;

        let temp_file = tempfile::NamedTempFile::new().unwrap();
        let path = temp_file.path();
        fs::write(path, git_config).unwrap();

        let url = extract_repo_url(path).unwrap();
        assert_eq!(url, "https://github.com/example/repo.git");
    }
}
