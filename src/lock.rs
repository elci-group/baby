// Copyright (c) 2026 sal
// SPDX-License-Identifier: MIT

//! Coordination with `locksmithd` so concurrent `baby` runs for the same
//! project wait for each other instead of colliding.
//!
//! Locking is best-effort: if the `locksmith` CLI is not installed or the
//! daemon cannot be reached, `baby` proceeds with a warning. When locking is
//! available, `baby` first waits for every active locksmith lease whose
//! resource names embed the canonical project root to clear, then acquires an
//! exclusive lease on a resource derived from that path, heartbeats while the
//! build runs, and releases the lease on completion or panic.

use std::path::Path;
use std::process::Command;
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use crate::error::{BabyError, Result};

/// Default seconds to wait in the locksmith queue before giving up.
pub const DEFAULT_TIMEOUT_SECS: u64 = 300;
/// Default lease duration. Heartbeats keep the lease alive during long builds.
pub const DEFAULT_LEASE_SECS: u64 = 20 * 60;
/// Seconds between heartbeats while a lock is held.
const HEARTBEAT_INTERVAL_SECS: u64 = 30;
/// Seconds between polls while waiting for repo-wide locks to clear.
pub const REPO_UNLOCK_POLL_INTERVAL_SECS: u64 = 3;

#[allow(clippy::assertions_on_constants)]
const _: () = {
    assert!(DEFAULT_TIMEOUT_SECS > 0);
    assert!(DEFAULT_LEASE_SECS > HEARTBEAT_INTERVAL_SECS);
};

/// RAII handle for a locksmith lease. Releases the lease and stops the
/// heartbeat thread when dropped.
pub struct LockGuard {
    resource: String,
    stop_tx: Option<mpsc::Sender<()>>,
    heartbeat_handle: Option<thread::JoinHandle<()>>,
}

impl LockGuard {
    fn new(resource: String) -> Self {
        let (stop_tx, rx) = mpsc::channel();
        let heartbeat_resource = resource.clone();
        let handle = thread::spawn(move || heartbeat_loop(&heartbeat_resource, rx));
        Self {
            resource,
            stop_tx: Some(stop_tx),
            heartbeat_handle: Some(handle),
        }
    }
}

impl Drop for LockGuard {
    fn drop(&mut self) {
        if let Some(tx) = self.stop_tx.take() {
            let _ = tx.send(());
        }
        if let Some(handle) = self.heartbeat_handle.take() {
            let _ = handle.join();
        }
        if let Err(e) = release(&self.resource) {
            log::warn!(
                "failed to release locksmith lease for {}: {}",
                self.resource,
                e
            );
        }
    }
}

/// Try to acquire an exclusive build lock for `project_root`.
///
/// Returns `Ok(None)` when locksmith is unavailable or the daemon cannot be
/// reached, so `baby` can proceed without coordination. Returns
/// `Ok(Some(guard))` on success, or `Err` if the wait timed out or another
/// unexpected failure occurred.
pub fn acquire_build_lock(
    project_root: &Path,
    project_name: &str,
    timeout_secs: u64,
    lease_secs: u64,
) -> Result<Option<LockGuard>> {
    if !locksmith_available() {
        log::debug!("locksmith CLI not found; proceeding without coordination");
        return Ok(None);
    }

    let resource = resource_name(project_root, project_name);
    log::info!("acquiring locksmith lease for {}", resource);

    let mut cmd = Command::new("locksmith");
    cmd.arg("acquire")
        .arg(&resource)
        .arg("--wait")
        .arg("--wait-timeout")
        .arg(timeout_secs.to_string())
        .arg("--lease")
        .arg(format!("{}s", lease_secs));

    let output = cmd
        .output()
        .map_err(|e| BabyError::io("run locksmith acquire", e))?;

    if output.status.success() {
        return Ok(Some(LockGuard::new(resource)));
    }

    let stderr = String::from_utf8_lossy(&output.stderr);
    if stderr.contains("timed out") || stderr.contains("wait timeout") {
        return Err(BabyError::lock_timeout(&resource));
    }
    if stderr.contains("cannot reach daemon") {
        log::warn!(
            "locksmithd is unreachable; proceeding without coordination for {}",
            resource
        );
        return Ok(None);
    }

    Err(BabyError::new(
        crate::error::ErrorKind::LockTimeout,
        format!(
            "locksmith acquire failed for {}: {}",
            resource,
            stderr.trim()
        ),
    ))
}

