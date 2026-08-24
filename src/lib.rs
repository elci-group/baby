// Copyright (c) 2026 sal
// SPDX-License-Identifier: MIT

pub mod boom;
pub mod config;
pub mod error;
pub mod logger;
pub mod recipe;
pub mod versioning;
pub mod workspace;

pub mod styles {
    //! Styling helpers for CLI output.
    use anstyle::{AnsiColor, Style};

    /// Return a set of custom colour styles for clap `--help` output.
    pub fn cli() -> clap::builder::Styles {
        clap::builder::Styles::styled()
            .header(
                Style::new()
                    .bold()
                    .underline()
                    .fg_color(Some(AnsiColor::Green.into())),
            )
            .usage(Style::new().bold().fg_color(Some(AnsiColor::Yellow.into())))
            .literal(Style::new().bold().fg_color(Some(AnsiColor::Cyan.into())))
            .placeholder(Style::new().fg_color(Some(AnsiColor::White.into())))
            .error(Style::new().bold().fg_color(Some(AnsiColor::Red.into())))
            .valid(Style::new().bold().fg_color(Some(AnsiColor::Green.into())))
            .invalid(Style::new().bold().fg_color(Some(AnsiColor::Red.into())))
    }
}

use std::collections::BTreeSet;
use std::env;
use std::fs;
use std::io::{self, IsTerminal, Write};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use crate::error::{BabyError, Result};

/// Initialise logging with a default filter of `info`.
///
/// Call this early in `main()` so that `log::info!`, `log::warn!`, etc. work
/// immediately. The user can override the level via the `RUST_LOG` environment
/// variable (`error`, `warn`, `info`, `debug`, `trace`).
pub fn setup_logging() {
    crate::logger::setup_logging();
}

/// Write a man page for the given clap `Command` to the supplied path.
///
/// The parent directories are created automatically. The file is written in
/// groff format and can be viewed with `man -l <path>`.
pub fn generate_man(cmd: &clap::Command, path: &Path) -> Result<()> {
    let man = clap_mangen::Man::new(cmd.clone());
    let mut buf = Vec::new();
    man.render(&mut buf)
        .map_err(|e| BabyError::io("render man page", e))?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|e| BabyError::io(format!("create directory {}", parent.display()), e))?;
    }
    fs::write(path, buf)
        .map_err(|e| BabyError::io(format!("write man page {}", path.display()), e))?;
    Ok(())
}

/// Configuration controlling how a binary is built and installed.
#[derive(Default)]
pub struct InstallConfig {
    /// Strip debug symbols from the binary before installing.
    pub strip: bool,
    /// Backup an existing binary before overwriting it.
    pub backup: bool,
    /// Restart the matching systemd service after installation.
    pub service: bool,
    /// Run privileged operations with `sudo`.
    pub sudo: bool,
    /// Install into the user's home bin directory instead of the system path.
    pub user: bool,
    /// Show what would happen without mutating the filesystem.
    pub dry_run: bool,
    /// Skip post-install cleanup of Cargo build artefacts.
    pub no_clean: bool,
    /// Override the Cargo target directory.
    pub target_dir: Option<PathBuf>,
    /// Override the installation directory.
    pub install_dir: Option<PathBuf>,
    /// Explicit versioned installation recipe.
    pub recipe: Option<PathBuf>,
}

/// Infer the project name from the current working directory's final component.
pub fn infer_project_name() -> Result<String> {
    let cwd = env::current_dir().map_err(|e| BabyError::io("get current directory", e))?;
    cwd.file_name()
        .and_then(|n| n.to_str())
        .map(|n| n.to_string())
        .ok_or_else(BabyError::project_name)
}

/// Return `~/.local/bin`.
pub fn home_local_bin() -> Result<PathBuf> {
    let home = env::var("HOME").map_err(|_| BabyError::home_not_set())?;
    Ok(PathBuf::from(home).join(".local/bin"))
}

