// Copyright (c) 2026 sal
// SPDX-License-Identifier: MIT

//! Versioned installation recipes.

use std::fs;
use std::path::{Component, Path, PathBuf};

use serde::Deserialize;

use crate::error::{BabyError, ErrorKind, Result};

pub const RECIPE_SCHEMA: &str = "baby.install/v1";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum BuildSystem {
    Cargo,
    Npm,
    Python,
    BinaryRelease,
    Script,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InstallRecipe {
    pub schema: String,
    pub build_system: BuildSystem,
    /// Marks a recipe as installing nothing (e.g. a library-only crate with
    /// no binary target). `binary`/`artifact`/`commands` are ignored and
    /// `build_and_install` no-ops after resolving the recipe.
    #[serde(default)]
    pub library: bool,
    #[serde(default)]
    pub binary: String,
    #[serde(default)]
    pub artifact: PathBuf,
    #[serde(default)]
    pub commands: Vec<Vec<String>>,
    /// Command to run instead of `systemctl restart <binary>.service` after
    /// a successful install, so a running daemon can hand off gracefully
    /// (e.g. spawn-and-elect a new leader) rather than be hard-restarted.
    /// Only used when `--service` is passed. Each argument containing the
    /// literal token `{binary}` has it replaced with the resolved install
    /// path before the command runs.
    #[serde(default)]
    pub restart_command: Option<Vec<String>>,
}

impl InstallRecipe {
    pub fn load(path: &Path) -> Result<Self> {
        let text = fs::read_to_string(path)
            .map_err(|e| BabyError::io(format!("read recipe {}", path.display()), e))?;
        let recipe: Self = toml::from_str(&text)
            .map_err(|e| BabyError::config_parse(path.display().to_string(), e))?;
        recipe.validate(path.parent().unwrap_or_else(|| Path::new(".")))?;
        Ok(recipe)
    }

    pub fn validate(&self, root: &Path) -> Result<()> {
        if self.schema != RECIPE_SCHEMA {
            return Err(BabyError::new(
                ErrorKind::RecipeInvalid,
                format!(
                    "unsupported recipe schema {:?}; expected {RECIPE_SCHEMA:?}",
                    self.schema
                ),
            ));
        }
        if self.library {
            return if root.is_dir() {
                Ok(())
            } else {
                Err(BabyError::new(
                    ErrorKind::RecipeInvalid,
                    format!("recipe root {} is not a directory", root.display()),
                ))
            };
        }
        if self.binary.trim().is_empty() || self.binary.contains(['/', '\\']) {
            return Err(BabyError::new(
                ErrorKind::RecipeInvalid,
                "recipe binary must be a non-empty executable name without path separators",
            ));
        }
        if self.artifact.is_absolute()
            || self
                .artifact
                .components()
                .any(|c| matches!(c, Component::ParentDir))
        {
            return Err(BabyError::new(
                ErrorKind::RecipeInvalid,
                "recipe artifact must be repository-relative and cannot contain `..`",
            ));
        }
        if self.build_system != BuildSystem::BinaryRelease && self.commands.is_empty() {
            return Err(BabyError::new(
                ErrorKind::RecipeInvalid,
                format!(
                    "{:?} recipe must declare at least one command",
                    self.build_system
                ),
            ));
        }
        if self
            .commands
            .iter()
            .any(|command| command.is_empty() || command[0].trim().is_empty())
        {
            return Err(BabyError::new(
                ErrorKind::RecipeInvalid,
                "recipe commands must each contain a non-empty executable",
            ));
        }
        if let Some(restart_command) = &self.restart_command
            && (restart_command.is_empty() || restart_command[0].trim().is_empty())
        {
            return Err(BabyError::new(
                ErrorKind::RecipeInvalid,
                "recipe restart_command must contain a non-empty executable",
            ));
        }
        if !root.is_dir() {
            return Err(BabyError::new(
                ErrorKind::RecipeInvalid,
                format!("recipe root {} is not a directory", root.display()),
            ));
        }
        Ok(())
    }

    /// Compatibility for existing Cargo projects. Package metadata, never the
    /// checkout directory name, determines the expected binary.
    pub fn from_cargo_manifest(path: &Path) -> Result<Self> {
        let text = fs::read_to_string(path)
            .map_err(|e| BabyError::io(format!("read Cargo manifest {}", path.display()), e))?;
        let value: toml::Value = toml::from_str(&text)
            .map_err(|e| BabyError::config_parse(path.display().to_string(), e))?;

        // Prefer explicit [[bin]] name, fall back to package name
        let binary = {
            let mut bin_name = None;

            // Check for explicit [[bin]] section with a name
            if let Some(bins) = value.get("bin") {
                if let Some(arr) = bins.as_array() {
                    if let Some(first_bin) = arr.first() {
                        if let Some(name) = first_bin.get("name").and_then(toml::Value::as_str) {
                            bin_name = Some(name.to_string());
                        }
                    }
                }
            }

            // Fall back to package name
            bin_name.or_else(|| {
                value
                    .get("package")
                    .and_then(|package| package.get("name"))
                    .and_then(toml::Value::as_str)
                    .map(|s| s.to_string())
            })
        }
        .ok_or_else(|| {
            BabyError::new(
                ErrorKind::RecipeInvalid,
                format!("{} has no [package].name or [[bin]].name", path.display()),
            )
        })?;

        // Use relative path from workspace root to manifest
        let manifest_path = if path.parent().is_some() {
            if let Ok(cwd) = std::env::current_dir() {
                if let Ok(rel_path) = path.strip_prefix(&cwd) {
                    rel_path.to_string_lossy().to_string()
                } else {
                    path.to_string_lossy().to_string()
                }
            } else {
                path.to_string_lossy().to_string()
            }
        } else {
            "Cargo.toml".to_string()
        };

        Ok(Self {
            schema: RECIPE_SCHEMA.to_string(),
            build_system: BuildSystem::Cargo,
            library: false,
            artifact: PathBuf::from("target/release").join(&binary),
            commands: vec![vec![
                "cargo".into(),
                "build".into(),
                "--release".into(),
                "--manifest-path".into(),
                manifest_path,
                "--bin".into(),
                binary.clone(),
            ]],
            binary,
            restart_command: None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loads_explicit_non_cargo_recipe() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(".baby.toml");
        fs::write(
            &path,
            "schema = \"baby.install/v1\"\nbuild_system = \"npm\"\nbinary = \"widget\"\nartifact = \"dist/widget\"\ncommands = [[\"npm\", \"ci\"], [\"npm\", \"run\", \"build\"]]\n",
        )
        .unwrap();
        let recipe = InstallRecipe::load(&path).unwrap();
        assert_eq!(recipe.build_system, BuildSystem::Npm);
        assert_eq!(recipe.binary, "widget");
        assert_eq!(recipe.commands.len(), 2);
    }

    #[test]
    fn loads_library_recipe_without_binary_or_commands() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(".baby.toml");
        fs::write(
            &path,
            "schema = \"baby.install/v1\"\nbuild_system = \"cargo\"\nlibrary = true\n",
        )
        .unwrap();
        let recipe = InstallRecipe::load(&path).unwrap();
        assert!(recipe.library);
        assert!(recipe.binary.is_empty());
        assert!(recipe.commands.is_empty());
    }

    #[test]
    fn rejects_unknown_schema_before_execution() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(".baby.toml");
        fs::write(
            &path,
            "schema = \"baby.install/v2\"\nbuild_system = \"script\"\nbinary = \"widget\"\nartifact = \"out/widget\"\ncommands = [[\"make\"]]\n",
        )
        .unwrap();
        let err = InstallRecipe::load(&path).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::RecipeInvalid);
    }

    #[test]
    fn cargo_fallback_uses_package_name_not_directory_name() {
        let dir = tempfile::tempdir().unwrap();
        let manifest = dir.path().join("Cargo.toml");
        fs::write(
            &manifest,
            "[package]\nname = \"actual-bin\"\nversion = \"0.1.0\"\n",
        )
        .unwrap();
        let recipe = InstallRecipe::from_cargo_manifest(&manifest).unwrap();
        assert_eq!(recipe.binary, "actual-bin");
        assert_eq!(recipe.artifact, PathBuf::from("target/release/actual-bin"));
    }

    #[test]
    fn rejects_artifact_escape() {
        let recipe = InstallRecipe {
            schema: RECIPE_SCHEMA.to_string(),
            build_system: BuildSystem::Script,
            library: false,
            binary: "x".into(),
            artifact: PathBuf::from("../x"),
            commands: vec![vec!["make".into()]],
            restart_command: None,
        };
        assert!(recipe.validate(Path::new(".")).is_err());
    }

    #[test]
    fn loads_recipe_with_restart_command() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(".baby.toml");
        fs::write(
            &path,
            "schema = \"baby.install/v1\"\nbuild_system = \"script\"\nbinary = \"widget\"\nartifact = \"out/widget\"\ncommands = [[\"make\"]]\nrestart_command = [\"widget-cli\", \"shark\", \"upgrade\", \"{binary}\"]\n",
        )
        .unwrap();
        let recipe = InstallRecipe::load(&path).unwrap();
        assert_eq!(
            recipe.restart_command,
            Some(vec![
                "widget-cli".to_string(),
                "shark".to_string(),
                "upgrade".to_string(),
                "{binary}".to_string(),
            ])
        );
    }

    #[test]
    fn rejects_empty_restart_command() {
        let recipe = InstallRecipe {
            schema: RECIPE_SCHEMA.to_string(),
            build_system: BuildSystem::Script,
            library: false,
            binary: "x".into(),
            artifact: PathBuf::from("out/x"),
            commands: vec![vec!["make".into()]],
            restart_command: Some(vec![]),
        };
        assert!(recipe.validate(Path::new(".")).is_err());
    }
}
