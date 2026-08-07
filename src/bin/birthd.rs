use baby::config::{ProjectConfig, load_all_configs, path_to_project_map};
use baby::error::{BabyError, Result};
use baby::{InstallConfig, build_and_install, setup_logging};
use chrono::Local;
use clap::Parser;
use notify::{Config, Event, RecommendedWatcher, RecursiveMode, Watcher};
use std::collections::{HashMap, HashSet};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{RecvTimeoutError, channel};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const LONG_ABOUT: &str = r#"
birthd — build daemon that watches Rust projects and rebuilds on change

birthd scans for .birth.toml files, sets up filesystem watchers, and
automatically rebuilds and reinstalls binaries when source files change.

It stores its PID in $XDG_RUNTIME_DIR/birthd.pid (or /tmp/birthd.pid) and
writes logs to $XDG_STATE_HOME/birthd.log (or ~/.local/state/birthd.log).

Signals:
  SIGHUP   — reload configuration
  SIGTERM  — graceful shutdown
  SIGINT   — graceful shutdown
"#;

#[derive(clap::Parser, Debug)]
#[command(name = "birthd")]
#[command(version)]
#[command(about = "Build daemon that watches and rebuilds Rust projects")]
#[command(long_about = LONG_ABOUT)]
#[command(styles = baby::styles::cli())]
struct Args {
    /// Generate a man page and exit
    #[arg(long, hide = true)]
    generate_man: Option<PathBuf>,
}

struct DaemonState {
    configs: Vec<(PathBuf, ProjectConfig)>,
    path_map: HashMap<PathBuf, Vec<usize>>,
    pending: HashMap<usize, Instant>,
    watcher: Option<RecommendedWatcher>,
}

