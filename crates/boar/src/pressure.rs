use std::fs;
use std::process::Command;
use std::sync::mpsc::{Receiver, RecvTimeoutError};
use std::time::{Duration, Instant};

use crate::memory::{SHM_EMERGENCY_KIB, human_kib};

type Result<T> = std::result::Result<T, String>;

pub const PREFLIGHT_MEMORY_SOME_PCT: f64 = 10.0;
pub const PREFLIGHT_MEMORY_FULL_PCT: f64 = 2.0;
pub const MONITOR_MEMORY_SOME_PCT: f64 = 25.0;
pub const MONITOR_MEMORY_FULL_PCT: f64 = 5.0;
pub const SWAP_ACTIVITY_LIMIT_KIB: u64 = 64 * 1024; // 64 MiB in KiB

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct KernelPressure {
    pub memory_some_avg10: f64,
    pub memory_full_avg10: f64,
    pub io_some_avg10: f64,
    pub swap_in_pages: u64,
    pub swap_out_pages: u64,
}

pub fn read_kernel_pressure() -> Result<KernelPressure> {
    let memory = fs::read_to_string("/proc/pressure/memory")
        .map_err(|error| format!("cannot read memory PSI: {error}"))?;
    let io = fs::read_to_string("/proc/pressure/io")
        .map_err(|error| format!("cannot read I/O PSI: {error}"))?;
    let vmstat = fs::read_to_string("/proc/vmstat")
        .map_err(|error| format!("cannot read /proc/vmstat: {error}"))?;
    let (memory_some_avg10, memory_full_avg10) = parse_psi(&memory)?;
    let (io_some_avg10, _) = parse_psi(&io)?;
    let (swap_in_pages, swap_out_pages) = parse_vmstat(&vmstat)?;
    Ok(KernelPressure {
        memory_some_avg10,
        memory_full_avg10,
        io_some_avg10,
        swap_in_pages,
        swap_out_pages,
    })
}

pub(crate) fn parse_psi(text: &str) -> Result<(f64, f64)> {
    let mut some = None;
    let mut full = None;
    for line in text.lines() {
        let mut fields = line.split_whitespace();
        let class = fields.next();
        let avg10 = fields.find_map(|field| {
            field
                .strip_prefix("avg10=")
                .and_then(|value| value.parse::<f64>().ok())
        });
        match class {
            Some("some") => some = avg10,
            Some("full") => full = avg10,
            _ => {}
        }
    }
    match (some, full) {
        (Some(some), Some(full)) => Ok((some, full)),
        _ => Err("PSI data lacks some/full avg10 values".into()),
    }
}

pub(crate) fn parse_vmstat(text: &str) -> Result<(u64, u64)> {
    let mut swap_in = None;
    let mut swap_out = None;
    for line in text.lines() {
        let mut fields = line.split_whitespace();
        match fields.next() {
            Some("pswpin") => swap_in = fields.next().and_then(|value| value.parse().ok()),
            Some("pswpout") => swap_out = fields.next().and_then(|value| value.parse().ok()),
            _ => {}
        }
    }
    match (swap_in, swap_out) {
        (Some(swap_in), Some(swap_out)) => Ok((swap_in, swap_out)),
        _ => Err("/proc/vmstat lacks pswpin or pswpout".into()),
    }
}

pub fn preflight_pressure_reason(pressure: KernelPressure) -> Option<String> {
    if pressure.memory_full_avg10 >= PREFLIGHT_MEMORY_FULL_PCT {
        Some(format!(
            "memory full PSI is {:.1}%",
            pressure.memory_full_avg10
        ))
    } else if pressure.memory_some_avg10 >= PREFLIGHT_MEMORY_SOME_PCT {
        Some(format!(
            "memory some PSI is {:.1}%",
            pressure.memory_some_avg10
        ))
    } else {
        None
    }
}