/// Return the XDG runtime directory, falling back to `/tmp`.
pub fn xdg_runtime_dir() -> PathBuf {
    env::var("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("/tmp"))
}

/// Return the XDG state directory, falling back to `~/.local/state` then `/tmp`.
pub fn xdg_state_dir() -> PathBuf {
    env::var("XDG_STATE_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            env::var("HOME")
                .map(|h| PathBuf::from(h).join(".local/state"))
                .unwrap_or_else(|_| PathBuf::from("/tmp"))
        })
}

/// Return the XDG config directory, falling back to `~/.config` then `/tmp`.
pub fn xdg_config_dir() -> PathBuf {
    env::var("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            env::var("HOME")
                .map(|h| PathBuf::from(h).join(".config"))
                .unwrap_or_else(|_| PathBuf::from("/tmp"))
        })
}

/// Path to the birthd PID file.
pub fn pid_file_path() -> PathBuf {
    xdg_runtime_dir().join("birthd.pid")
}

/// Path to the birthd log file.
pub fn log_file_path() -> PathBuf {
    xdg_state_dir().join("birthd.log")
}

/// Directories searched for `.birth.toml` config files.
pub fn birthd_config_dirs() -> Vec<PathBuf> {
    let mut dirs = vec![];
    if let Ok(home) = env::var("HOME") {
        dirs.push(PathBuf::from(&home).join(".config/birth.d"));
    }
    dirs.push(PathBuf::from("/etc/birth.d"));
    dirs
}

/// Read the PID from the PID file, if it exists and is valid.
pub fn read_pid_file() -> Option<u32> {
    fs::read_to_string(pid_file_path())
        .ok()
        .and_then(|s| s.trim().parse().ok())
}

/// Write the daemon PID to the PID file.
pub fn write_pid_file(pid: u32) -> Result<()> {
    let path = pid_file_path();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| {
            BabyError::io(format!("create runtime directory {}", parent.display()), e)
        })?;
    }
    fs::write(&path, pid.to_string())
        .map_err(|e| BabyError::io(format!("write PID file {}", path.display()), e))?;
    Ok(())
}

/// Remove the PID file, silently ignoring missing-file errors.
pub fn remove_pid_file() {
    let _ = fs::remove_file(pid_file_path());
}

/// Check whether a process with the given PID is alive.
pub fn is_process_alive(pid: u32) -> bool {
    // Safety: `kill(pid, 0)` does not send a signal; it only performs process
    // existence checks. The PID is cast from `u32` to `i32`; negative values are
    // invalid for this use and will simply return an error.
    unsafe { libc::kill(pid as i32, 0) == 0 }
}

/// Resolve and validate an installation recipe without executing it.
pub fn resolve_install_recipe(config: &InstallConfig) -> Result<(recipe::InstallRecipe, PathBuf)> {
    let cwd = env::current_dir().map_err(|e| BabyError::io("get current directory", e))?;
    let recipe_path = config
        .recipe
        .clone()
        .unwrap_or_else(|| cwd.join(".baby.toml"));
    if recipe_path.is_file() {
        let root = recipe_path.parent().unwrap_or(&cwd).to_path_buf();
        return Ok((recipe::InstallRecipe::load(&recipe_path)?, root));
    }

    let cargo_manifest = cwd.join("Cargo.toml");
    if cargo_manifest.is_file() {
        // Try single package first
        if let Ok(recipe) = recipe::InstallRecipe::from_cargo_manifest(&cargo_manifest) {
            return Ok((recipe, cwd));
        }

        // If that fails, try workspace detection
        if let Ok(binary_crate_manifest) =
            workspace::find_binary_crate_in_workspace(&cargo_manifest)
        {
            let recipe = recipe::InstallRecipe::from_cargo_manifest(&binary_crate_manifest)?;
            return Ok((recipe, cwd));
        }

        // If both fail, report the original error
        return Err(BabyError::new(
            crate::error::ErrorKind::RecipeInvalid,
            format!(
                "{} is a workspace with no suitable binary crate, or has no [package].name",
                cargo_manifest.display()
            ),
        ));
    }

    Err(BabyError::new(
        crate::error::ErrorKind::RecipeInvalid,
        format!(
            "no installation recipe found in {}; add .baby.toml (schema {}) or a Cargo.toml with [package].name",
            cwd.display(),
            recipe::RECIPE_SCHEMA
        ),
    ))
}

