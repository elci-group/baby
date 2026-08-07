use baby::config::{ProjectConfig, load_all_configs};
use baby::setup_logging;
use clap::{Parser, Subcommand};
use std::fs;
use std::path::PathBuf;

const LONG_ABOUT: &str = r#"
birthctl — control and query the birthd build daemon

birthctl talks to the birthd daemon via PID-file and signal-based IPC. It can
show status, reload configuration, stop the daemon, stream logs, and generate
local per-project configuration files.

Examples:
  birthctl status            # show daemon status and watched projects
  birthctl reload            # tell birthd to reload its configs
  birthctl stop              # gracefully stop the daemon
  birthctl logs              # tail the daemon log file
  birthctl watch --project foo --path src/ --path Cargo.toml
"#;

/// birthctl — control and query the birthd build daemon
#[derive(Parser, Debug)]
#[command(name = "birthctl")]
#[command(version)]
#[command(about = "Control and query the birthd build daemon")]
#[command(long_about = LONG_ABOUT)]
#[command(styles = baby::styles::cli())]
struct Args {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Show daemon status and watched projects
    Status,
    /// Reload daemon configuration
    Reload,
    /// Stop the daemon
    Stop,
    /// Show recent daemon logs
    Logs,
    /// Add a local .birth.toml for the current directory
    Watch {
        /// Project name
        #[arg(long)]
        project: String,
        /// Paths to watch
        #[arg(long)]
        path: Vec<PathBuf>,
        /// Install directory
        #[arg(long, default_value = "/usr/local/bin")]
        install: PathBuf,
        /// Service to restart after install
        #[arg(long)]
        restart: Option<String>,
        /// Build command
        #[arg(long, default_value = "cargo build --release")]
        build: String,
    },
}

fn main() {
    setup_logging();

    // Handle --generate-man before clap enforces subcommand rules.
    if let Some(pos) = std::env::args().position(|a| a == "--generate-man") {
        if let Some(path) = std::env::args().nth(pos + 1) {
            let cmd = <Args as clap::CommandFactory>::command();
            if let Err(e) = baby::generate_man(&cmd, &PathBuf::from(path)) {
                log::error!("{e}");
                std::process::exit(1);
            }
            log::info!("man page written");
            std::process::exit(0);
        }
    }

    if let Err(e) = run() {
        log::error!("{e}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let args = Args::parse();

    match args.command {
        Command::Status => cmd_status(),
        Command::Reload => cmd_reload(),
        Command::Stop => cmd_stop(),
        Command::Logs => cmd_logs(),
        Command::Watch {
            project,
            path,
            install,
            restart,
            build,
        } => cmd_watch(project, path, install, restart, build),
    }
}

fn cmd_status() -> Result<(), String> {
    if let Some(pid) = baby::read_pid_file() {
        if baby::is_process_alive(pid) {
            log::info!("birthd is running (pid {})", pid);
        } else {
            log::warn!("birthd pid file exists but process {} is not alive", pid);
        }
    } else {
        log::info!("birthd is not running");
    }

    let configs = load_all_configs();
    if !configs.is_empty() {
        log::info!("watched projects:");
        for (path, cfg) in &configs {
            log::info!(
                "  {} ({}) -> {} [{}]",
                cfg.project,
                path.display(),
                cfg.install.display(),
                cfg.watch.join(", ")
            );
        }
    } else {
        log::info!("no watched projects found");
    }

    Ok(())
}

fn cmd_reload() -> Result<(), String> {
    if let Some(pid) = baby::read_pid_file() {
        if baby::is_process_alive(pid) {
            unsafe {
                libc::kill(pid as i32, libc::SIGHUP);
            }
            log::info!("birthd reload signaled (pid {})", pid);
            Ok(())
        } else {
            Err(format!("birthd process {} is not alive", pid))
        }
    } else {
        Err("birthd is not running".to_string())
    }
}

fn cmd_stop() -> Result<(), String> {
    if let Some(pid) = baby::read_pid_file() {
        if baby::is_process_alive(pid) {
            unsafe {
                libc::kill(pid as i32, libc::SIGTERM);
            }
            log::info!("birthd stop signaled (pid {})", pid);
            Ok(())
        } else {
            baby::remove_pid_file();
            Err(format!(
                "birthd process {} was not alive, cleaned up pid file",
                pid
            ))
        }
    } else {
        Err("birthd is not running".to_string())
    }
}

fn cmd_logs() -> Result<(), String> {
    let path = baby::log_file_path();
    if path.exists() {
        let content = fs::read_to_string(&path).map_err(|e| format!("failed to read logs: {e}"))?;
        print!("{}", content);
    } else {
        log::info!("no logs found at {}", path.display());
    }
    Ok(())
}

fn cmd_watch(
    project: String,
    paths: Vec<PathBuf>,
    install: PathBuf,
    restart: Option<String>,
    build: String,
) -> Result<(), String> {
    let config = ProjectConfig {
        project,
        watch: paths
            .into_iter()
            .map(|p| p.to_string_lossy().to_string())
            .collect(),
        build,
        install,
        restart,
        debounce_ms: 500,
        strip: false,
        backup: false,
        sudo: false,
        user: false,
    };

    let toml = toml::to_string_pretty(&config).map_err(|e| format!("serialize failed: {e}"))?;
    fs::write(".birth.toml", toml).map_err(|e| format!("write failed: {e}"))?;
    log::info!("created .birth.toml in current directory");

    // Signal reload if daemon is running
    if let Some(pid) = baby::read_pid_file() {
        if baby::is_process_alive(pid) {
            unsafe {
                libc::kill(pid as i32, libc::SIGHUP);
            }
            log::info!("birthd reload signaled");
        }
    }

    Ok(())
}
