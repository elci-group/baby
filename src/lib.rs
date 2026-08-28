// Copyright (c) 2026 sal
// SPDX-License-Identifier: MIT

pub mod boar;
pub mod boom;
pub mod config;
pub mod error;
pub mod lock;
pub mod logger;
pub mod recipe;
pub mod terminal_ui;
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
use std::time::Instant;

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
    /// Never delegate a failed build to `boar` for RAM/disk recovery.
    pub no_boar: bool,
    /// Override the Cargo target directory.
    pub target_dir: Option<PathBuf>,
    /// Override the installation directory.
    pub install_dir: Option<PathBuf>,
    /// Explicit versioned installation recipe.
    pub recipe: Option<PathBuf>,
    /// Skip locksmithd coordination for this run.
    pub no_lock: bool,
    /// Seconds to wait for a contended locksmith lease.
    pub lock_timeout_secs: u64,
    /// Seconds to request for the locksmith lease duration.
    pub lock_lease_secs: u64,
}

impl Default for InstallConfig {
    fn default() -> Self {
        Self {
            strip: false,
            backup: false,
            service: false,
            sudo: false,
            user: false,
            dry_run: false,
            no_clean: false,
            no_boar: false,
            target_dir: None,
            install_dir: None,
            recipe: None,
            no_lock: false,
            lock_timeout_secs: crate::lock::DEFAULT_TIMEOUT_SECS,
            lock_lease_secs: crate::lock::DEFAULT_LEASE_SECS,
        }
    }
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
    let started_at = Instant::now();
    let (mut recipe, root) = resolve_install_recipe(config)?;
    if recipe.library {
        log::info!(
            "{} is a library-only recipe (schema={}); nothing to build or install",
            root.display(),
            recipe.schema
        );
        return Ok(());
    }
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
    let expected_binary_path = root.join(&recipe.artifact);
    log::debug!(
        "recipe root: {root}, binary path: {binary_path}",
        root = root.display(),
        binary_path = expected_binary_path.display()
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

    let mut animation = terminal_ui::InstallAnimation::start(&project, !config.dry_run);

    let _lock_guard = if config.no_lock || config.dry_run {
        None
    } else {
        install_ticker(animation.as_ref(), "🔐", "acquiring project lease");
        lock::acquire_build_lock(
            &root,
            &project,
            config.lock_timeout_secs,
            config.lock_lease_secs,
        )?
    };

    let outcome = run_recipe_commands(config, animation.as_ref(), &recipe, &root)?;

    install_ticker(
        animation.as_ref(),
        "🧱",
        "build complete; inspecting existing binaries",
    );

    let mut binary_path = expected_binary_path.clone();
    let mut boar_managed_target: Option<PathBuf> = None;
    if let BuildOutcome::Recovered { target_dir } = &outcome
        && !config.dry_run
        && !binary_path.is_file()
    {
        let relocated = target_dir
            .as_deref()
            .and_then(|dir| relocated_artifact_path(&recipe.artifact, dir));
        match relocated {
            Some(candidate) if candidate.is_file() => {
                install_ticker(
                    animation.as_ref(),
                    "🐗",
                    &format!(
                        "artifact recovered under boar-managed placement: {}",
                        candidate.display()
                    ),
                );
                boar_managed_target = target_dir.clone();
                binary_path = candidate;
            }
            _ => {
                return Err(BabyError::new(
                    crate::error::ErrorKind::RecipeInvalid,
                    format!(
                        "boar recovered the build, but the artifact could not be located \
                         (expected {}); run `boar target-dir` to inspect placement or pass \
                         --target-dir explicitly",
                        expected_binary_path.display()
                    ),
                ));
            }
        }
    }

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
        install_ticker(animation.as_ref(), "✂️", "stripping symbols");
        strip_binary(config, animation.as_ref(), &binary_path)?;
    }

    inspect_existing_binary(
        config,
        animation.as_ref(),
        &project,
        &binary_path,
        &install_path,
    )?;

    ensure_install_dir(config, &install_dir)?;

    if config.backup {
        backup_existing(config, &install_path)?;
    }

