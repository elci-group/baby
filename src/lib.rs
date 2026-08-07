pub mod config;

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
            .usage(
                Style::new()
                    .bold()
                    .fg_color(Some(AnsiColor::Yellow.into())),
            )
            .literal(
                Style::new()
                    .bold()
                    .fg_color(Some(AnsiColor::Cyan.into())),
            )
            .placeholder(
                Style::new()
                    .fg_color(Some(AnsiColor::White.into())),
            )
            .error(
                Style::new()
                    .bold()
                    .fg_color(Some(AnsiColor::Red.into())),
            )
            .valid(
                Style::new()
                    .bold()
                    .fg_color(Some(AnsiColor::Green.into())),
            )
            .invalid(
                Style::new()
                    .bold()
                    .fg_color(Some(AnsiColor::Red.into())),
            )
    }
}

use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// Initialise `env_logger` with a default filter of `info`.
///
/// Call this early in `main()` so that `log::info!`, `log::warn!`, etc. work
/// immediately. The user can override the level via the `RUST_LOG` environment
/// variable (e.g. `RUST_LOG=baby=trace`).
pub fn setup_logging() {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info"))
        .format_timestamp_secs()
        .format_target(false)
        .init();
}

/// Write a man page for the given clap `Command` to the supplied path.
///
/// The parent directories are created automatically. The file is written in
/// groff format and can be viewed with `man -l <path>`.
pub fn generate_man(cmd: &clap::Command, path: &Path) -> Result<(), String> {
    let man = clap_mangen::Man::new(cmd.clone());
    let mut buf = Vec::new();
    man.render(&mut buf)
        .map_err(|e| format!("failed to render man page: {e}"))?;
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    fs::write(path, buf).map_err(|e| format!("failed to write man page: {e}"))?;
    Ok(())
}

pub struct InstallConfig {
    pub strip: bool,
    pub backup: bool,
    pub service: bool,
    pub sudo: bool,
    pub user: bool,
    pub dry_run: bool,
    pub target_dir: Option<PathBuf>,
    pub install_dir: Option<PathBuf>,
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
            target_dir: None,
            install_dir: None,
        }
    }
}

pub fn infer_project_name() -> Result<String, String> {
    let cwd = env::current_dir().map_err(|e| format!("failed to get cwd: {e}"))?;
    cwd.file_name()
        .and_then(|n| n.to_str())
        .map(|n| n.to_string())
        .ok_or_else(|| "cannot infer project name from current directory".to_string())
}

pub fn home_local_bin() -> Result<PathBuf, String> {
    let home = env::var("HOME").map_err(|e| format!("HOME not set: {e}"))?;
    Ok(PathBuf::from(home).join(".local/bin"))
}

pub fn xdg_runtime_dir() -> PathBuf {
    env::var("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("/tmp"))
}

pub fn xdg_state_dir() -> PathBuf {
    env::var("XDG_STATE_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            env::var("HOME")
                .map(|h| PathBuf::from(h).join(".local/state"))
                .unwrap_or_else(|_| PathBuf::from("/tmp"))
        })
}

pub fn xdg_config_dir() -> PathBuf {
    env::var("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            env::var("HOME")
                .map(|h| PathBuf::from(h).join(".config"))
                .unwrap_or_else(|_| PathBuf::from("/tmp"))
        })
}

pub fn pid_file_path() -> PathBuf {
    xdg_runtime_dir().join("birthd.pid")
}

pub fn log_file_path() -> PathBuf {
    xdg_state_dir().join("birthd.log")
}

pub fn birthd_config_dirs() -> Vec<PathBuf> {
    let mut dirs = vec![];
    if let Ok(home) = env::var("HOME") {
        dirs.push(PathBuf::from(&home).join(".config/birth.d"));
    }
    dirs.push(PathBuf::from("/etc/birth.d"));
    dirs
}

pub fn read_pid_file() -> Option<u32> {
    fs::read_to_string(pid_file_path())
        .ok()
        .and_then(|s| s.trim().parse().ok())
}

pub fn write_pid_file(pid: u32) -> Result<(), String> {
    let path = pid_file_path();
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    fs::write(&path, pid.to_string())
        .map_err(|e| format!("failed to write pid file: {e}"))
}

pub fn remove_pid_file() {
    let _ = fs::remove_file(pid_file_path());
}

pub fn is_process_alive(pid: u32) -> bool {
    unsafe { libc::kill(pid as i32, 0) == 0 }
}

