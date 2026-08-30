use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::filesystem::directory_size;
use crate::style;

type Result<T> = std::result::Result<T, String>;

pub fn estimate_cache_dir() -> Option<PathBuf> {
    env::var_os("BOAR_CACHE_DIR")
        .map(PathBuf::from)
        .or_else(|| env::var_os("XDG_CACHE_HOME").map(|root| PathBuf::from(root).join("boar")))
        .or_else(|| env::var_os("HOME").map(|root| PathBuf::from(root).join(".cache/boar")))
}

pub fn estimate_cache_path(project_root: &Path) -> Option<PathBuf> {
    estimate_cache_dir().map(|root| {
        root.join("estimates").join(format!(
            "{:016x}.estimate",
            crate::filesystem::stable_hash(&project_root.to_string_lossy())
        ))
    })
}

pub(crate) fn parse_estimate(text: &str) -> Option<u64> {
    text.lines().find_map(|line| {
        line.strip_prefix("size_kib=")
            .and_then(|value| value.parse().ok())
            .filter(|value| *value > 0)
    })
}

pub fn load_estimate(project_root: &Path) -> Option<u64> {
    estimate_cache_path(project_root)
        .and_then(|path| fs::read_to_string(path).ok())
        .and_then(|text| parse_estimate(text.as_str()))
}

pub fn load_or_seed_estimate(
    project_root: &Path,
    ram_target: &Path,
    disk_target: &Path,
) -> Option<u64> {
    if let Some(estimate) = load_estimate(project_root) {
        return Some(estimate);
    }
    let measured = [ram_target, disk_target]
        .into_iter()
        .filter(|target| target.exists())
        .filter_map(|target| directory_size(target).ok())
        .max()
        .map(|bytes| bytes.div_ceil(1024))
        .filter(|size| *size > 0);
    if let Some(measured) = measured {
        if let Err(error) = save_estimate(project_root, measured) {
            let support = style::stderr_support();
            eprintln!(
                "{} {} {error}",
                style::label("BOAR:", support),
                style::warn("warning:", support)
            );
        }
    }
    measured
}

pub fn save_estimate(project_root: &Path, size_kib: u64) -> Result<()> {
    if size_kib == 0 {
        return Ok(());
    }
    let Some(path) = estimate_cache_path(project_root) else {
        return Ok(());
    };
    let parent = path
        .parent()
        .ok_or_else(|| format!("invalid estimate path {}", path.display()))?;
    fs::create_dir_all(parent)
        .map_err(|error| format!("cannot create {}: {error}", parent.display()))?;
    let temporary = path.with_extension(format!("tmp-{}", std::process::id()));
    let updated = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    fs::write(
        &temporary,
        format!("size_kib={size_kib}\nupdated_unix={updated}\n"),
    )
    .map_err(|error| format!("cannot write {}: {error}", temporary.display()))?;
    fs::rename(&temporary, &path)
        .map_err(|error| format!("cannot replace {}: {error}", path.display()))
}

pub fn refresh_disk_estimate(
    project_root: &Path,
    target: &Path,
    elapsed: std::time::Duration,
    previous: Option<u64>,
) {
    if previous.is_some() && elapsed < std::time::Duration::from_secs(2) {
        return;
    }
    let measured = directory_size(target)
        .ok()
        .map(|bytes| bytes.div_ceil(1024))
        .filter(|size| *size > 0);
    if let Some(measured) = measured {
        if let Err(error) = save_estimate(project_root, measured) {
            let support = style::stderr_support();
            eprintln!(
                "{} {} {error}",
                style::label("BOAR:", support),
                style::warn("warning:", support)
            );
        }
    }
}

pub fn refresh_ram_estimate(
    project_root: &Path,
    ram_root: &Path,
    starting_free_kib: u64,
    starting_target_kib: u64,
) {
    let Some(used_kib) = crate::filesystem::filesystem_available_kib(ram_root)
        .ok()
        .map(|current_free| {
            starting_target_kib.saturating_add(starting_free_kib.saturating_sub(current_free))
        })
        .filter(|size| *size > 0)
    else {
        return;
    };
    if let Err(error) = save_estimate(project_root, used_kib) {
        let support = style::stderr_support();
        eprintln!(
            "{} {} {error}",
            style::label("BOAR:", support),
            style::warn("warning:", support)
        );
    }
}
