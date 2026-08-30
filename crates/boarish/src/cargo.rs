//! Cargo invocation and input fingerprinting helpers.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::CompilationIdentity;
use crate::identity::fingerprint_sources;

/// Relevant environment variables that can affect a Rust compilation.
const RELEVANT_ENV_VARS: &[&str] = &[
    "RUSTFLAGS",
    "CARGO_ENCODED_RUSTFLAGS",
    "RUSTC",
    "RUSTC_WRAPPER",
    "RUSTC_LINKER",
    "CARGO_TARGET_DIR",
    "CARGO_BUILD_TARGET",
    "CARGO_FEATURE_",
    "CARGO_CFG_",
    "CC",
    "CXX",
    "AR",
    "LD",
    "PKG_CONFIG_PATH",
];

/// A prepared Cargo invocation.
#[derive(Debug, Clone, Default)]
pub struct CargoInvocation {
    pub cwd: PathBuf,
    pub subcommand: String,
    pub args: Vec<String>,
}

impl CargoInvocation {
    /// Create a new invocation for `cargo <subcommand>` in `cwd`.
    pub fn new(cwd: impl Into<PathBuf>, subcommand: impl Into<String>) -> Self {
        Self {
            cwd: cwd.into(),
            subcommand: subcommand.into(),
            args: Vec::new(),
        }
    }

    /// Add an argument, mirroring `cargo <subcommand> <arg>`.
    pub fn arg(mut self, arg: impl Into<String>) -> Self {
        self.args.push(arg.into());
        self
    }

    /// Run the invocation and return the path to the configured target directory.
    pub fn run(&self) -> Result<PathBuf, String> {
        let mut cmd = Command::new("cargo");
        cmd.arg(&self.subcommand)
            .args(&self.args)
            .current_dir(&self.cwd);

        let status = cmd
            .status()
            .map_err(|e| format!("failed to spawn cargo: {e}"))?;
        if !status.success() {
            return Err(format!("cargo {} exited with {status}", self.subcommand));
        }

        target_dir(&self.cwd)
    }
}

/// Return the target directory for a Cargo project.
pub fn target_dir(cwd: &Path) -> Result<PathBuf, String> {
    // CARGO_TARGET_DIR overrides everything when cargo itself runs, but for a
    // cache lookup we also honour the environment variable.
    if let Ok(dir) = std::env::var("CARGO_TARGET_DIR") {
        return Ok(PathBuf::from(dir));
    }
    Ok(cwd.join("target"))
}

/// Extract the package name from a `Cargo.toml`.
fn package_name(crate_root: &Path) -> Result<String, String> {
    let manifest = crate_root.join("Cargo.toml");
    let text =
        fs::read_to_string(&manifest).map_err(|e| format!("read {}: {e}", manifest.display()))?;
    for line in text.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix("name = ") {
            let name = rest.trim().trim_matches('"').to_string();
            if !name.is_empty() {
                return Ok(name);
            }
        }
    }
    Err("package name not found in Cargo.toml".into())
}

/// Map Cargo's internal profile name to the directory name it uses under
/// `target`. The default `dev` profile is emitted as `debug`.
fn cargo_profile_dir(profile: &str) -> &str {
    if profile == "dev" { "debug" } else { profile }
}

/// Locate the primary artifact produced by Cargo for the crate rooted at
/// `crate_root` under `profile`. For library crates this is
/// `target/<profile>/lib<name>.rlib`; for binary crates it is
/// `target/<profile>/<name>`.
pub fn locate_main_artifact(crate_root: &Path, profile: &str) -> Result<PathBuf, String> {
    let target = target_dir(crate_root)?;
    let dir = cargo_profile_dir(profile);
    let name = package_name(crate_root)?;
    let lib_artifact = target.join(dir).join(format!("lib{name}.rlib"));
    if lib_artifact.exists() {
        return Ok(lib_artifact);
    }
    let bin_artifact = target.join(dir).join(&name);
    if bin_artifact.exists() {
        return Ok(bin_artifact);
    }
    Err(format!(
        "could not locate main artifact for {name} (looked at {} and {})",
        lib_artifact.display(),
        bin_artifact.display()
    ))
}