/// Wait until no active locksmith lease names the canonical `repo_root` path.
///
/// This is a pre-flight gate: before `baby` mutates the project, it polls
/// `locksmith leases --json` and blocks while any lease's resource string
/// contains the canonical repo path. The check catches `baby`'s own
/// `baby:{project}@{path}` leases as well as path-based resources from other
/// tools.
///
/// Best-effort: if the `locksmith` CLI is missing or the daemon cannot be
/// reached, `baby` proceeds with a warning. Returns `Err` only when the wait
/// exceeds `timeout_secs`.
pub fn wait_for_repo_unlock(repo_root: &Path, timeout_secs: u64) -> Result<()> {
    if !locksmith_available() {
        log::debug!("locksmith CLI not found; skipping repo unlock wait");
        return Ok(());
    }

    let canonical = repo_root
        .canonicalize()
        .unwrap_or_else(|_| repo_root.to_path_buf());
    let repo_str = canonical.to_string_lossy();
    log::info!("waiting for repo locks on {} to clear", repo_str);

    let deadline = Instant::now() + Duration::from_secs(timeout_secs);
    loop {
        match list_active_leases() {
            Ok(leases) => {
                let blocking = leases_for_repo(&leases, &repo_str);
                if blocking.is_empty() {
                    log::debug!("no active locksmith leases on {}; proceeding", repo_str);
                    return Ok(());
                }
                for lease in &blocking {
                    log::info!(
                        "repo still locked by {} on {} ({})",
                        lease.owner,
                        lease.resource,
                        lease.mode
                    );
                }
            }
            Err(e) => {
                log::warn!(
                    "could not list locksmith leases for repo wait; proceeding: {}",
                    e
                );
                return Ok(());
            }
        }

        if Instant::now() >= deadline {
            return Err(BabyError::lock_timeout(format!(
                "repo {} (timeout: {}s)",
                repo_str, timeout_secs
            )));
        }

        thread::sleep(Duration::from_secs(REPO_UNLOCK_POLL_INTERVAL_SECS));
    }
}

/// Minimal representation of a locksmith lease for filtering.
#[derive(serde::Deserialize)]
struct LeaseSummary {
    resource: String,
    owner: String,
    mode: String,
}

/// Fetch all active leases from `locksmithd`.
fn list_active_leases() -> Result<Vec<LeaseSummary>> {
    let output = Command::new("locksmith")
        .arg("leases")
        .arg("--json")
        .output()
        .map_err(|e| BabyError::io("run locksmith leases", e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(BabyError::new(
            crate::error::ErrorKind::LockTimeout,
            format!("locksmith leases failed: {}", stderr.trim()),
        ));
    }

    serde_json::from_slice(&output.stdout).map_err(|e| {
        BabyError::new(
            crate::error::ErrorKind::LockTimeout,
            format!("parse locksmith leases JSON: {}", e),
        )
    })
}

/// Return every lease whose resource names embed `repo_path`.
fn leases_for_repo<'a>(leases: &'a [LeaseSummary], repo_path: &str) -> Vec<&'a LeaseSummary> {
    leases
        .iter()
        .filter(|l| l.resource.contains(repo_path))
        .collect()
}

/// Release a locksmith lease. Used directly by the guard's `Drop` impl.
fn release(resource: &str) -> Result<()> {
    let status = Command::new("locksmith")
        .arg("release")
        .arg(resource)
        .status()
        .map_err(|e| BabyError::io("run locksmith release", e))?;

    if !status.success() {
        return Err(BabyError::new(
            crate::error::ErrorKind::LockTimeout,
            format!("locksmith release failed for {}", resource),
        ));
    }
    Ok(())
}