/// Build according to a validated recipe and install its declared artifact.
pub fn build_and_install(config: &InstallConfig) -> Result<()> {
    let (mut recipe, root) = resolve_install_recipe(config)?;
    let project = recipe.binary.clone();
    log::info!(
        "installation recipe resolved: schema={} build_system={:?} binary={project}",
        recipe.schema,
        recipe.build_system
    );

    if let Some(target_dir) = &config.target_dir {
        if recipe.build_system != recipe::BuildSystem::Cargo {
            return Err(BabyError::new(
                crate::error::ErrorKind::RecipeInvalid,
                "--target-dir is only valid for Cargo recipes",
            ));
        }
        recipe.artifact = target_dir.join(&project);
        for command in &mut recipe.commands {
            if command.first().map(String::as_str) == Some("cargo") {
                command.push("--target-dir".into());
                command.push(target_dir.display().to_string());
            }
        }
    }
    let binary_path = root.join(&recipe.artifact);
    log::debug!(
        "recipe root: {root}, binary path: {binary_path}",
        root = root.display(),
        binary_path = binary_path.display()
    );

    let install_dir = if let Some(ref dir) = config.install_dir {
        dir.clone()
    } else if config.user {
        home_local_bin()?
    } else {
        PathBuf::from("/usr/local/bin")
    };

    let install_path = install_dir.join(&project);
    log::debug!(
        "install dir: {install_dir}, install path: {install_path}",
        install_dir = install_dir.display(),
        install_path = install_path.display()
    );

    run_recipe_commands(config, &recipe, &root)?;

    install_ticker("🧱", "build complete; inspecting existing binaries");

    if !config.dry_run && !binary_path.is_file() {
        return Err(BabyError::new(
            crate::error::ErrorKind::RecipeInvalid,
            format!(
                "recipe completed but expected artifact {} was not produced",
                binary_path.display()
            ),
        ));
    }

    if config.strip {
        install_ticker("✂️", "stripping symbols");
        strip_binary(config, &binary_path)?;
    }

    inspect_existing_binary(config, &project, &binary_path, &install_path)?;

    ensure_install_dir(config, &install_dir)?;

    if config.backup {
        backup_existing(config, &install_path)?;
    }

    install_binary(config, &binary_path, &install_path)?;

    install_ticker(
        "✅",
        &format!("installed {project} → {}", install_path.display()),
    );

    if config.service {
        restart_systemd_service(config, &project)?;
    }

    if recipe.build_system == recipe::BuildSystem::Cargo && !config.no_clean {
        clean_build_artifacts(config, &root, config.target_dir.as_deref())?;
    }

    if !config.dry_run {
        log::info!(
            "installed {} -> {}",
            binary_path.display(),
            install_path.display()
        );
    }

    Ok(())
}

fn clean_build_artifacts(
    config: &InstallConfig,
    root: &Path,
    target_dir: Option<&Path>,
) -> Result<()> {
    let target_dir = target_dir
        .map(PathBuf::from)
        .unwrap_or_else(|| root.join("target"));
    let mut cmd = Command::new("deckhand");
    cmd.current_dir(root)
        .arg("clean")
        .arg("--target-dir")
        .arg(&target_dir);

    if config.dry_run {
        log::info!("[dry-run] would run: {}", format_command(&cmd));
        return Ok(());
    }

    install_ticker(
        "🧹",
        &format!("cleaning build artefacts in {}", target_dir.display()),
    );
    let status = cmd
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()
        .map_err(|e| BabyError::io("run deckhand clean", e))?;
    if !status.success() {
        return Err(BabyError::command_failed("deckhand clean", status.code()));
    }
    log::info!("cleaned build artefacts in {}", target_dir.display());
    Ok(())
}