pub struct PressureMonitor<'a> {
    pub ram_root: &'a std::path::Path,
    pub memory_floor_kib: u64,
    pub target_limit_kib: u64,
    pub starting_free_kib: u64,
    pub starting_target_kib: u64,
    pub page_size_kib: u64,
}

pub fn monitor_pressure(
    monitor: PressureMonitor<'_>,
    process_group: u32,
    completion: Receiver<()>,
) -> Option<String> {
    let mut last_storage_check = Instant::now() - Duration::from_secs(2);
    let mut previous_kernel = read_kernel_pressure().ok();
    loop {
        let mut pressure = super::memory::read_meminfo()
            .ok()
            .filter(|memory| memory.available_kib < monitor.memory_floor_kib)
            .map(|memory| {
                format!(
                    "available RAM fell to {}",
                    super::memory::human_kib(memory.available_kib)
                )
            });
        if pressure.is_none() {
            if let Ok(current) = read_kernel_pressure() {
                pressure = live_pressure_reason(
                    current,
                    previous_kernel,
                    crate::memory::FALLBACK_PAGE_KIB,
                );
                previous_kernel = Some(current);
            }
        }
        if pressure.is_none() && last_storage_check.elapsed() >= Duration::from_secs(2) {
            if let Ok(current_free) = super::filesystem::filesystem_available_kib(monitor.ram_root)
            {
                if current_free < SHM_EMERGENCY_KIB {
                    pressure = Some(format!(
                        "tmpfs free space fell to {}",
                        human_kib(current_free)
                    ));
                } else {
                    let estimated = monitor
                        .starting_target_kib
                        .saturating_add(monitor.starting_free_kib.saturating_sub(current_free));
                    if estimated > monitor.target_limit_kib {
                        pressure = Some(format!(
                            "target grew to {}, above its {} RAM budget",
                            human_kib(estimated),
                            human_kib(monitor.target_limit_kib)
                        ));
                    }
                }
            }
            last_storage_check = Instant::now();
        }

        if let Some(reason) = pressure {
            signal_process_group(process_group, "-TERM");
            if matches!(
                completion.recv_timeout(Duration::from_secs(2)),
                Err(RecvTimeoutError::Timeout)
            ) {
                signal_process_group(process_group, "-KILL");
            }
            return Some(reason);
        }
        match completion.recv_timeout(Duration::from_millis(250)) {
            Ok(()) | Err(RecvTimeoutError::Disconnected) => return None,
            Err(RecvTimeoutError::Timeout) => {}
        }
    }
}

pub(crate) fn live_pressure_reason(
    current: KernelPressure,
    previous: Option<KernelPressure>,
    page_size_kib: u64,
) -> Option<String> {
    if current.memory_full_avg10 >= MONITOR_MEMORY_FULL_PCT {
        return Some(format!(
            "memory full PSI reached {:.1}%",
            current.memory_full_avg10
        ));
    }
    if current.memory_some_avg10 >= MONITOR_MEMORY_SOME_PCT {
        return Some(format!(
            "memory some PSI reached {:.1}%",
            current.memory_some_avg10
        ));
    }
    let previous = previous?;
    let swapped_pages = current
        .swap_in_pages
        .saturating_sub(previous.swap_in_pages)
        .saturating_add(
            current
                .swap_out_pages
                .saturating_sub(previous.swap_out_pages),
        );
    let swapped_kib = swapped_pages.saturating_mul(page_size_kib);
    (swapped_kib >= SWAP_ACTIVITY_LIMIT_KIB).then(|| {
        format!(
            "swap activity reached {} per sample",
            human_kib(swapped_kib)
        )
    })
}

fn signal_process_group(process_group: u32, signal: &str) {
    let group = format!("-{process_group}");
    let _ = Command::new("kill").args([signal, "--", &group]).status();
}

pub fn parse_bool(name: &str, value: &str) -> Result<bool> {
    match value {
        "true" | "1" | "yes" | "on" => Ok(true),
        "false" | "0" | "no" | "off" => Ok(false),
        _ => Err(format!("{name} must be true or false")),
    }
}