fn main() {
    setup_logging();
    if let Err(e) = run() {
        log::error!("{e}");
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    let args = Args::parse();

    if let Some(path) = args.generate_man {
        let cmd = <Args as clap::CommandFactory>::command();
        baby::generate_man(&cmd, &path)?;
        log::info!("man page written to {}", path.display());
        return Ok(());
    }

    let pid = std::process::id();
    baby::write_pid_file(pid)?;

    let running = Arc::new(AtomicBool::new(true));
    let r = running.clone();
    let _ = signal_hook::flag::register(signal_hook::consts::SIGTERM, r.clone());
    let _ = signal_hook::flag::register(signal_hook::consts::SIGINT, r.clone());

    let state = Arc::new(Mutex::new(DaemonState {
        configs: vec![],
        path_map: HashMap::new(),
        pending: HashMap::new(),
        watcher: None,
    }));

    reload_configs(&state)?;

    let (tx, rx) = channel::<std::result::Result<Event, notify::Error>>();
    let watcher = RecommendedWatcher::new(
        move |res| {
            let _ = tx.send(res);
        },
        Config::default(),
    )
    .map_err(BabyError::watch)?;

    {
        let mut s = state.lock().unwrap();
        s.watcher = Some(watcher);
    }

    setup_watchers(&state)?;

    log::info!("birthd started (pid {})", pid);
    log_message("birthd started");

    loop {
        if !running.load(Ordering::Relaxed) {
            break;
        }

        match rx.recv_timeout(Duration::from_millis(100)) {
            Ok(Ok(event)) => {
                if is_relevant_event(&event) {
                    if let Err(e) = handle_event(&state, &event) {
                        log::warn!("failed to handle event: {e}");
                        log_message(&format!("failed to handle event: {e}"));
                    }
                }
            }
            Ok(Err(e)) => {
                log::warn!("watch error: {e}");
                log_message(&format!("watch error: {e}"));
            }
            Err(RecvTimeoutError::Timeout) => {
                if let Err(e) = process_pending(&state) {
                    log::warn!("failed to process pending builds: {e}");
                    log_message(&format!("failed to process pending builds: {e}"));
                }
            }
            Err(RecvTimeoutError::Disconnected) => {
                log::warn!("watch channel disconnected, shutting down");
                log_message("watch channel disconnected, shutting down");
                break;
            }
        }
    }

    log::info!("birthd shutting down");
    log_message("birthd shutting down");
    baby::remove_pid_file();
    Ok(())
}

fn reload_configs(state: &Arc<Mutex<DaemonState>>) -> Result<()> {
    let configs = load_all_configs();
    let path_map = path_to_project_map(&configs);
    let mut s = state.lock().unwrap();
    s.configs = configs;
    s.path_map = path_map;
    Ok(())
}

fn setup_watchers(state: &Arc<Mutex<DaemonState>>) -> Result<()> {
    let mut s = state.lock().unwrap();
    let watched: HashSet<PathBuf> = s.path_map.keys().cloned().collect();

    if let Some(ref mut watcher) = s.watcher {
        for path in watched {
            let mode = if path.is_dir() {
                RecursiveMode::Recursive
            } else {
                RecursiveMode::NonRecursive
            };
            if let Err(e) = watcher.watch(&path, mode) {
                log::warn!("failed to watch {}: {e}", path.display());
                log_message(&format!("failed to watch {}: {e}", path.display()));
            } else {
                log::info!("watching {}", path.display());
                log_message(&format!("watching {}", path.display()));
            }
        }
    }

    Ok(())
}

fn handle_event(state: &Arc<Mutex<DaemonState>>, event: &Event) -> Result<()> {
    let mut s = state.lock().unwrap();
    let now = Instant::now();

    let mut matches = vec![];
    for path in event.paths.iter() {
        for (watch_path, indices) in s.path_map.iter() {
            if path.starts_with(watch_path) {
                for &idx in indices {
                    matches.push(idx);
                }
            }
        }
    }

    for idx in matches {
        s.pending.insert(idx, now);
    }

    Ok(())
}

fn process_pending(state: &Arc<Mutex<DaemonState>>) -> Result<()> {
    let mut ready = vec![];
    {
        let mut s = state.lock().unwrap();
        let now = Instant::now();
        for (&idx, &last) in &s.pending {
            let debounce = Duration::from_millis(s.configs[idx].1.debounce_ms);
            if now.duration_since(last) >= debounce {
                ready.push(idx);
            }
        }
        for idx in &ready {
            s.pending.remove(idx);
        }
    }

    for idx in ready {
        let (path, cfg) = {
            let s = state.lock().unwrap();
            s.configs[idx].clone()
        };

        log::info!("rebuilding {}", cfg.project);
        log_message(&format!("rebuilding {}", cfg.project));

        let install_cfg = InstallConfig {
            strip: cfg.strip,
            backup: cfg.backup,
            service: cfg.restart.is_some(),
            sudo: cfg.sudo,
            user: cfg.user,
            dry_run: false,
            target_dir: None,
            install_dir: Some(cfg.install.clone()),
        };

        // Run the custom build command if it differs from the default
        if cfg.build != "cargo build --release" {
            let base = path.parent().unwrap_or(Path::new("."));
            let mut parts = cfg.build.split_whitespace();
            let cmd = parts.next().unwrap_or("cargo");
            let mut command = std::process::Command::new(cmd);
            command.args(parts).current_dir(base);

            let status = command
                .stdout(std::process::Stdio::inherit())
                .stderr(std::process::Stdio::inherit())
                .status()
                .map_err(|e| BabyError::io(format!("run build command for {}", cfg.project), e))?;

            if !status.success() {
                log::warn!("build failed for {}", cfg.project);
                log_message(&format!("build failed for {}", cfg.project));
                continue;
            }
        } else if let Err(e) = build_and_install(&install_cfg) {
            log::warn!("build failed for {}: {e}", cfg.project);
            log_message(&format!("build failed for {}: {e}", cfg.project));
            continue;
        }

        if let Some(ref service) = cfg.restart {
            let mut cmd = std::process::Command::new("systemctl");
            cmd.arg("restart").arg(service);
            if cfg.sudo {
                let mut sudo = std::process::Command::new("sudo");
                sudo.arg("systemctl").arg("restart").arg(service);
                cmd = sudo;
            }
            let _ = cmd.status();
            log::info!("restarted {}", service);
            log_message(&format!("restarted {}", service));
        }

        log::info!("{} rebuilt successfully", cfg.project);
        log_message(&format!("{} rebuilt successfully", cfg.project));
    }

    Ok(())
}

fn is_relevant_event(event: &Event) -> bool {
    use notify::EventKind::*;
    matches!(event.kind, Create(_) | Modify(_) | Remove(_))
}

fn log_message(msg: &str) {
    let line = format!(
        "{} [birthd] {}\n",
        Local::now().format("%Y-%m-%d %H:%M:%S"),
        msg
    );
    let path = baby::log_file_path();
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(&path) {
        let _ = file.write_all(line.as_bytes());
    }
}