fn install_ticker(emoji: &str, message: &str) {
    // Keep each stage to one concise, scannable line. The emoji acts as the
    // animation/ticker beat without corrupting logs or non-interactive output.
    log::info!("{emoji} · {message}");
}

fn paths_for_binary(name: &str) -> Vec<PathBuf> {
    let mut seen = BTreeSet::new();
    let mut paths = Vec::new();
    for dir in env::var_os("PATH")
        .as_deref()
        .map(env::split_paths)
        .into_iter()
        .flatten()
    {
        let path = dir.join(name);
        if path.is_file() && seen.insert(path.clone()) {
            paths.push(path);
        }
    }
    paths
}

fn binary_version(path: &Path) -> Option<versioning::Version> {
    let output = Command::new(path).arg("--version").output().ok()?;
    if !output.status.success() {
        return None;
    }
    String::from_utf8_lossy(&output.stdout)
        .split_whitespace()
        .find_map(|word| {
            versioning::Version::parse(
                word.trim_matches(|c: char| !c.is_ascii_alphanumeric() && c != '.' && c != '-'),
            )
            .ok()
        })
}

fn inspect_existing_binary(
    config: &InstallConfig,
    name: &str,
    built: &Path,
    install_path: &Path,
) -> Result<()> {
    let existing = paths_for_binary(name);
    if existing.is_empty() {
        install_ticker("🆕", &format!("no existing `{name}` found in PATH"));
    } else {
        for path in &existing {
            let version = binary_version(path)
                .map(|v| v.to_string())
                .unwrap_or_else(|| "unknown".into());
            log::info!("🔎 · existing `{name}`: {version} ({})", path.display());
        }
    }

    if config.dry_run || !install_path.exists() {
        return Ok(());
    }

    let Some(old) = binary_version(install_path) else {
        log::warn!("⚠️  · existing install has no readable --version; proceeding");
        return Ok(());
    };
    let Some(new) = binary_version(built) else {
        log::warn!("⚠️  · built binary has no readable --version; proceeding");
        return Ok(());
    };
    log::info!("📦 · built version: {new}  |  installed version: {old}");
    if new >= old {
        return Ok(());
    }

    log::warn!("🚨 · regression detected: {new} is older than {old}");
    if !io::stdin().is_terminal() {
        log::warn!(
            "⚠️  · non-interactive install; continuing (use --backup to preserve the old binary)"
        );
        return Ok(());
    }
    print!("❓ Install the older version anyway? [y/N] ");
    io::stdout()
        .flush()
        .map_err(|e| BabyError::io("flush confirmation prompt", e))?;
    let mut answer = String::new();
    io::stdin()
        .read_line(&mut answer)
        .map_err(|e| BabyError::io("read confirmation", e))?;
    if answer.trim().eq_ignore_ascii_case("y") || answer.trim().eq_ignore_ascii_case("yes") {
        Ok(())
    } else {
        Err(BabyError::new(
            crate::error::ErrorKind::RecipeInvalid,
            "installation cancelled after version regression",
        ))
    }
}

fn run_recipe_commands(
    config: &InstallConfig,
    recipe: &recipe::InstallRecipe,
    root: &Path,
) -> Result<()> {
    for argv in &recipe.commands {
        let mut cmd = Command::new(&argv[0]);
        cmd.args(&argv[1..]).current_dir(root);
        log::debug!("recipe command: {}", format_command(&cmd));
        if config.dry_run {
            log::info!("[dry-run] would run: {}", format_command(&cmd));
            continue;
        }
        let status = cmd
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit())
            .status()
            .map_err(|e| BabyError::io(format!("run recipe command {}", argv[0]), e))?;
        if !status.success() {
            return Err(BabyError::command_failed(
                format_command(&cmd),
                status.code(),
            ));
        }
    }
    Ok(())
}