    install_binary(config, &binary_path, &install_path)?;

    install_ticker(
        animation.as_ref(),
        "✅",
        &format!("installed {project} → {}", install_path.display()),
    );

    if config.service {
        run_post_install_restart(
            config,
            animation.as_ref(),
            &project,
            recipe.restart_command.as_deref(),
            &install_path,
        )?;
    }

    let cleanup_ran = recipe.build_system == recipe::BuildSystem::Cargo
        && !config.no_clean
        && !config.dry_run
        && boar_managed_target.is_none();
    if cleanup_ran {
        clean_build_artifacts(animation.as_ref(), &root, config.target_dir.as_deref())?;
    } else if boar_managed_target.is_some() {
        install_ticker(
            animation.as_ref(),
            "🐗",
            "build artefacts remain under boar; run `boar clean` there if needed",
        );
    }

    if !config.dry_run {
        log::info!(
            "installed {} -> {}",
            binary_path.display(),
            install_path.display()
        );
    }

    if let Some(animation) = animation.as_mut() {
        let artifact_bytes = fs::metadata(&install_path)
            .map(|metadata| metadata.len())
            .unwrap_or(0);
        animation.finish(&terminal_ui::InstallTelemetry {
            project: &project,
            install_path: &install_path,
            elapsed: started_at.elapsed(),
            build_commands: recipe.commands.len(),
            artifact_bytes,
            cleanup_ran,
            dry_run: config.dry_run,
        });
    }

    Ok(())
}

fn clean_build_artifacts(
    animation: Option<&terminal_ui::InstallAnimation>,
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

    install_ticker(
        animation,
        "🧹",
        &format!("cleaning build artefacts in {}", target_dir.display()),
    );
    if let Some(a) = animation {
        a.pause();
    }
    let status = cmd
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()
        .map_err(|e| BabyError::io("run deckhand clean", e))?;
    if let Some(a) = animation {
        a.resume();
    }
    if !status.success() {
        return Err(BabyError::command_failed("deckhand clean", status.code()));
    }
    log::info!("cleaned build artefacts in {}", target_dir.display());
    Ok(())
}

/// Log one concise, scannable install-stage line and, when the animated
/// baby is running, update its live stage/elapsed status too. The emoji
/// acts as the ticker beat without corrupting logs or non-interactive
/// output; `log::info!` itself erases any drawn animation frame first (see
/// `logger::RENDER`), so this can never interleave with the art block.
fn install_ticker(animation: Option<&terminal_ui::InstallAnimation>, emoji: &str, message: &str) {
    if let Some(a) = animation {
        a.set_stage(message);
    }
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
    animation: Option<&terminal_ui::InstallAnimation>,
    name: &str,
    built: &Path,
    install_path: &Path,
) -> Result<()> {
    let existing = paths_for_binary(name);
    if existing.is_empty() {
        install_ticker(
            animation,
            "🆕",
            &format!("no existing `{name}` found in PATH"),
        );
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
    if let Some(a) = animation {
        a.pause();
    }
    print!("❓ Install the older version anyway? [y/N] ");
    io::stdout()
        .flush()
        .map_err(|e| BabyError::io("flush confirmation prompt", e))?;
    let mut answer = String::new();
    io::stdin()
        .read_line(&mut answer)
        .map_err(|e| BabyError::io("read confirmation", e))?;
    if let Some(a) = animation {
        a.resume();
    }
    if answer.trim().eq_ignore_ascii_case("y") || answer.trim().eq_ignore_ascii_case("yes") {
        Ok(())
    } else {
        Err(BabyError::new(
            crate::error::ErrorKind::RecipeInvalid,
            "installation cancelled after version regression",
        ))
    }
}

/// Outcome of running a recipe's build commands.
enum BuildOutcome {
    /// Every command succeeded on its first attempt.
    Normal,
    /// A retryable command failed and `boar` recovered it. `target_dir` is
    /// where BOAR reports it placed the build, if that could be determined.
    Recovered { target_dir: Option<PathBuf> },
}

/// Map a recipe artifact path (e.g. `target/release/widget`) onto a
/// different target-dir root (e.g. a boar-managed RAM path), by keeping
/// everything after the leading `target` component. Recipes with a
/// differently named or nested artifact path can't be remapped this way and
/// return `None`.
fn relocated_artifact_path(recipe_artifact: &Path, target_dir: &Path) -> Option<PathBuf> {
    let suffix = recipe_artifact.strip_prefix("target").ok()?;
    Some(target_dir.join(suffix))
}

fn run_recipe_commands(
    config: &InstallConfig,
    animation: Option<&terminal_ui::InstallAnimation>,
    recipe: &recipe::InstallRecipe,
    root: &Path,
) -> Result<BuildOutcome> {
    let mut outcome = BuildOutcome::Normal;
    for argv in &recipe.commands {
        let mut cmd = Command::new(&argv[0]);
        cmd.args(&argv[1..]).current_dir(root);
        if config.dry_run {
            log::info!("[dry-run] would run: {}", format_command(&cmd));
            continue;
        }
        install_ticker(
            animation,
            "🔨",
            &format!("running: {}", format_command(&cmd)),
        );
        if let Some(a) = animation {
            a.pause();
        }
        let status = cmd
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit())
            .status()
            .map_err(|e| BabyError::io(format!("run recipe command {}", argv[0]), e))?;
        if let Some(a) = animation {
            a.resume();
        }
        if status.success() {
            continue;
        }
        if let Some(target_dir) = attempt_boar_recovery(config, animation, recipe, root, argv)? {
            outcome = BuildOutcome::Recovered { target_dir };
            continue;
        }
        return Err(BabyError::command_failed(
            format_command(&cmd),
            status.code(),
        ));
    }
    Ok(outcome)
}

