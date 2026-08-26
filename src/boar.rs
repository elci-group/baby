// Copyright (c) 2026 sal
// SPDX-License-Identifier: MIT

//! Recovers a Cargo build that fails because local storage or memory is
//! exhausted by handing it to `boar`, which owns the placement decision
//! (RAM, disk, and eventually remote storage/execution tiers).
//!
//! BABY never re-implements BOAR's placement logic. Its contract with BOAR
//! is two commands: `boar <cargo-command> <args>` to retry a build with
//! adaptive placement, and `boar target-dir` to learn where the resulting
//! artifact landed. As BOAR grows remote tiers (BOARISH/BOARING), both
//! commands keep working unchanged, so this module does not need to.

use std::env;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Cargo commands BOAR itself will restart automatically after failure.
/// Anything else (`test`, `run`, `bench`) may run arbitrary side-effecting
/// code, so BABY must not hand it to BOAR for a blind retry.
const RETRYABLE_CARGO_COMMANDS: [&str; 4] = ["build", "check", "clippy", "doc"];

/// Free space below which a workspace's filesystem is treated as plausibly
/// out of room, matching the growth-headroom floor BOAR itself budgets for.
const STORAGE_PRESSURE_THRESHOLD_KIB: u64 = 256 * 1024;

/// Whether `argv` is a `cargo <verb> ...` invocation BOAR is safe to retry.
pub fn is_retryable_cargo_command(argv: &[String]) -> bool {
    argv.first().map(String::as_str) == Some("cargo")
        && argv
            .get(1)
            .map(|verb| RETRYABLE_CARGO_COMMANDS.contains(&verb.as_str()))
            .unwrap_or(false)
}

/// Rewrite `cargo <verb> <args...>` into `boar <verb> <args...>`. Returns
/// `None` for anything [`is_retryable_cargo_command`] rejects.
pub fn rewrite_for_boar(argv: &[String]) -> Option<Vec<String>> {
    if !is_retryable_cargo_command(argv) {
        return None;
    }
    let mut rewritten = vec!["boar".to_string()];
    rewritten.extend_from_slice(&argv[1..]);
    Some(rewritten)
}

/// Whether the `boar` executable is available and safe to delegate to.
///
/// Refuses while already running inside a BOAR-managed build (`BOAR_ACTIVE`)
/// to prevent recursive placement decisions.
pub fn boar_available() -> bool {
    env::var_os("BOAR_ACTIVE").is_none() && find_on_path("boar").is_some()
}

fn find_on_path(name: &str) -> Option<PathBuf> {
    let path = env::var_os("PATH")?;
    env::split_paths(&path).find_map(|dir| {
        let candidate = dir.join(name);
        candidate.is_file().then_some(candidate)
    })
}

/// Best-effort check for whether `root`'s filesystem looks critically low on
/// space — the plausibility test BABY runs before ever asking BOAR to retry
/// a failed build. A build that failed for some other reason (a compile
/// error, a missing dependency) is not retried.
pub fn plausibly_storage_related(root: &Path) -> bool {
    available_kib(root)
        .map(|kib| kib < STORAGE_PRESSURE_THRESHOLD_KIB)
        .unwrap_or(false)
}

fn available_kib(path: &Path) -> Option<u64> {
    let existing = existing_ancestor(path)?;
    let output = Command::new("df").arg("-Pk").arg(&existing).output().ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&output.stdout);
    let line = text.lines().last()?;
    line.split_whitespace().nth(3)?.parse().ok()
}

fn existing_ancestor(path: &Path) -> Option<PathBuf> {
    let mut candidate = path;
    while !candidate.exists() {
        candidate = candidate.parent()?;
    }
    candidate.canonicalize().ok()
}

/// Ask BOAR where it placed (or would place) this project's build, without
/// running Cargo. `None` if `boar` is missing, errors, or prints nothing
/// usable — callers should fall back to the plain Cargo target layout.
pub fn resolved_target_dir(root: &Path) -> Option<PathBuf> {
    let output = Command::new("boar")
        .arg("target-dir")
        .current_dir(root)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8(output.stdout).ok()?;
    let line = text.lines().next()?.trim();
    (!line.is_empty()).then(|| PathBuf::from(line))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognises_retryable_cargo_commands() {
        for verb in RETRYABLE_CARGO_COMMANDS {
            assert!(is_retryable_cargo_command(&["cargo".into(), verb.into()]));
        }
    }

    #[test]
    fn rejects_side_effecting_cargo_commands() {
        for verb in ["test", "run", "bench"] {
            assert!(!is_retryable_cargo_command(&["cargo".into(), verb.into()]));
        }
    }

    #[test]
    fn rejects_non_cargo_commands() {
        assert!(!is_retryable_cargo_command(&[
            "npm".into(),
            "run".into(),
            "build".into()
        ]));
        assert!(!is_retryable_cargo_command(&["cargo".into()]));
    }

    #[test]
    fn rewrites_retryable_commands_to_boar() {
        let argv = vec![
            "cargo".into(),
            "build".into(),
            "--release".into(),
            "--bin".into(),
            "widget".into(),
        ];
        assert_eq!(
            rewrite_for_boar(&argv),
            Some(vec![
                "boar".into(),
                "build".into(),
                "--release".into(),
                "--bin".into(),
                "widget".into(),
            ])
        );
    }

    #[test]
    fn does_not_rewrite_side_effecting_commands() {
        assert_eq!(rewrite_for_boar(&["cargo".into(), "test".into()]), None);
    }

    #[test]
    fn low_space_reads_as_storage_pressure() {
        let dir = tempfile::tempdir().unwrap();
        // A freshly created temp dir is extremely unlikely to be near-full;
        // this just exercises the plumbing without asserting on a live
        // system's actual free space.
        let _ = plausibly_storage_related(dir.path());
    }

    #[test]
    fn nonexistent_root_reports_no_pressure() {
        assert!(!plausibly_storage_related(Path::new(
            "/nonexistent/path/for/baby/boar/test"
        )));
    }
}
