pub mod config;
pub mod error;
pub mod logger;

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

use std::env;
use std::fs;
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
    /// Override the Cargo target directory.
    pub target_dir: Option<PathBuf>,
    /// Override the installation directory.
    pub install_dir: Option<PathBuf>,
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

/// Build the current project in release mode and install the resulting binary.
pub fn build_and_install(config: &InstallConfig) -> Result<()> {
    let project = infer_project_name()?;
    log::info!("project inferred as: {project}");

    let release_dir = config
        .target_dir
        .clone()
        .unwrap_or_else(|| PathBuf::from("target/release"));
    let binary_path = release_dir.join(&project);
    log::debug!(
        "release dir: {release_dir}, binary path: {binary_path}",
        release_dir = release_dir.display(),
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
        log::info!(
            "installed {} -> {}",
            binary_path.display(),
            install_path.display()
        );
    }

    Ok(())
}

fn cargo_build_release(config: &InstallConfig, target_dir: &Path) -> Result<()> {
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
        .map_err(|e| BabyError::io("run cargo build", e))?;

    if !status.success() {
        return Err(BabyError::command_failed("cargo build", status.code()));
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