fn strip_binary(config: &InstallConfig, binary: &Path) -> Result<()> {
    let mut cmd = Command::new("strip");
    cmd.arg(binary);

    log::debug!("strip command: {}", format_command(&cmd));

    if config.dry_run {
        log::info!("[dry-run] would run: {}", format_command(&cmd));
        return Ok(());
    }

    let status = cmd
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()
        .map_err(|e| BabyError::io("run strip", e))?;

    if !status.success() {
        return Err(BabyError::command_failed("strip", status.code()));
    }
    Ok(())
}

fn ensure_install_dir(config: &InstallConfig, dir: &Path) -> Result<()> {
    if !dir.exists() {
        if config.dry_run {
            log::info!("[dry-run] would create directory {}", dir.display());
            return Ok(());
        }

        fs::create_dir_all(dir)
            .map_err(|e| BabyError::io(format!("create install directory {}", dir.display()), e))?;
    } else {
        log::debug!("install dir already exists: {}", dir.display());
    }
    Ok(())
}

fn backup_existing(config: &InstallConfig, install_path: &Path) -> Result<()> {
    if !install_path.exists() {
        log::debug!(
            "no existing binary at {}, skipping backup",
            install_path.display()
        );
        return Ok(());
    }
    let backup_path = install_path.with_extension("backup");

    if config.dry_run {
        log::info!(
            "[dry-run] would backup {} -> {}",
            install_path.display(),
            backup_path.display()
        );
        return Ok(());
    }

    fs::copy(install_path, &backup_path).map_err(|e| {
        BabyError::io(
            format!(
                "backup {} -> {}",
                install_path.display(),
                backup_path.display()
            ),
            e,
        )
    })?;

    log::info!(
        "backed up {} -> {}",
        install_path.display(),
        backup_path.display()
    );
    Ok(())
}

fn install_binary(config: &InstallConfig, from: &Path, to: &Path) -> Result<()> {
    if config.dry_run {
        log::info!(
            "[dry-run] would install {} -> {} with mode 0o755",
            from.display(),
            to.display()
        );
        return Ok(());
    }

    fs::copy(from, to)
        .map_err(|e| BabyError::io(format!("install {} -> {}", from.display(), to.display()), e))?;

    let mut permissions = fs::metadata(to)
        .map_err(|e| BabyError::io(format!("read permissions of {}", to.display()), e))?
        .permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(to, permissions)
        .map_err(|e| BabyError::io(format!("set permissions of {}", to.display()), e))?;

    Ok(())
}

fn restart_systemd_service(config: &InstallConfig, project: &str) -> Result<()> {
    let service_name = format!("{}.service", project);
    let mut cmd = Command::new("systemctl");
    cmd.arg("restart").arg(&service_name);

    if config.sudo {
        cmd = wrap_sudo(cmd);
    }

    log::debug!("systemctl command: {}", format_command(&cmd));

    if config.dry_run {
        log::info!("[dry-run] would run: {}", format_command(&cmd));
        return Ok(());
    }

    let status = cmd
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()
        .map_err(|e| BabyError::io("run systemctl", e))?;

    if !status.success() {
        return Err(BabyError::command_failed(
            format!("systemctl restart {}", service_name),
            status.code(),
        ));
    }

    log::info!("restarted systemd service {}", service_name);
    Ok(())
}

fn wrap_sudo(cmd: Command) -> Command {
    let mut sudo = Command::new("sudo");
    sudo.arg(cmd.get_program());
    for arg in cmd.get_args() {
        sudo.arg(arg);
    }
    sudo
}

fn format_command(cmd: &Command) -> String {
    let mut s = cmd.get_program().to_string_lossy().to_string();
    for arg in cmd.get_args() {
        s.push(' ');
        s.push_str(&arg.to_string_lossy());
    }
    s
}

