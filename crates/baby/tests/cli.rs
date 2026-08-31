// Copyright (c) 2026 sal
// SPDX-License-Identifier: MIT

use assert_cmd::Command;
use predicates::prelude::*;
use std::fs;
use std::path::PathBuf;
use std::sync::OnceLock;

fn bin_command(name: &str) -> Command {
    static CACHE: OnceLock<std::collections::HashMap<String, PathBuf>> = OnceLock::new();
    let map = CACHE.get_or_init(|| {
        let manifest = concat!(env!("CARGO_MANIFEST_DIR"), "/../../Cargo.toml");
        ["baby", "birthctl", "birthd"]
            .into_iter()
            .map(|n| {
                let cargo = escargot::CargoBuild::new()
                    .manifest_path(manifest)
                    .bin(n)
                    .run()
                    .unwrap_or_else(|e| panic!("failed to build {n}: {e}"));
                (n.to_string(), cargo.path().to_path_buf())
            })
            .collect()
    });
    let path = map.get(name).unwrap_or_else(|| panic!("unknown binary: {name}"));
    Command::from_std(std::process::Command::new(path))
}

#[test]
fn baby_help_shows_usage() {
    let mut cmd = bin_command("baby");
    cmd.arg("--help");
    cmd.assert()
        .success()
        .stdout(predicate::str::contains("Build And Bin Yield"));
}

#[test]
fn baby_help_documents_post_install_cleanup_override() {
    let mut cmd = bin_command("baby");
    cmd.arg("--help");
    cmd.assert()
        .success()
        .stdout(predicate::str::contains("--no-clean"));
}

#[test]
fn baby_help_documents_locksmith_flags() {
    let mut cmd = bin_command("baby");
    cmd.arg("--help");
    cmd.assert()
        .success()
        .stdout(predicate::str::contains("--no-lock"))
        .stdout(predicate::str::contains("--lock-timeout"))
        .stdout(predicate::str::contains("--lock-lease"))
        .stdout(predicate::str::contains("--repo-lock-timeout"));
}

#[test]
fn baby_version_prints_semver() {
    let mut cmd = bin_command("baby");
    cmd.arg("--version");
    cmd.assert()
        .success()
        .stdout(predicate::str::is_match(r"^baby \d+\.\d+\.\d+\n$").unwrap());
}

#[test]
fn baby_dry_run_in_temp_project() {
    let root = tempfile::tempdir().unwrap();
    let project_dir = root.path().join("dummy");
    fs::create_dir(&project_dir).unwrap();
    fs::write(
        project_dir.join("Cargo.toml"),
        r#"[package]
name = "dummy"
version = "0.1.0"
edition = "2021"
"#,
    )
    .unwrap();

    let mut cmd = bin_command("baby");
    cmd.current_dir(&project_dir).arg("--dry-run").arg("--user");
    cmd.assert()
        .success()
        .stderr(predicate::str::contains("installation recipe resolved:"))
        .stderr(predicate::str::contains("binary=dummy"));
}

#[test]
fn birthctl_help_shows_usage() {
    let mut cmd = bin_command("birthctl");
    cmd.arg("--help");
    cmd.assert()
        .success()
        .stdout(predicate::str::contains("birthd build daemon"));
}

#[test]
fn birthctl_status_when_not_running() {
    let mut cmd = bin_command("birthctl");
    cmd.arg("status");
    // Should succeed even when daemon is not running.
    cmd.assert()
        .success()
        .stderr(predicate::str::contains("birthd is not running"));
}

#[test]
fn birthctl_reload_when_not_running_fails() {
    let mut cmd = bin_command("birthctl");
    cmd.arg("reload");
    cmd.assert()
        .failure()
        .stderr(predicate::str::contains("birthd is not running"));
}

#[test]
fn birthctl_stop_when_not_running_fails() {
    let mut cmd = bin_command("birthctl");
    cmd.arg("stop");
    cmd.assert()
        .failure()
        .stderr(predicate::str::contains("birthd is not running"));
}

#[test]
fn birthctl_watch_creates_config() {
    let dir = tempfile::tempdir().unwrap();
    let mut cmd = bin_command("birthctl");
    cmd.current_dir(&dir)
        .arg("watch")
        .arg("--project")
        .arg("demo")
        .arg("--path")
        .arg("src")
        .arg("--install")
        .arg("/tmp/demo-bin");
    cmd.assert()
        .success()
        .stderr(predicate::str::contains("created .birth.toml"));

    let toml_path = dir.path().join(".birth.toml");
    assert!(toml_path.exists());
    let content = fs::read_to_string(toml_path).unwrap();
    assert!(content.contains("project = \"demo\""));
    assert!(content.contains("install = \"/tmp/demo-bin\""));
}

#[test]
fn birthctl_watch_requires_path() {
    let dir = tempfile::tempdir().unwrap();
    let mut cmd = bin_command("birthctl");
    cmd.current_dir(&dir)
        .arg("watch")
        .arg("--project")
        .arg("demo");
    cmd.assert().failure().stderr(predicate::str::contains(
        "watch requires at least one --path",
    ));
}

#[test]
fn birthd_help_shows_usage() {
    let mut cmd = bin_command("birthd");
    cmd.arg("--help");
    cmd.assert()
        .success()
        .stdout(predicate::str::contains("build daemon"));
}

#[test]
fn birthd_generate_man() {
    let dir = tempfile::tempdir().unwrap();
    let man_path = dir.path().join("birthd.1");
    let mut cmd = bin_command("birthd");
    cmd.arg("--generate-man").arg(&man_path);
    cmd.assert().success();
    assert!(man_path.exists());
    let content = fs::read_to_string(man_path).unwrap();
    assert!(content.contains("birthd"));
}
