use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;

type Result<T> = std::result::Result<T, String>;

pub fn filesystem_available_kib(path: &Path) -> Result<u64> {
    let existing = existing_ancestor(path)?;
    let output = Command::new("df")
        .arg("-Pk")
        .arg(&existing)
        .output()
        .map_err(|error| format!("cannot run df for {}: {error}", path.display()))?;
    if !output.status.success() {
        return Err(format!("df failed for {}", path.display()));
    }
    let text = String::from_utf8_lossy(&output.stdout);
    let line = text
        .lines()
        .last()
        .ok_or_else(|| "df returned no output".to_owned())?;
    line.split_whitespace()
        .nth(3)
        .and_then(|value| value.parse().ok())
        .ok_or_else(|| format!("cannot parse available space from df output: {line}"))
}

pub fn filesystem_type(path: &Path) -> Result<String> {
    let canonical = existing_ancestor(path)?;
    let mounts = fs::read_to_string("/proc/mounts")
        .map_err(|error| format!("cannot read /proc/mounts: {error}"))?;
    let mut best: Option<(usize, String)> = None;
    for line in mounts.lines() {
        let fields: Vec<_> = line.split_whitespace().collect();
        if fields.len() < 3 {
            continue;
        }
        let mount = PathBuf::from(unescape_mount(fields[1]));
        if canonical.starts_with(&mount) {
            let length = mount.as_os_str().len();
            if best
                .as_ref()
                .is_none_or(|(best_length, _)| length > *best_length)
            {
                best = Some((length, fields[2].to_owned()));
            }
        }
    }
    best.map(|(_, kind)| kind)
        .ok_or_else(|| format!("cannot find mount for {}", canonical.display()))
}

fn existing_ancestor(path: &Path) -> Result<PathBuf> {
    let mut candidate = path;
    while !candidate.exists() {
        candidate = candidate
            .parent()
            .ok_or_else(|| format!("{} has no existing ancestor", path.display()))?;
    }
    candidate
        .canonicalize()
        .map_err(|error| format!("cannot resolve {}: {error}", candidate.display()))
}

pub(crate) fn unescape_mount(value: &str) -> String {
    value
        .replace("\\040", " ")
        .replace("\\011", "\t")
        .replace("\\012", "\n")
        .replace("\\134", "\\")
}

pub fn directory_size(path: &Path) -> io::Result<u64> {
    let metadata = fs::symlink_metadata(path)?;
    if metadata.file_type().is_symlink() {
        return Ok(0);
    }
    if metadata.is_file() {
        return Ok(metadata.len());
    }
    let mut total = 0_u64;
    for entry in fs::read_dir(path)? {
        total = total.saturating_add(directory_size(&entry?.path())?);
    }
    Ok(total)
}

pub fn project_target(ram_root: &Path, project_root: &Path) -> PathBuf {
    let name = project_root
        .file_name()
        .and_then(std::ffi::OsStr::to_str)
        .map(sanitize)
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "project".into());
    ram_root.join(format!(
        "{name}-{:016x}",
        stable_hash(&project_root.to_string_lossy())
    ))
}

pub(crate) fn sanitize(value: &str) -> String {
    value
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '-' | '_') {
                character
            } else {
                '-'
            }
        })
        .collect()
}

pub fn stable_hash(value: &str) -> u64 {
    let mut hash = 0xcbf29ce484222325_u64;
    for byte in value.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

pub fn disk_target(settings: &crate::config::Settings, project_root: &Path) -> PathBuf {
    let configured = settings
        .disk_target
        .clone()
        .or_else(|| std::env::var_os("CARGO_TARGET_DIR").map(PathBuf::from))
        .unwrap_or_else(|| PathBuf::from("target"));
    if configured.is_absolute() {
        configured
    } else {
        project_root.join(configured)
    }
}

pub fn discover_project_root() -> Result<PathBuf> {
    let cargo_result = Command::new("cargo")
        .args(["locate-project", "--workspace", "--message-format", "plain"])
        .stderr(std::process::Stdio::null())
        .output();
    if let Ok(output) = cargo_result {
        if output.status.success() {
            let manifest = String::from_utf8_lossy(&output.stdout).trim().to_owned();
            if let Some(parent) = Path::new(&manifest).parent() {
                return parent
                    .canonicalize()
                    .map_err(|error| format!("cannot resolve project root: {error}"));
            }
        }
    }
    std::env::current_dir()
        .map_err(|error| format!("cannot read current directory: {error}"))?
        .canonicalize()
        .map_err(|error| format!("cannot resolve current directory: {error}"))
}

pub fn write_marker(target: &Path, project_root: &Path) -> Result<()> {
    fs::create_dir_all(target)
        .map_err(|error| format!("cannot create {}: {error}", target.display()))?;
    let marker = target.join(".boar-owner");
    let content = format!(
        "{}\nversion={}\n",
        project_root.display(),
        env!("CARGO_PKG_VERSION")
    );
    fs::write(&marker, content)
        .map_err(|error| format!("cannot write {}: {error}", marker.display()))
}

pub fn guarded_remove(ram_root: &Path, target: &Path) -> Result<bool> {
    if !target.starts_with(ram_root) || target == ram_root {
        return Err(format!(
            "refusing to clean unsafe path {}",
            target.display()
        ));
    }
    if !target.exists() {
        return Ok(false);
    }
    if !target.join(".boar-owner").is_file() {
        return Err(format!(
            "refusing to clean unowned path {}",
            target.display()
        ));
    }
    fs::remove_dir_all(target)
        .map_err(|error| format!("cannot clean {}: {error}", target.display()))?;
    Ok(true)
}