/// Retry a failed, retryable Cargo command through `boar` when the failure
/// looks plausibly storage-related. Returns `Ok(Some(target_dir))` on a
/// successful recovery (BOAR's reported target dir, if resolvable),
/// `Ok(None)` when recovery was not attempted or did not help (the caller
/// should report the original failure), and `Err` only for I/O failures
/// while trying to invoke `boar` itself.
fn attempt_boar_recovery(
    config: &InstallConfig,
    animation: Option<&terminal_ui::InstallAnimation>,
    recipe: &recipe::InstallRecipe,
    root: &Path,
    argv: &[String],
) -> Result<Option<Option<PathBuf>>> {
    if config.no_boar || config.target_dir.is_some() {
        return Ok(None);
    }
    if recipe.build_system != recipe::BuildSystem::Cargo {
        return Ok(None);
    }
    let Some(boar_argv) = boar::rewrite_for_boar(argv) else {
        return Ok(None);
    };
    if !boar::boar_available() || !boar::plausibly_storage_related(root) {
        return Ok(None);
    }

    install_ticker(
        animation,
        "🐗",
        "local build storage under pressure; retrying via boar for adaptive placement",
    );
    let mut cmd = Command::new(&boar_argv[0]);
    cmd.args(&boar_argv[1..]).current_dir(root);
    log::debug!("boar recovery command: {}", format_command(&cmd));
    if let Some(a) = animation {
        a.pause();
    }
    let status = cmd
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()
        .map_err(|e| BabyError::io("run boar recovery build", e))?;
    if let Some(a) = animation {
        a.resume();
    }
    if !status.success() {
        log::warn!(
            "boar recovery build also failed (status {:?}); reporting the original failure",
            status.code()
        );
        return Ok(None);
    }
    install_ticker(animation, "✅", "build recovered via boar");
    Ok(Some(boar::resolved_target_dir(root)))
}

