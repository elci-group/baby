//! Integration tests for the `boarish` CLI and library.

use std::fs;
use std::path::PathBuf;
use std::process::Command;

fn boarish_bin() -> PathBuf {
    // Prefer the just-built debug binary when running under cargo test.
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let candidate = manifest_dir.join("target").join("debug").join("boarish");
    if candidate.exists() {
        return candidate;
    }
    // Cargo integration tests run with CARGO_TARGET_DIR set at runtime; fall
    // back to the workspace target directory if present.
    if let Ok(target_dir) = std::env::var("CARGO_TARGET_DIR") {
        let candidate = PathBuf::from(target_dir).join("debug").join("boarish");
        if candidate.exists() {
            return candidate;
        }
    }
    PathBuf::from("boarish")
}

fn run(args: &[&str]) -> Result<String, String> {
    let output = Command::new(boarish_bin())
        .args(args)
        .env("XDG_CACHE_HOME", cache_root_for("cli"))
        .output()
        .map_err(|e| format!("failed to spawn boarish: {e}"))?;
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    if !output.status.success() {
        return Err(format!(
            "boarish {} failed ({}): stdout: {stdout}, stderr: {stderr}",
            args.join(" "),
            output.status
        ));
    }
    Ok(stdout)
}

fn cache_root_for(name: &str) -> String {
    let dir = std::env::temp_dir().join(format!("boarish-int-test-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    dir.to_string_lossy().to_string()
}

#[test]
fn status_prints_version_and_target() {
    let out = run(&["status"]).unwrap();
    assert!(out.contains("Boarish"));
    assert!(out.contains("Target triple"));
}

#[test]
fn doctor_reports_ready() {
    let out = run(&["doctor"]).unwrap();
    assert!(out.contains("Boarish is ready"));
}

#[test]
fn cache_lifecycle() {
    let cache = cache_root_for("lifecycle");

    // Initially empty.
    let out = Command::new(boarish_bin())
        .args(["cache", "status"])
        .env("XDG_CACHE_HOME", &cache)
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("Artifacts: 0"));

    // Clear should succeed even when empty.
    let out = Command::new(boarish_bin())
        .args(["cache", "clear"])
        .env("XDG_CACHE_HOME", &cache)
        .output()
        .unwrap();
    assert!(out.status.success());
}

#[test]
fn library_cache_hit_miss() {
    use boarish::CompilationIdentity;
    use boarish::cache::BoarishCache;
    use boarish::identity::IdentityInputs;

    let cache_dir = PathBuf::from(cache_root_for("lib"));
    let _ = fs::remove_dir_all(&cache_dir);
    let cache = BoarishCache::new(&cache_dir).unwrap();

    let mut inputs = IdentityInputs::new();
    inputs.rustc_version = "rustc test".into();
    inputs.target_triple = "x86_64-unknown-linux-gnu".into();
    inputs.profile = "dev".into();
    let identity = CompilationIdentity::from_inputs(inputs);

    // First resolve: miss.
    let artifact = cache_dir.join("artifact.rlib");
    fs::write(&artifact, b"compiled").unwrap();
    let miss = cache.resolve(&identity, || Ok(artifact.clone()));
    assert!(!miss.hit, "first resolve should miss");

    // Second resolve: hit.
    let hit = cache.resolve(&identity, || Ok(artifact.clone()));
    assert!(hit.hit, "second resolve should hit with identical identity");

    let stats = cache.stats();
    assert_eq!(stats.hits, 1);
    assert_eq!(stats.misses, 1);

    let _ = fs::remove_dir_all(&cache_dir);
}

/// Create a minimal Cargo library crate at `root` named `name`.
fn write_library_crate(root: &std::path::Path, name: &str) {
    let manifest =
        format!("[package]\nname = \"{name}\"\nversion = \"0.1.0\"\nedition = \"2021\"\n");
    fs::create_dir_all(root.join("src")).unwrap();
    fs::write(root.join("Cargo.toml"), manifest).unwrap();
    fs::write(root.join("src/lib.rs"), "pub fn answer() -> i32 { 42 }\n").unwrap();
}

#[test]
fn e2e_library_reused_across_projects() {
    let cache = cache_root_for("e2e");
    let base = std::env::temp_dir().join(format!("boarish-e2e-{}", std::process::id()));
    let _ = fs::remove_dir_all(&base);
    fs::create_dir_all(&base).unwrap();

    let project_a = base.join("shared");
    let project_b = base.join("shared-copy");
    write_library_crate(&project_a, "shared");
    write_library_crate(&project_b, "shared");

    // First build: miss.
    let out = Command::new(boarish_bin())
        .arg("build")
        .current_dir(&project_a)
        .env("XDG_CACHE_HOME", &cache)
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(out.status.success(), "first build failed: {stdout}");
    assert!(
        stdout.contains("cache miss"),
        "first build should be a cache miss: {stdout}"
    );

    // Second build of identical sources in a different project: hit.
    let out = Command::new(boarish_bin())
        .arg("build")
        .current_dir(&project_b)
        .env("XDG_CACHE_HOME", &cache)
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(out.status.success(), "second build failed: {stdout}");
    assert!(
        stdout.contains("cache hit"),
        "second build should reuse the cached artifact: {stdout}"
    );

    let _ = fs::remove_dir_all(&base);
}

#[test]
fn identity_changes_invalidate_cache() {
    use boarish::CompilationIdentity;
    use boarish::cache::BoarishCache;
    use boarish::identity::IdentityInputs;

    let cache_dir = PathBuf::from(cache_root_for("identity"));
    let _ = fs::remove_dir_all(&cache_dir);
    let cache = BoarishCache::new(&cache_dir).unwrap();

    let mut inputs_a = IdentityInputs::new();
    inputs_a.rustc_version = "rustc test".into();
    inputs_a.target_triple = "x86_64-unknown-linux-gnu".into();
    inputs_a.profile = "dev".into();
    inputs_a.features = vec!["a".into()];
    let identity_a = CompilationIdentity::from_inputs(inputs_a);

    let mut inputs_b = IdentityInputs::new();
    inputs_b.rustc_version = "rustc test".into();
    inputs_b.target_triple = "x86_64-unknown-linux-gnu".into();
    inputs_b.profile = "dev".into();
    inputs_b.features = vec!["b".into()];
    let identity_b = CompilationIdentity::from_inputs(inputs_b);

    let artifact = cache_dir.join("artifact.rlib");
    fs::write(&artifact, b"compiled").unwrap();

    cache.resolve(&identity_a, || Ok(artifact.clone()));
    let second = cache.resolve(&identity_b, || Ok(artifact.clone()));
    assert!(!second.hit, "different identity should be a miss");

    let _ = fs::remove_dir_all(&cache_dir);
}
