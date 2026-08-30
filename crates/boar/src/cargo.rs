use std::os::unix::process::CommandExt;
use std::path::Path;
use std::process::{Command, ExitStatus, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::Instant;

use crate::config::Settings;
use crate::estimate::{refresh_disk_estimate, refresh_ram_estimate};
use crate::filesystem::{filesystem_available_kib, guarded_remove};
use crate::pressure::{PressureMonitor, monitor_pressure};
use crate::style;

type Result<T> = std::result::Result<T, String>;

pub enum CargoResult {
    Exited(ExitStatus, std::time::Duration),
    Pressure(String),
}

pub fn run_cargo_once(
    project_root: &Path,
    target: &Path,
    cargo_command: &str,
    cargo_args: &[String],
    monitor: Option<PressureMonitor<'_>>,
) -> Result<CargoResult> {
    let mut command = Command::new("cargo");
    command
        .arg(cargo_command)
        .args(cargo_args)
        .current_dir(project_root)
        .env("CARGO_TARGET_DIR", target)
        .env("BOAR_ACTIVE", "1")
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .process_group(0);

    let mut child = command
        .spawn()
        .map_err(|error| format!("cannot start cargo: {error}"))?;
    let started = Instant::now();
    let Some(monitor) = monitor else {
        return child
            .wait()
            .map(|status| CargoResult::Exited(status, started.elapsed()))
            .map_err(|error| format!("cannot wait for cargo: {error}"));
    };

    let pid = child.id();
    let (completion_tx, completion_rx) = mpsc::channel();
    thread::scope(|scope| {
        let monitor_thread = scope.spawn(move || monitor_pressure(monitor, pid, completion_rx));
        let status = child
            .wait()
            .map_err(|error| format!("cannot wait for cargo: {error}"));
        let _ = completion_tx.send(());
        let pressure = monitor_thread
            .join()
            .map_err(|_| "pressure monitor panicked".to_owned())?;
        match (status, pressure) {
            (_, Some(reason)) => Ok(CargoResult::Pressure(reason)),
            (Ok(status), None) => Ok(CargoResult::Exited(status, started.elapsed())),
            (Err(error), None) => Err(error),
        }
    })
}

pub fn finish_disk_build(
    result: CargoResult,
    project_root: &Path,
    disk_target: &Path,
    previous_estimate: Option<u64>,
) -> Result<i32> {
    match result {
        CargoResult::Exited(status, elapsed) => {
            if status.success() {
                refresh_disk_estimate(project_root, disk_target, elapsed, previous_estimate);
            }
            Ok(status_code(status))
        }
        CargoResult::Pressure(_) => unreachable!("disk builds are not monitored"),
    }
}

pub fn status_code(status: ExitStatus) -> i32 {
    status.code().unwrap_or(1)
}

pub fn run_cargo(
    settings: &Settings,
    project_root: &Path,
    cargo_command: &str,
    cargo_args: &[String],
) -> Result<i32> {
    if cargo_args
        .iter()
        .any(|arg| arg == "--target-dir" || arg.starts_with("--target-dir="))
    {
        return Err(
            "Cargo --target-dir conflicts with BOAR; use --disk-target or --ram-root".into(),
        );
    }

    let ram_target = crate::filesystem::project_target(&settings.ram_root, project_root);
    let disk_target = crate::filesystem::disk_target(settings, project_root);
    let previous_estimate =
        crate::estimate::load_or_seed_estimate(project_root, &ram_target, &disk_target);

    if settings.mode == crate::config::Mode::Disk {
        let support = style::stderr_support();
        eprintln!(
            "{} disk target {} ({})",
            style::label("BOAR:", support),
            style::path(disk_target.display().to_string(), support),
            style::dim("disk mode was requested", support)
        );
        let result = run_cargo_once(project_root, &disk_target, cargo_command, cargo_args, None)?;
        return finish_disk_build(result, project_root, &disk_target, previous_estimate);
    }

    let memory = crate::memory::read_meminfo()?;
    let ram_kind = crate::filesystem::filesystem_type(&settings.ram_root)?;
    let ram_free = filesystem_available_kib(&settings.ram_root)?;
    let estimate = previous_estimate.unwrap_or(crate::memory::DEFAULT_ESTIMATE_KIB);
    let starting_target_kib = if ram_target.exists() {
        previous_estimate.unwrap_or(0)
    } else {
        0
    };
    let allocation = crate::memory::calculate_allocation(
        memory,
        ram_free,
        settings.reserve_mib,
        settings.max_ram_mib,
        estimate,
    );
    let is_memory_fs = matches!(ram_kind.as_str(), "tmpfs" | "ramfs");
    let pressure_reason = crate::pressure::read_kernel_pressure()
        .ok()
        .and_then(crate::pressure::preflight_pressure_reason);

    let use_ram = match settings.mode {
        crate::config::Mode::Disk => false,
        crate::config::Mode::Auto => {
            is_memory_fs && allocation.enough() && pressure_reason.is_none()
        }
        crate::config::Mode::Ram if !is_memory_fs => {
            return Err(format!(
                "{} is on {ram_kind}, not tmpfs/ramfs; refusing forced RAM mode",
                settings.ram_root.display()
            ));
        }
        crate::config::Mode::Ram if allocation.budget_kib < crate::memory::MIN_RAM_KIB => {
            return Err(format!(
                "only {} can be safely allocated; forced RAM mode needs at least {}",
                crate::memory::human_kib(allocation.budget_kib),
                crate::memory::human_kib(crate::memory::MIN_RAM_KIB)
            ));
        }
        crate::config::Mode::Ram if settings.monitor && pressure_reason.is_some() => {
            return Err(format!(
                "forced RAM mode refused because {}; wait for pressure to fall or use --no-monitor",
                pressure_reason
                    .as_deref()
                    .unwrap_or("memory pressure is elevated")
            ));
        }
        crate::config::Mode::Ram => true,
    };

    if !use_ram {
        let support = style::stderr_support();
        let reason = match settings.mode {
            _ if !is_memory_fs => format!("RAM root uses {ram_kind}, not tmpfs"),
            _ if pressure_reason.is_some() => pressure_reason
                .as_deref()
                .unwrap_or("memory pressure is elevated")
                .to_owned(),
            _ => format!(
                "safe RAM budget {} is below estimated need {}",
                crate::memory::human_kib(allocation.budget_kib),
                crate::memory::human_kib(allocation.required_kib)
            ),
        };
        eprintln!(
            "{} disk target {} ({})",
            style::label("BOAR:", support),
            style::path(disk_target.display().to_string(), support),
            style::dim(reason, support)
        );
        let result = run_cargo_once(project_root, &disk_target, cargo_command, cargo_args, None)?;
        return finish_disk_build(result, project_root, &disk_target, previous_estimate);
    }

    crate::filesystem::write_marker(&ram_target, project_root)?;
    let support = style::stderr_support();
    eprintln!(
        "{} RAM target {} (budget {}, estimate {})",
        style::label("BOAR:", support),
        style::path(ram_target.display().to_string(), support),
        style::value(crate::memory::human_kib(allocation.budget_kib), support),
        style::value(crate::memory::human_kib(allocation.required_kib), support)
    );
    let retryable = matches!(cargo_command, "build" | "check" | "clippy" | "doc");
    let memory_floor = (memory.total_kib / 20)
        .clamp(
            256 * crate::memory::KIB_PER_MIB,
            1024 * crate::memory::KIB_PER_MIB,
        )
        .min(allocation.reserve_kib.max(256 * crate::memory::KIB_PER_MIB));
    match run_cargo_once(
        project_root,
        &ram_target,
        cargo_command,
        cargo_args,
        (settings.monitor && retryable).then_some(PressureMonitor {
            ram_root: &settings.ram_root,
            memory_floor_kib: memory_floor,
            target_limit_kib: allocation.budget_kib,
            starting_free_kib: ram_free,
            starting_target_kib,
            page_size_kib: crate::memory::system_page_size_kib(),
        }),
    )? {
        CargoResult::Exited(status, _) => {
            if status.success() {
                refresh_ram_estimate(
                    project_root,
                    &settings.ram_root,
                    ram_free,
                    starting_target_kib,
                );
            }
            Ok(status_code(status))
        }
        CargoResult::Pressure(reason) => {
            guarded_remove(&settings.ram_root, &ram_target)?;
            if settings.mode == crate::config::Mode::Ram {
                return Err(format!(
                    "forced RAM build stopped because {reason}; increase the budget or use auto mode"
                ));
            }
            let support = style::stderr_support();
            eprintln!(
                "{} {}; spilling build to {}",
                style::warn("BOAR:", support),
                style::warn(reason, support),
                style::path(disk_target.display().to_string(), support)
            );
            let result =
                run_cargo_once(project_root, &disk_target, cargo_command, cargo_args, None)?;
            finish_disk_build(result, project_root, &disk_target, previous_estimate)
        }
    }
}