fn strip_binary(
    config: &InstallConfig,
    animation: Option<&terminal_ui::InstallAnimation>,
    binary: &Path,
) -> Result<()> {
    let mut cmd = Command::new("strip");
    cmd.arg(binary);

    log::debug!("strip command: {}", format_command(&cmd));

    if config.dry_run {
        log::info!("[dry-run] would run: {}", format_command(&cmd));
        return Ok(());
    }

    if let Some(a) = animation {
        a.pause();
    }
    let status = cmd
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()
        .map_err(|e| BabyError::io("run strip", e))?;
    if let Some(a) = animation {
        a.resume();
    }

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

/// Install `from` at `to` by staging a copy alongside the destination and
/// `rename`-ing it into place, rather than truncating the destination file
/// in place. A running process keeps its old binary mapped for execution;
/// truncating that inode (what `fs::copy` does when `to` already exists)
/// fails with ETXTBSY. `rename` only repoints the directory entry, so it
/// succeeds even while the old inode is still executing — the running
/// process keeps serving from the orphaned old inode until it exits or is
/// restarted (e.g. by the systemd restart baby issues right after this).
fn install_binary(config: &InstallConfig, from: &Path, to: &Path) -> Result<()> {
    if config.dry_run {
        log::info!(
            "[dry-run] would install {} -> {} with mode 0o755",
            from.display(),
            to.display()
        );
        return Ok(());
    }

    let parent = to.parent().ok_or_else(|| {
        BabyError::new(
            crate::error::ErrorKind::Io,
            format!("install path {} has no parent directory", to.display()),
        )
    })?;

    let mut staged = tempfile::Builder::new()
        .prefix(&format!(
            ".{}.",
            to.file_name().and_then(|n| n.to_str()).unwrap_or("baby-install")
        ))
        .tempfile_in(parent)
        .map_err(|e| BabyError::io(format!("create temp file in {}", parent.display()), e))?;

    let mut source = fs::File::open(from)
        .map_err(|e| BabyError::io(format!("open {}", from.display()), e))?;
    io::copy(&mut source, staged.as_file_mut())
        .map_err(|e| BabyError::io(format!("copy {} -> temp file", from.display()), e))?;

    let mut permissions = staged
        .as_file()
        .metadata()
        .map_err(|e| BabyError::io("read temp file metadata", e))?
        .permissions();
    permissions.set_mode(0o755);
    staged
        .as_file()
        .set_permissions(permissions)
        .map_err(|e| BabyError::io("set permissions of temp file", e))?;

    staged.persist(to).map_err(|e| {
        let busy = processes_executing(to);
        let mut context = format!("install {} -> {}", from.display(), to.display());
        if !busy.is_empty() {
            let who = busy
                .iter()
                .map(|(pid, comm)| format!("{comm}(pid {pid})"))
                .collect::<Vec<_>>()
                .join(", ");
            context = format!("{context} (currently running: {who})");
        }
        BabyError::io(context, e.error)
    })?;

    Ok(())
}

/// Return `(pid, comm)` for every process currently executing `path`, by
/// scanning `/proc/*/exe`. Used only to enrich error messages; failures to
/// read `/proc` are silently ignored since this is best-effort diagnostics.
#[cfg(target_os = "linux")]
fn processes_executing(path: &Path) -> Vec<(u32, String)> {
    let Ok(canonical) = fs::canonicalize(path) else {
        return Vec::new();
    };
    let Ok(entries) = fs::read_dir("/proc") else {
        return Vec::new();
    };
    let mut hits = Vec::new();
    for entry in entries.flatten() {
        let Some(pid) = entry.file_name().to_str().and_then(|s| s.parse::<u32>().ok()) else {
            continue;
        };
        let Ok(exe_target) = fs::read_link(entry.path().join("exe")) else {
            continue;
        };
        if exe_target == canonical {
            let comm = fs::read_to_string(entry.path().join("comm"))
                .unwrap_or_else(|_| "?".into());
            hits.push((pid, comm.trim().to_string()));
        }
    }
    hits
}

#[cfg(not(target_os = "linux"))]
fn processes_executing(_path: &Path) -> Vec<(u32, String)> {
    Vec::new()
}

/// Run the post-install restart step: a recipe-supplied `restart_command`
/// (for daemons that hand off gracefully, e.g. kaptaind's shark-stating)
/// when present, otherwise the default hard `systemctl restart`.
fn run_post_install_restart(
    config: &InstallConfig,
    animation: Option<&terminal_ui::InstallAnimation>,
    project: &str,
    restart_command: Option<&[String]>,
    install_path: &Path,
) -> Result<()> {
    match restart_command {
        Some(argv) => run_restart_hook(config, animation, argv, install_path),
        None => restart_systemd_service(config, animation, project),
    }
}

/// Run a recipe-supplied restart hook instead of `systemctl restart`, so a
/// running daemon can hand off leadership/state gracefully before the old
/// process is replaced, rather than being hard-killed and relaunched.
/// Any argument equal to the literal token `{binary}` is replaced with the
/// resolved install path.
fn run_restart_hook(
    config: &InstallConfig,
    animation: Option<&terminal_ui::InstallAnimation>,
    argv: &[String],
    install_path: &Path,
) -> Result<()> {
    let install_path_str = install_path.display().to_string();
    let resolved: Vec<String> = argv
        .iter()
        .map(|arg| {
            if arg == "{binary}" {
                install_path_str.clone()
            } else {
                arg.clone()
            }
        })
        .collect();

    let mut cmd = Command::new(&resolved[0]);
    cmd.args(&resolved[1..]);

    if config.sudo {
        cmd = wrap_sudo(cmd);
    }

    if config.dry_run {
        log::info!("[dry-run] would run: {}", format_command(&cmd));
        return Ok(());
    }

    install_ticker(
        animation,
        "🦈",
        &format!("running graceful restart hook: {}", format_command(&cmd)),
    );
    if let Some(a) = animation {
        a.pause();
    }
    let status = cmd
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()
        .map_err(|e| BabyError::io("run restart hook", e))?;
    if let Some(a) = animation {
        a.resume();
    }

    if !status.success() {
        return Err(BabyError::command_failed(format_command(&cmd), status.code()));
    }

    log::info!("ran graceful restart hook for {}", install_path.display());
    Ok(())
}

fn restart_systemd_service(
    config: &InstallConfig,
    animation: Option<&terminal_ui::InstallAnimation>,
    project: &str,
) -> Result<()> {
    let service_name = format!("{}.service", project);
    let mut cmd = Command::new("systemctl");
    cmd.arg("restart").arg(&service_name);

    if config.sudo {
        cmd = wrap_sudo(cmd);
    }

    if config.dry_run {
        log::info!("[dry-run] would run: {}", format_command(&cmd));
        return Ok(());
    }

    install_ticker(
        animation,
        "🔁",
        &format!("running: {}", format_command(&cmd)),
    );
    if let Some(a) = animation {
        a.pause();
    }
    let status = cmd
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()
        .map_err(|e| BabyError::io("run systemctl", e))?;
    if let Some(a) = animation {
        a.resume();
    }

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
    fn run_restart_hook_substitutes_binary_placeholder() {
        let dir = tempfile::tempdir().unwrap();
        let output = dir.path().join("out.txt");
        let install_path = dir.path().join("installed-widget");
        let argv = vec![
            "/bin/sh".to_string(),
            "-c".to_string(),
            format!("printf '%s' \"$1\" > {}", output.display()),
            "_".to_string(),
            "{binary}".to_string(),
        ];
        let config = InstallConfig::default();
        run_restart_hook(&config, None, &argv, &install_path).unwrap();
        let written = fs::read_to_string(&output).unwrap();
        assert_eq!(written, install_path.display().to_string());
    }

    #[test]
    fn run_restart_hook_skips_execution_in_dry_run() {
        let dir = tempfile::tempdir().unwrap();
        let output = dir.path().join("out.txt");
        let install_path = dir.path().join("installed-widget");
        let argv = vec![
            "/bin/sh".to_string(),
            "-c".to_string(),
            format!("touch {}", output.display()),
        ];
        let config = InstallConfig {
            dry_run: true,
            ..InstallConfig::default()
        };
        run_restart_hook(&config, None, &argv, &install_path).unwrap();
        assert!(!output.exists());
    }

    #[test]
    fn run_post_install_restart_prefers_recipe_hook_over_systemd() {
        // With a restart_command present, dispatch must run the hook and
        // never shell out to systemctl (which would fail/hang in a test
        // sandbox with no such service).
        let dir = tempfile::tempdir().unwrap();
        let output = dir.path().join("out.txt");
        let install_path = dir.path().join("installed-widget");
        let argv = vec!["/bin/sh".to_string(), "-c".to_string(), format!("touch {}", output.display())];
        let config = InstallConfig::default();
        run_post_install_restart(&config, None, "widget", Some(&argv), &install_path).unwrap();
        assert!(output.exists());
    }

    #[test]
    fn install_binary_overwrites_a_running_target() {
        // Regression test: fs::copy's in-place truncate fails with ETXTBSY
        // (os error 26) when `to` is currently executing. install_binary
        // must succeed anyway by staging + renaming instead of truncating.
        let dir = tempfile::tempdir().unwrap();
        let old = dir.path().join("target");
        let new = dir.path().join("source");
        fs::copy("/bin/sleep", &old).unwrap();
        fs::copy("/bin/sleep", &new).unwrap();
        let mut perms = fs::metadata(&old).unwrap().permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&old, perms).unwrap();

        let mut child = Command::new(&old).arg("2").spawn().unwrap();

        let config = InstallConfig::default();
        let result = install_binary(&config, &new, &old);

        child.kill().ok();
        child.wait().ok();

        result.unwrap();
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
        assert!(!cfg.no_boar);
        assert!(!cfg.no_lock);
        assert_eq!(cfg.lock_timeout_secs, crate::lock::DEFAULT_TIMEOUT_SECS);
        assert_eq!(cfg.lock_lease_secs, crate::lock::DEFAULT_LEASE_SECS);
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

    #[test]
    fn relocated_artifact_path_remaps_target_prefix() {
        let recipe_artifact = PathBuf::from("target/release/widget");
        let ram_target_dir = PathBuf::from("/dev/shm/boar/widget-abc123");
        assert_eq!(
            relocated_artifact_path(&recipe_artifact, &ram_target_dir),
            Some(PathBuf::from("/dev/shm/boar/widget-abc123/release/widget"))
        );
    }

    #[test]
    fn relocated_artifact_path_none_for_non_standard_layout() {
        let recipe_artifact = PathBuf::from("dist/widget");
        let ram_target_dir = PathBuf::from("/dev/shm/boar/widget-abc123");
        assert_eq!(
            relocated_artifact_path(&recipe_artifact, &ram_target_dir),
            None
        );
    }

    #[test]
    fn boar_recovery_is_not_attempted_without_storage_pressure() {
        // A freshly created temp directory is never plausibly out of space,
        // so recovery must not run cargo/boar at all here; `no_boar` alone
        // is enough to prove the gate short-circuits before any subprocess.
        let dir = tempfile::tempdir().unwrap();
        let recipe = recipe::InstallRecipe {
            schema: recipe::RECIPE_SCHEMA.to_string(),
            build_system: recipe::BuildSystem::Cargo,
            library: false,
            binary: "widget".into(),
            artifact: PathBuf::from("target/release/widget"),
            commands: vec![vec!["cargo".into(), "build".into(), "--release".into()]],
            restart_command: None,
        };
        let config = InstallConfig::default();
        let result =
            attempt_boar_recovery(&config, None, &recipe, dir.path(), &recipe.commands[0]).unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn boar_recovery_is_skipped_when_disabled_or_target_dir_overridden() {
        let dir = tempfile::tempdir().unwrap();
        let recipe = recipe::InstallRecipe {
            schema: recipe::RECIPE_SCHEMA.to_string(),
            build_system: recipe::BuildSystem::Cargo,
            library: false,
            binary: "widget".into(),
            artifact: PathBuf::from("target/release/widget"),
            commands: vec![vec!["cargo".into(), "build".into(), "--release".into()]],
            restart_command: None,
        };
        let mut config = InstallConfig {
            no_boar: true,
            ..InstallConfig::default()
        };
        assert!(
            attempt_boar_recovery(&config, None, &recipe, dir.path(), &recipe.commands[0])
                .unwrap()
                .is_none()
        );

        config.no_boar = false;
        config.target_dir = Some(PathBuf::from("/tmp/explicit-target"));
        assert!(
            attempt_boar_recovery(&config, None, &recipe, dir.path(), &recipe.commands[0])
                .unwrap()
                .is_none()
        );
    }
}
