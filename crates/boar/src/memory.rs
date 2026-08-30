use std::fs;
use std::process::Command;

type Result<T> = std::result::Result<T, String>;

pub const KIB_PER_MIB: u64 = 1024;
pub const DEFAULT_ESTIMATE_KIB: u64 = 512 * KIB_PER_MIB;
pub const MIN_RAM_KIB: u64 = 128 * KIB_PER_MIB;
pub const SHM_HEADROOM_KIB: u64 = 128 * KIB_PER_MIB;
pub const SHM_EMERGENCY_KIB: u64 = 64 * KIB_PER_MIB;
pub const FALLBACK_PAGE_KIB: u64 = 4;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MemInfo {
    pub total_kib: u64,
    pub available_kib: u64,
    pub swap_total_kib: u64,
    pub swap_free_kib: u64,
}

pub fn read_meminfo() -> Result<MemInfo> {
    let text = fs::read_to_string("/proc/meminfo")
        .map_err(|error| format!("cannot read /proc/meminfo: {error}"))?;
    parse_meminfo(&text)
}

pub(crate) fn parse_meminfo(text: &str) -> Result<MemInfo> {
    let mut total = None;
    let mut available = None;
    let mut swap_total = 0;
    let mut swap_free = 0;
    for line in text.lines() {
        let mut fields = line.split_whitespace();
        match fields.next() {
            Some("MemTotal:") => total = fields.next().and_then(|value| value.parse().ok()),
            Some("MemAvailable:") => available = fields.next().and_then(|value| value.parse().ok()),
            Some("SwapTotal:") => {
                swap_total = fields
                    .next()
                    .and_then(|value| value.parse().ok())
                    .unwrap_or(0)
            }
            Some("SwapFree:") => {
                swap_free = fields
                    .next()
                    .and_then(|value| value.parse().ok())
                    .unwrap_or(0)
            }
            _ => {}
        }
    }
    match (total, available) {
        (Some(total_kib), Some(available_kib)) => Ok(MemInfo {
            total_kib,
            available_kib,
            swap_total_kib: swap_total,
            swap_free_kib: swap_free,
        }),
        _ => Err("/proc/meminfo lacks MemTotal or MemAvailable".into()),
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Allocation {
    pub reserve_kib: u64,
    pub budget_kib: u64,
    pub required_kib: u64,
}

impl Allocation {
    pub fn enough(self) -> bool {
        self.budget_kib >= self.required_kib
    }
}

pub fn calculate_allocation(
    memory: MemInfo,
    filesystem_free_kib: u64,
    reserve_mib: Option<u64>,
    max_ram_mib: Option<u64>,
    estimate_kib: u64,
) -> Allocation {
    let dynamic_floor = (2 * 1024 * KIB_PER_MIB).min(memory.total_kib / 2);
    let reserve_kib = reserve_mib
        .map(|value| value.saturating_mul(KIB_PER_MIB))
        .unwrap_or_else(|| (memory.total_kib / 4).max(dynamic_floor));
    let memory_budget = memory.available_kib.saturating_sub(reserve_kib);
    let filesystem_budget = filesystem_free_kib.saturating_sub(SHM_HEADROOM_KIB);
    let mut budget_kib = memory_budget.min(filesystem_budget);
    if let Some(max_mib) = max_ram_mib {
        budget_kib = budget_kib.min(max_mib.saturating_mul(KIB_PER_MIB));
    }
    let growth = (estimate_kib / 4).max(256 * KIB_PER_MIB);
    let required_kib = estimate_kib
        .saturating_add(growth)
        .max(DEFAULT_ESTIMATE_KIB);
    Allocation {
        reserve_kib,
        budget_kib,
        required_kib,
    }
}

pub fn parse_u64(name: &str, value: &str) -> Result<u64> {
    value
        .parse()
        .map_err(|_| format!("{name} must be a non-negative integer"))
}

pub fn human_kib(kib: u64) -> String {
    human_bytes(kib.saturating_mul(1024))
}

pub fn human_kib_aligned(kib: u64) -> String {
    format!(" {:>8}", human_kib(kib))
}

pub fn human_bytes(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KiB", "MiB", "GiB", "TiB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} {}", UNITS[unit])
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

pub fn system_page_size_kib() -> u64 {
    Command::new("getconf")
        .arg("PAGESIZE")
        .output()
        .ok()
        .filter(|output| output.status.success())
        .and_then(|output| String::from_utf8(output.stdout).ok())
        .and_then(|value| value.trim().parse::<u64>().ok())
        .map(|bytes| bytes.div_ceil(1024))
        .filter(|kib| *kib > 0)
        .unwrap_or(FALLBACK_PAGE_KIB)
}
