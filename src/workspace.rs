// Copyright (c) 2026 sal
// SPDX-License-Identifier: MIT

//! Rust workspace detection and binary crate discovery.

use std::fs;
use std::path::{Path, PathBuf};

use crate::error::{BabyError, Result};

/// Detects if a Cargo.toml is a workspace and finds the binary crate.
pub fn find_binary_crate_in_workspace(manifest: &Path) -> Result<PathBuf> {
    let text = fs::read_to_string(manifest)
        .map_err(|e| BabyError::io(format!("read Cargo manifest {}", manifest.display()), e))?;
    let value: toml::Value = toml::from_str(&text)
        .map_err(|e| BabyError::config_parse(manifest.display().to_string(), e))?;

    // Check if this is a workspace
    let workspace = match value.get("workspace") {
        Some(ws) => ws,
        None => return Err(BabyError::new(
            crate::error::ErrorKind::RecipeInvalid,
            format!("{} is not a workspace", manifest.display()),
        )),
    };

    // Get workspace members
    let members = match workspace.get("members") {
        Some(m) => m.as_array().ok_or_else(|| BabyError::new(
            crate::error::ErrorKind::RecipeInvalid,
            "workspace.members must be an array".to_string(),
        ))?,
        None => return Err(BabyError::new(
            crate::error::ErrorKind::RecipeInvalid,
            "workspace has no members".to_string(),
        )),
    };

    let root_dir = manifest.parent().unwrap_or_else(|| Path::new("."));
    let mut binary_crates = Vec::new();

    for member in members {
        let member_path = member.as_str().ok_or_else(|| BabyError::new(
            crate::error::ErrorKind::RecipeInvalid,
            "workspace member must be a string".to_string(),
        ))?;

        let crate_manifest = root_dir.join(member_path).join("Cargo.toml");
        if !crate_manifest.is_file() {
            continue;
        }

        if is_binary_crate(&crate_manifest)? {
            binary_crates.push(crate_manifest);
        }
    }

    // Prefer a single binary crate; if multiple exist, prefer one matching the workspace name
    match binary_crates.len() {
        0 => Err(BabyError::new(
            crate::error::ErrorKind::RecipeInvalid,
            "workspace contains no binary crates".to_string(),
        )),
        1 => Ok(binary_crates.into_iter().next().unwrap()),
        _ => {
            // Multiple binary crates: try to find one matching the workspace name
            let workspace_name = value
                .get("package")
                .and_then(|p| p.get("name"))
                .and_then(toml::Value::as_str)
                .unwrap_or_else(|| {
                    root_dir
                        .file_name()
                        .and_then(|n| n.to_str())
                        .unwrap_or("workspace")
                });

            for crate_manifest in &binary_crates {
                if let Ok(binary_name) = crate_binary_name(crate_manifest) {
                    if binary_name == workspace_name {
                        return Ok(crate_manifest.clone());
                    }
                }
            }

            // If no match found, prefer the one in a dir matching workspace name
            for crate_manifest in &binary_crates {
                if let Some(parent) = crate_manifest.parent().and_then(|p| p.file_name()) {
                    if parent.to_string_lossy() == workspace_name {
                        return Ok(crate_manifest.clone());
                    }
                }
            }

            // Last resort: use the first one
            Ok(binary_crates.into_iter().next().unwrap())
        }
    }
}

/// Checks if a Cargo.toml defines a binary crate (has `[[bin]]` or is a simple binary package).
fn is_binary_crate(manifest: &Path) -> Result<bool> {
    let text = fs::read_to_string(manifest)
        .map_err(|e| BabyError::io(format!("read Cargo manifest {}", manifest.display()), e))?;
    let value: toml::Value = toml::from_str(&text)
        .map_err(|e| BabyError::config_parse(manifest.display().to_string(), e))?;

    // Check for explicit [[bin]] sections
    if value.get("bin").is_some() {
        return Ok(true);
    }

    // Check if it's a package with a main.rs (implicit binary)
    if let Some(_package) = value.get("package") {
        let src_dir = manifest
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .join("src");
        if src_dir.join("main.rs").is_file() {
            return Ok(true);
        }
    }

    Ok(false)
}