/// Send a heartbeat for the held lease. Errors are logged by the caller.
fn heartbeat(resource: &str) -> Result<()> {
    let status = Command::new("locksmith")
        .arg("heartbeat")
        .arg(resource)
        .status()
        .map_err(|e| BabyError::io("run locksmith heartbeat", e))?;

    if !status.success() {
        return Err(BabyError::new(
            crate::error::ErrorKind::LockTimeout,
            format!("locksmith heartbeat failed for {}", resource),
        ));
    }
    Ok(())
}

/// Background heartbeat loop. Exits when `stop` signals or disconnects.
fn heartbeat_loop(resource: &str, stop: mpsc::Receiver<()>) {
    loop {
        match stop.recv_timeout(Duration::from_secs(HEARTBEAT_INTERVAL_SECS)) {
            Ok(()) | Err(mpsc::RecvTimeoutError::Disconnected) => break,
            Err(mpsc::RecvTimeoutError::Timeout) => {
                if let Err(e) = heartbeat(resource) {
                    log::warn!("locksmith heartbeat failed for {}: {}", resource, e);
                }
            }
        }
    }
}

/// Whether the `locksmith` executable is on `PATH`.
fn locksmith_available() -> bool {
    Command::new("locksmith")
        .arg("--version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

/// Build a deterministic resource name from the canonical project path.
///
/// The `baby:` prefix makes this an opaque locksmith resource owned by the
/// `baby` tool; the canonical path ensures two shells in the same project
/// directory contend for the same lease.
fn resource_name(project_root: &Path, project_name: &str) -> String {
    let canonical = project_root
        .canonicalize()
        .unwrap_or_else(|_| project_root.to_path_buf());
    format!("baby:{}@{}", project_name, canonical.display())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resource_name_is_deterministic() {
        let dir = tempfile::tempdir().unwrap();
        let name = resource_name(dir.path(), "widget");
        assert!(name.starts_with("baby:widget@"));
        assert!(name.contains(dir.path().canonicalize().unwrap().to_str().unwrap()));
    }

    #[test]
    fn resource_name_uses_fallback_when_not_canonicalizable() {
        let path = Path::new("/definitely/not/a/real/path/for/baby/lock/test");
        let name = resource_name(path, "widget");
        assert_eq!(name, format!("baby:widget@{}", path.display()));
    }

    #[test]
    fn leases_for_repo_matches_baby_and_path_style_resources() {
        let repo = "/home/sal/baby";
        let leases = vec![
            LeaseSummary {
                resource: format!("baby:baby@{repo}"),
                owner: "sal".into(),
                mode: "exclusive".into(),
            },
            LeaseSummary {
                resource: format!("{repo}/src/main.rs"),
                owner: "agent".into(),
                mode: "shared".into(),
            },
            LeaseSummary {
                resource: "gpu:0".into(),
                owner: "ci".into(),
                mode: "exclusive".into(),
            },
        ];
        let blocking = leases_for_repo(&leases, repo);
        assert_eq!(blocking.len(), 2);
        assert!(blocking.iter().any(|l| l.resource.starts_with("baby:")));
        assert!(blocking.iter().any(|l| l.resource.ends_with("src/main.rs")));
    }

    #[test]
    fn leases_for_repo_ignores_unrelated_resources() {
        let repo = "/home/sal/baby";
        let leases = vec![
            LeaseSummary {
                resource: "/home/sal/locksmith/src/lib.rs".into(),
                owner: "sal".into(),
                mode: "exclusive".into(),
            },
            LeaseSummary {
                resource: "deploy:prod".into(),
                owner: "ci".into(),
                mode: "exclusive".into(),
            },
        ];
        assert!(leases_for_repo(&leases, repo).is_empty());
    }
}