/// Run `rustc --version --verbose` and return its output.
pub fn rustc_fingerprint() -> Result<String, String> {
    let output = Command::new("rustc")
        .args(["--version", "--verbose"])
        .output()
        .map_err(|e| format!("failed to run rustc --version: {e}"))?;
    if !output.status.success() {
        return Err("rustc --version failed".into());
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

/// Run `cargo --version` and return its output.
pub fn cargo_version() -> Result<String, String> {
    let output = Command::new("cargo")
        .arg("--version")
        .output()
        .map_err(|e| format!("failed to run cargo --version: {e}"))?;
    if !output.status.success() {
        return Err("cargo --version failed".into());
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

/// Fingerprint every `.rs` source file under the crate root.
pub fn source_fingerprint(crate_root: &Path) -> Result<BTreeMap<String, String>, String> {
    let src = crate_root.join("src");
    fingerprint_sources(&src)
}

/// Extract a simplified dependency identity map from `Cargo.lock` if present.
pub fn dependency_identities(crate_root: &Path) -> Result<BTreeMap<String, String>, String> {
    let mut map = BTreeMap::new();
    let lock = crate_root.join("Cargo.lock");
    if !lock.exists() {
        return Ok(map);
    }
    let text = std::fs::read_to_string(&lock).map_err(|e| format!("read Cargo.lock: {e}"))?;
    // Parse the legacy/compact TOML-ish Cargo.lock format enough to extract
    // [[package]] name/version pairs. We avoid pulling in a TOML parser.
    let mut name = None;
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("[[package]]") {
            name = None;
        } else if let Some(rest) = trimmed.strip_prefix("name = ") {
            name = Some(rest.trim().trim_matches('"').to_string());
        } else if let Some(rest) = trimmed.strip_prefix("version = ") {
            if let Some(n) = name.take() {
                let v = rest.trim().trim_matches('"').to_string();
                map.insert(n, v);
            }
        }
    }
    Ok(map)
}

/// Extract features enabled from the environment and CLI-style arguments.
pub fn enabled_features<I, S>(args: I) -> Vec<String>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let mut features = Vec::new();
    let mut iter = args.into_iter().peekable();
    while let Some(arg) = iter.next() {
        let arg = arg.as_ref();
        if arg == "--features" || arg == "-F" {
            if let Some(val) = iter.next() {
                let val = val.as_ref();
                for f in val.split(',') {
                    let f = f.trim();
                    if !f.is_empty() {
                        features.push(f.to_string());
                    }
                }
            }
        } else if let Some(rest) = arg.strip_prefix("--features=") {
            for f in rest.split(',') {
                let f = f.trim();
                if !f.is_empty() {
                    features.push(f.to_string());
                }
            }
        }
    }
    features.sort_unstable();
    features.dedup();
    features
}

/// Extract RUSTFLAGS from the environment and `-- -F...` style trailing args.
pub fn collect_rustflags<I, S>(trailing: I) -> Vec<String>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let mut flags: Vec<String> = Vec::new();
    if let Ok(env) = std::env::var("RUSTFLAGS") {
        flags.extend(env.split_whitespace().map(String::from));
    }
    for arg in trailing {
        let arg = arg.as_ref();
        if arg == "--" {
            continue;
        }
        flags.push(arg.to_string());
    }
    flags.sort_unstable();
    flags.dedup();
    flags
}

/// Detect the target triple from environment or default to the host.
pub fn target_triple() -> String {
    std::env::var("CARGO_BUILD_TARGET")
        .or_else(|_| std::env::var("TARGET"))
        .unwrap_or_else(|_| "host".to_string())
}

/// Detect the active Cargo profile from arguments.
pub fn active_profile<I, S>(args: I) -> String
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let mut profile = "dev".to_string();
    let mut iter = args.into_iter().peekable();
    while let Some(arg) = iter.next() {
        let arg = arg.as_ref();
        if arg == "--release" {
            profile = "release".to_string();
        } else if arg == "--profile" {
            if let Some(p) = iter.next() {
                profile = p.as_ref().to_string();
            }
        } else if let Some(p) = arg.strip_prefix("--profile=") {
            profile = p.to_string();
        }
    }
    profile
}

