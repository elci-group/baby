use std::fs;
use std::path::PathBuf;
use std::process::Command;

fn unique_path(base: &str) -> PathBuf {
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("clock after epoch")
        .as_nanos();
    PathBuf::from(base).join(format!("boar-integration-{}-{nonce}", std::process::id()))
}

#[test]
fn ram_build_disk_fallback_status_and_cleanup_lifecycle() {
    let fixture = unique_path("/tmp");
    let ram_root = unique_path("/dev/shm");
    fs::create_dir_all(fixture.join("src")).expect("create fixture source");
    fs::write(
        fixture.join("Cargo.toml"),
        "[package]\nname = \"boar-fixture\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
    )
    .expect("write fixture manifest");
    fs::write(
        fixture.join("src/main.rs"),
        "fn main() { println!(\"boar\"); }\n",
    )
    .expect("write fixture source");

    let binary = env!("CARGO_BIN_EXE_boar");
    let build = Command::new(binary)
        .args([
            "--mode",
            "ram",
            "--reserve-mib",
            "0",
            "--no-monitor",
            "--ram-root",
        ])
        .arg(&ram_root)
        .arg("build")
        .env("BOAR_CACHE_DIR", fixture.join("cache"))
        .current_dir(&fixture)
        .output()
        .expect("run BOAR build");
    assert!(
        build.status.success(),
        "BOAR build failed:\n{}\n{}",
        String::from_utf8_lossy(&build.stdout),
        String::from_utf8_lossy(&build.stderr)
    );

    let target = fs::read_dir(&ram_root)
        .expect("RAM root exists")
        .next()
        .expect("project target exists")
        .expect("read project target")
        .path();
    assert!(target.join(".boar-owner").is_file());
    assert!(target.join("debug/boar-fixture").is_file());
    let estimates: Vec<_> = fs::read_dir(fixture.join("cache/estimates"))
        .expect("estimate cache exists")
        .collect::<Result<_, _>>()
        .expect("read estimate cache");
    assert_eq!(estimates.len(), 1);
    assert!(
        fs::read_to_string(estimates[0].path())
            .expect("read estimate")
            .contains("size_kib=")
    );

    let status = Command::new(binary)
        .args(["--ram-root"])
        .arg(&ram_root)
        .arg("status")
        .env("BOAR_CACHE_DIR", fixture.join("cache"))
        .current_dir(&fixture)
        .output()
        .expect("run BOAR status");
    assert!(status.status.success());
    let status_text = String::from_utf8_lossy(&status.stdout);
    assert!(status_text.contains("boar-integration"));
    assert!(status_text.contains("across 1 project(s)"));

    let clean = Command::new(binary)
        .args(["--ram-root"])
        .arg(&ram_root)
        .arg("clean")
        .env("BOAR_CACHE_DIR", fixture.join("cache"))
        .current_dir(&fixture)
        .output()
        .expect("run BOAR clean");
    assert!(clean.status.success());
    assert!(!target.exists());

    let disk_target = fixture.join("disk-target");
    let fallback = Command::new(binary)
        .args(["--mode", "auto", "--reserve-mib", "1000000", "--ram-root"])
        .arg(&ram_root)
        .arg("--disk-target")
        .arg(&disk_target)
        .arg("build")
        .env("BOAR_CACHE_DIR", fixture.join("cache"))
        .current_dir(&fixture)
        .output()
        .expect("run BOAR fallback build");
    assert!(
        fallback.status.success(),
        "BOAR fallback failed:\n{}\n{}",
        String::from_utf8_lossy(&fallback.stdout),
        String::from_utf8_lossy(&fallback.stderr)
    );
    let fallback_stderr = String::from_utf8_lossy(&fallback.stderr);
    assert!(fallback_stderr.contains("BOAR: disk target"));
    assert!(disk_target.join("debug/boar-fixture").is_file());

    fs::remove_dir_all(&fixture).expect("clean fixture");
    let _ = fs::remove_dir(&ram_root);
}