/// Run an already-installed binary, forwarding stdout/stderr and exiting with its status code.
pub fn run_binary(path: &Path, args: &[String]) -> Result<()> {
    let mut cmd = Command::new(path);
    cmd.args(args);

    let status = cmd
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()
        .map_err(|e| BabyError::io(format!("run {}", path.display()), e))?;

    std::process::exit(status.code().unwrap_or(1));
}

/// Check for available updates from GitHub releases.
///
/// Fetches the latest release information for the given channel (stable, nightly, or bleeding)
/// from the GitHub repository and compares with the currently installed version.
pub fn check_for_updates(channel: versioning::Channel) -> Result<()> {
    let current = versioning::Version::current();
    let latest = fetch_latest_version(channel)?;

    let channel_name = match channel {
        versioning::Channel::Stable => "stable",
        versioning::Channel::Nightly => "nightly",
        versioning::Channel::Bleeding => "bleeding",
    };

    println!("Current version: {}", current);
    println!("Latest {} version: {}", channel_name, latest);

    match current.cmp(&latest) {
        std::cmp::Ordering::Less => {
            println!(
                "\nUpdate available! A newer version ({}) is available.",
                latest
            );
        }
        std::cmp::Ordering::Equal => {
            println!("\nYou are running the latest {} version.", channel_name);
        }
        std::cmp::Ordering::Greater => {
            println!(
                "\nYou are running a newer version ({}) than the latest {} release ({})",
                current, channel_name, latest
            );
        }
    }

    Ok(())
}

fn fetch_latest_version(channel: versioning::Channel) -> Result<versioning::Version> {
    let repo = "elci-group/baby";
    let url = match channel {
        versioning::Channel::Stable => {
            format!("https://api.github.com/repos/{}/releases/latest", repo)
        }
        versioning::Channel::Nightly => {
            format!("https://api.github.com/repos/{}/releases?per_page=50", repo)
        }
        versioning::Channel::Bleeding => {
            format!("https://api.github.com/repos/{}/releases?per_page=50", repo)
        }
    };

    let output = Command::new("curl")
        .arg("-s")
        .arg(&url)
        .output()
        .map_err(|e| BabyError::io("fetch latest version", e))?;

    if !output.status.success() {
        return Err(BabyError::new(
            crate::error::ErrorKind::VersionCheck,
            format!(
                "failed to fetch version information from GitHub: {}",
                String::from_utf8_lossy(&output.stderr)
            ),
        ));
    }

    let response = String::from_utf8(output.stdout).map_err(|e| {
        BabyError::new(
            crate::error::ErrorKind::VersionCheck,
            format!("invalid UTF-8 in GitHub response: {}", e),
        )
    })?;

    parse_version_from_github_response(&response, channel)
}