/// Detect the linker identity from environment or return an empty string.
pub fn linker_identity() -> String {
    std::env::var("RUSTC_LINKER")
        .or_else(|_| std::env::var("CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_LINKER"))
        .unwrap_or_default()
}

/// Collect relevant environment variables that affect compilation.
pub fn relevant_env() -> BTreeMap<String, String> {
    let mut map = BTreeMap::new();
    for key in RELEVANT_ENV_VARS {
        if let Ok(val) = std::env::var(key) {
            map.insert(key.to_string(), val);
        }
    }
    map
}

/// Build a [`CompilationIdentity`] for the crate rooted at `crate_root`,
/// given the Cargo subcommand arguments.
pub fn identity_for_crate(
    crate_root: &Path,
    cargo_args: &[String],
) -> Result<CompilationIdentity, String> {
    let mut inputs = crate::identity::IdentityInputs::new();

    inputs.source_files = source_fingerprint(crate_root)?;
    inputs.rustc_version = rustc_fingerprint()?;
    inputs.target_triple = target_triple();
    inputs.profile = active_profile(cargo_args);
    inputs.features = enabled_features(cargo_args);
    inputs.rustflags = collect_rustflags(std::iter::empty::<&str>());
    inputs.dependency_identities = dependency_identities(crate_root)?;
    inputs.linker = linker_identity();
    inputs.relevant_env = relevant_env();

    // Build-script outputs are best-effort for the MVP; include a placeholder
    // derived from any existing `OUT_DIR` contents.
    inputs.build_script_outputs = build_script_outputs(crate_root)?;

    Ok(CompilationIdentity::from_inputs(inputs))
}

/// Best-effort fingerprint of build-script outputs.
fn build_script_outputs(crate_root: &Path) -> Result<BTreeMap<String, String>, String> {
    let mut map = BTreeMap::new();
    let out = target_dir(crate_root)?.join("build");
    if !out.exists() {
        return Ok(map);
    }
    for entry in std::fs::read_dir(&out).map_err(|e| format!("read_dir {}: {e}", out.display()))? {
        let entry = entry.map_err(|e| format!("dir entry: {e}"))?;
        if entry.file_type().is_ok_and(|t| t.is_dir()) {
            let name = entry.file_name().to_string_lossy().to_string();
            let marker = entry.path().join("output");
            if marker.exists() {
                let content = std::fs::read_to_string(&marker)
                    .map_err(|e| format!("read {}: {e}", marker.display()))?;
                map.insert(name, crate::identity::fnv1a_hex(content.as_bytes()));
            }
        }
    }
    Ok(map)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn active_profile_defaults_to_dev() {
        assert_eq!(active_profile(["build"]), "dev");
    }

    #[test]
    fn active_profile_detects_release() {
        assert_eq!(active_profile(["build", "--release"]), "release");
    }

    #[test]
    fn active_profile_detects_named_profile() {
        assert_eq!(active_profile(["build", "--profile", "custom"]), "custom");
        assert_eq!(active_profile(["build", "--profile=custom"]), "custom");
    }

    #[test]
    fn enabled_features_parse_space_and_comma() {
        let args = vec!["build", "--features", "a,b", "-F", "c"];
        assert_eq!(
            enabled_features(&args),
            vec!["a".to_string(), "b".to_string(), "c".to_string()]
        );
    }

    #[test]
    fn enabled_features_parse_equals() {
        let args = vec!["build", "--features=a,b"];
        assert_eq!(
            enabled_features(&args),
            vec!["a".to_string(), "b".to_string()]
        );
    }

    #[test]
    fn rustc_fingerprint_non_empty() {
        let fp = rustc_fingerprint().expect("rustc must be installed in test environment");
        assert!(fp.contains("rustc"));
    }
}