/// Extracts the binary name from a Cargo.toml.
fn crate_binary_name(manifest: &Path) -> Result<String> {
    let text = fs::read_to_string(manifest)
        .map_err(|e| BabyError::io(format!("read Cargo manifest {}", manifest.display()), e))?;
    let value: toml::Value = toml::from_str(&text)
        .map_err(|e| BabyError::config_parse(manifest.display().to_string(), e))?;

    // Prefer explicit [[bin]] name
    if let Some(bins) = value.get("bin").and_then(toml::Value::as_array) {
        if let Some(first_bin) = bins.first() {
            if let Some(name) = first_bin.get("name").and_then(toml::Value::as_str) {
                return Ok(name.to_string());
            }
        }
    }

    // Fall back to package name
    value
        .get("package")
        .and_then(|package| package.get("name"))
        .and_then(toml::Value::as_str)
        .map(|s| s.to_string())
        .ok_or_else(|| {
            BabyError::new(
                crate::error::ErrorKind::RecipeInvalid,
                format!("{} has no package name or bin name", manifest.display()),
            )
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn finds_binary_crate_in_workspace() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();

        // Create workspace root
        fs::write(
            root.join("Cargo.toml"),
            "[workspace]\nmembers = [\"crates/bin-app\", \"crates/lib-module\"]\n",
        )
        .unwrap();

        // Create binary crate
        fs::create_dir_all(root.join("crates/bin-app/src")).unwrap();
        fs::write(
            root.join("crates/bin-app/Cargo.toml"),
            "[package]\nname = \"bin-app\"\nversion = \"0.1.0\"\n",
        )
        .unwrap();
        fs::write(root.join("crates/bin-app/src/main.rs"), "fn main() {}").unwrap();

        // Create library crate
        fs::create_dir_all(root.join("crates/lib-module/src")).unwrap();
        fs::write(
            root.join("crates/lib-module/Cargo.toml"),
            "[package]\nname = \"lib-module\"\nversion = \"0.1.0\"\n",
        )
        .unwrap();
        fs::write(root.join("crates/lib-module/src/lib.rs"), "").unwrap();

        let result = find_binary_crate_in_workspace(&root.join("Cargo.toml")).unwrap();
        assert_eq!(result.file_name(), Some(std::ffi::OsStr::new("Cargo.toml")));
        assert!(result.parent().unwrap().ends_with("crates/bin-app"));
    }

    #[test]
    fn detects_non_workspace() {
        let dir = tempfile::tempdir().unwrap();
        let manifest = dir.path().join("Cargo.toml");
        fs::write(
            &manifest,
            "[package]\nname = \"single-app\"\nversion = \"0.1.0\"\n",
        )
        .unwrap();

        let err = find_binary_crate_in_workspace(&manifest).unwrap_err();
        assert!(err.message().contains("not a workspace"));
    }

    #[test]
    fn rejects_workspace_with_no_binaries() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();

        fs::write(
            root.join("Cargo.toml"),
            "[workspace]\nmembers = [\"crates/lib-only\"]\n",
        )
        .unwrap();

        fs::create_dir_all(root.join("crates/lib-only/src")).unwrap();
        fs::write(
            root.join("crates/lib-only/Cargo.toml"),
            "[package]\nname = \"lib-only\"\nversion = \"0.1.0\"\n",
        )
        .unwrap();
        fs::write(root.join("crates/lib-only/src/lib.rs"), "").unwrap();

        let err = find_binary_crate_in_workspace(&root.join("Cargo.toml")).unwrap_err();
        assert!(err.message().contains("no binary crates"));
    }
}