fn parse_version_from_github_response(
    response: &str,
    channel: versioning::Channel,
) -> Result<versioning::Version> {
    // Simple JSON parsing for GitHub API responses
    // Extract tag_name field(s) from the response
    let versions: Vec<versioning::Version> = response
        .lines()
        .filter(|line| line.contains("\"tag_name\""))
        .filter_map(|line| {
            let start = line.find('"').map(|i| i + 1)?;
            let end = line[start..].find('"')?;
            let tag = &line[start..start + end];
            versioning::Version::parse(tag).ok()
        })
        .collect();

    if versions.is_empty() {
        return Err(BabyError::new(
            crate::error::ErrorKind::VersionCheck,
            "no valid releases found for the selected channel".to_string(),
        ));
    }

    let latest = match channel {
        versioning::Channel::Stable => versions
            .into_iter()
            .filter(|v| v.prerelease.is_none())
            .max()
            .ok_or_else(|| {
                BabyError::new(
                    crate::error::ErrorKind::VersionCheck,
                    "no stable releases found".to_string(),
                )
            })?,
        versioning::Channel::Nightly => {
            let nightly_max = versions
                .iter()
                .filter(|v| {
                    v.prerelease
                        .as_ref()
                        .map(|p| p.contains("nightly"))
                        .unwrap_or(false)
                })
                .max()
                .cloned();
            nightly_max
                .or_else(|| versions.into_iter().max())
                .ok_or_else(|| {
                    BabyError::new(
                        crate::error::ErrorKind::VersionCheck,
                        "no releases found".to_string(),
                    )
                })?
        }
        versioning::Channel::Bleeding => versions.into_iter().max().ok_or_else(|| {
            BabyError::new(
                crate::error::ErrorKind::VersionCheck,
                "no releases found".to_string(),
            )
        })?,
    };

    Ok(latest)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::ErrorKind;
    use std::sync::Mutex;

    static ENV_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn infer_project_name_from_temp_dir() {
        let dir = tempfile::tempdir().unwrap();
        let original = env::current_dir().unwrap();
        env::set_current_dir(&dir).unwrap();
        let name = infer_project_name().unwrap();
        assert!(!name.is_empty());
        env::set_current_dir(original).unwrap();
    }

    #[test]
    fn home_local_bin_requires_home() {
        let _guard = ENV_LOCK.lock().unwrap();
        unsafe { env::remove_var("HOME") };
        let err = home_local_bin().unwrap_err();
        assert_eq!(err.kind(), ErrorKind::HomeNotSet);
    }

    #[test]
    fn xdg_runtime_dir_uses_env() {
        let _guard = ENV_LOCK.lock().unwrap();
        unsafe { env::set_var("XDG_RUNTIME_DIR", "/custom/run") };
        assert_eq!(xdg_runtime_dir(), PathBuf::from("/custom/run"));
        unsafe { env::remove_var("XDG_RUNTIME_DIR") };
        assert_eq!(xdg_runtime_dir(), PathBuf::from("/tmp"));
    }

    #[test]
    fn xdg_state_dir_falls_back_to_local_state() {
        let _guard = ENV_LOCK.lock().unwrap();
        unsafe { env::remove_var("XDG_STATE_HOME") };
        unsafe { env::set_var("HOME", "/home/test") };
        assert_eq!(xdg_state_dir(), PathBuf::from("/home/test/.local/state"));
    }

    #[test]
    fn pid_file_round_trip() {
        let _guard = ENV_LOCK.lock().unwrap();
        let dir = tempfile::tempdir().unwrap();
        unsafe { env::set_var("XDG_RUNTIME_DIR", dir.path()) };

        assert!(read_pid_file().is_none());
        write_pid_file(12345).unwrap();
        assert_eq!(read_pid_file(), Some(12345));
        remove_pid_file();
        assert!(read_pid_file().is_none());
    }

    #[test]
    fn process_alive_with_current_process() {
        let pid = std::process::id();
        assert!(is_process_alive(pid));
    }

    #[test]
    fn process_not_alive_for_high_pid() {
        // PID 99999 is extremely unlikely to exist on a normal system.
        assert!(!is_process_alive(99999));
    }

    #[test]
    fn install_config_default() {
        let cfg = InstallConfig::default();
        assert!(!cfg.strip);
        assert!(!cfg.backup);
        assert!(!cfg.service);
        assert!(!cfg.sudo);
        assert!(!cfg.user);
        assert!(!cfg.dry_run);
        assert!(cfg.target_dir.is_none());
        assert!(cfg.install_dir.is_none());
        assert!(cfg.recipe.is_none());
    }

    #[test]
    fn format_command_joins_args() {
        let mut cmd = Command::new("cargo");
        cmd.arg("build").arg("--release");
        assert_eq!(format_command(&cmd), "cargo build --release");
    }

    #[test]
    fn command_failed_message_with_code() {
        let err = BabyError::command_failed("strip", Some(1));
        assert_eq!(err.kind(), ErrorKind::CommandFailed);
        assert!(err.message().contains("status 1"));
    }
}