pub fn build_and_install(config: &InstallConfig) -> Result<(), String> {
    let project = infer_project_name()?;
    log::info!("project inferred as: {project}");

    let release_dir = config
        .target_dir
        .clone()
        .unwrap_or_else(|| PathBuf::from("target/release"));
    let binary_path = release_dir.join(&project);
    log::debug!("release dir: {release_dir}, binary path: {binary_path}", release_dir = release_dir.display(), binary_path = binary_path.display());

    let install_dir = if let Some(ref dir) = config.install_dir {
        dir.clone()
    } else if config.user {
        home_local_bin()?
    } else {
        PathBuf::from("/usr/local/bin")
    };

    let install_path = install_dir.join(&project);
    log::debug!("install dir: {install_dir}, install path: {install_path}", install_dir = install_dir.display(), install_path = install_path.display());

    cargo_build_release(config, &release_dir)?;

    if config.strip {
        strip_binary(config, &binary_path)?;
    }

    ensure_install_dir(config, &install_dir)?;

    if config.backup {
        backup_existing(config, &install_path)?;
    }

    install_binary(config, &binary_path, &install_path)?;

    if config.service {
        restart_systemd_service(config, &project)?;
    }

    if !config.dry_run {
        log::info!("installed {} -> {}", binary_path.display(), install_path.display());
    }

    Ok(())
}

fn cargo_build_release(config: &InstallConfig, target_dir: &Path) -> Result<(), String> {
    let mut cmd = Command::new("cargo");
    cmd.arg("build").arg("--release");
    if target_dir != Path::new("target/release") {
        cmd.arg("--target-dir").arg(target_dir);
    }

    log::debug!("cargo build command: {}", format_command(&cmd));

    if config.dry_run {
        log::info!("[dry-run] would run: {}", format_command(&cmd));
        return Ok(());
    }

    let status = cmd
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()
        .map_err(|e| format!("failed to run cargo: {e}"))?;

    if !status.success() {
        return Err("cargo build failed".to_string());
    }
    Ok(())
}

fn strip_binary(config: &InstallConfig, binary: &Path) -> Result<(), String> {
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
        .map_err(|e| format!("failed to run strip: {e}"))?;

    if !status.success() {
        return Err("strip failed".to_string());
    }
    Ok(())
}

fn ensure_install_dir(config: &InstallConfig, dir: &Path) -> Result<(), String> {
    if !dir.exists() {
        let mut cmd = Command::new("mkdir");
        cmd.arg("-p").arg(dir);

        if config.sudo {
            cmd = wrap_sudo(cmd);
        }

        log::debug!("mkdir command: {}", format_command(&cmd));

        if config.dry_run {
            log::info!("[dry-run] would run: {}", format_command(&cmd));
            return Ok(());
        }

        let status = cmd
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit())
            .status()
            .map_err(|e| format!("failed to create install dir: {e}"))?;

        if !status.success() {
            return Err("mkdir failed".to_string());
        }
    } else {
        log::debug!("install dir already exists: {}", dir.display());
    }
    Ok(())
}

fn backup_existing(config: &InstallConfig, install_path: &Path) -> Result<(), String> {
    if !install_path.exists() {
        log::debug!("no existing binary at {}, skipping backup", install_path.display());
        return Ok(());
    }
    let backup_path = install_path.with_extension("backup");
    let mut cmd = Command::new("cp");
    cmd.arg(install_path).arg(&backup_path);

    if config.sudo {
        cmd = wrap_sudo(cmd);
    }

    log::debug!("backup command: {}", format_command(&cmd));

    if config.dry_run {
        log::info!("[dry-run] would run: {}", format_command(&cmd));
        return Ok(());
    }

    let status = cmd
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()
        .map_err(|e| format!("failed to backup binary: {e}"))?;

    if !status.success() {
        return Err("backup failed".to_string());
    }

    log::info!("backed up {} -> {}", install_path.display(), backup_path.display());
    Ok(())
}

fn install_binary(config: &InstallConfig, from: &Path, to: &Path) -> Result<(), String> {
    let mut cmd = Command::new("install");
    cmd.arg("-m755").arg(from).arg(to);

    if config.sudo {
        cmd = wrap_sudo(cmd);
    }

    log::debug!("install command: {}", format_command(&cmd));

    if config.dry_run {
        log::info!("[dry-run] would run: {}", format_command(&cmd));
        return Ok(());
    }

    let status = cmd
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()
        .map_err(|e| format!("failed to install binary: {e}"))?;

    if !status.success() {
        return Err("install failed".to_string());
    }
    Ok(())
}

fn restart_systemd_service(config: &InstallConfig, project: &str) -> Result<(), String> {
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
        .map_err(|e| format!("failed to restart service: {e}"))?;

    if !status.success() {
        return Err(format!("systemctl restart {} failed", service_name));
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

pub fn run_binary(path: &Path, args: &[String]) -> Result<(), String> {
    let mut cmd = Command::new(path);
    cmd.args(args);

    let status = cmd
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()
        .map_err(|e| format!("failed to run binary: {e}"))?;

    std::process::exit(status.code().unwrap_or(1));
}
