//! `boarish` CLI: a Rust/Cargo compilation cache built on Boaring.

use std::env;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::Duration;

use boarish::cache::{BoarishCache, format_time};
use boarish::cargo::{
    CargoInvocation, cargo_version, identity_for_crate, locate_main_artifact, target_triple,
};
use boarish::rustc_fingerprint;

const USAGE: &str = "Usage:
  boarish status
  boarish cache status
  boarish cache list
  boarish cache inspect <id>
  boarish cache verify
  boarish cache prune [--max-age <seconds>] [--max-size <bytes>]
  boarish cache clear
  boarish gc
  boarish stats
  boarish doctor
  boarish build [<cargo-args>...]
  boarish cargo <subcommand> [<cargo-args>...]
";

fn main() -> ExitCode {
    let args: Vec<String> = env::args().skip(1).collect();
    if args.is_empty() {
        eprintln!("{USAGE}");
        return ExitCode::from(2);
    }

    match run(&args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("boarish: error: {e}");
            ExitCode::from(1)
        }
    }
}

fn run(args: &[String]) -> Result<(), String> {
    match args.first().map(String::as_str) {
        Some("status") => cmd_status(),
        Some("cache") => match args.get(1).map(String::as_str) {
            Some("status") => cmd_cache_status(),
            Some("list") => cmd_cache_list(),
            Some("inspect") => cmd_cache_inspect(&args[2..]),
            Some("verify") => cmd_cache_verify(),
            Some("prune") => cmd_cache_prune(&args[2..]),
            Some("clear") => cmd_cache_clear(),
            _ => Err(format!("unknown cache subcommand\n{USAGE}")),
        },
        Some("gc") => cmd_cache_prune(&[]),
        Some("stats") => cmd_stats(),
        Some("doctor") => cmd_doctor(),
        Some("build") => cmd_build(&args[1..]),
        Some("cargo") => {
            let rest = &args[1..];
            if rest.is_empty() {
                return Err(format!("missing cargo subcommand\n{USAGE}"));
            }
            cmd_cargo(rest)
        }
        Some("--help" | "-h") => {
            println!("{USAGE}");
            Ok(())
        }
        Some(other) => Err(format!("unknown command: {other}\n{USAGE}")),
        None => Err(format!("missing command\n{USAGE}")),
    }
}

fn open_cache() -> Result<BoarishCache, String> {
    let base = BoarishCache::default_location()?;
    BoarishCache::new(&base)
}

fn cmd_status() -> Result<(), String> {
    println!("Boarish {}", env!("CARGO_PKG_VERSION"));
    println!(
        "Cache directory: {}",
        BoarishCache::default_location()?.display()
    );
    println!("Rustc: {}", rustc_fingerprint()?);
    println!("Cargo: {}", cargo_version()?);
    println!("Target triple: {}", target_triple());

    match open_cache() {
        Ok(cache) => {
            let status = cache.status();
            println!("Artifacts: {}", status.artifact_count);
            println!("Total bytes: {}", status.total_bytes);
            println!(
                "Requests: {} (hits {}, misses {})",
                status.requests, status.hits, status.misses
            );
        }
        Err(e) => println!("Cache not yet accessible: {e}"),
    }
    Ok(())
}

fn cmd_cache_status() -> Result<(), String> {
    let cache = open_cache()?;
    let status = cache.status();
    println!("Cache directory: {}", cache.base().display());
    println!("Artifacts: {}", status.artifact_count);
    println!("Total bytes: {}", status.total_bytes);
    println!("Requests: {}", status.requests);
    println!("Hits: {}", status.hits);
    println!("Misses: {}", status.misses);
    println!("Validation failures: {}", status.validation_failures);
    Ok(())
}

fn cmd_cache_list() -> Result<(), String> {
    let cache = open_cache()?;
    let items = cache.list();
    if items.is_empty() {
        println!("No cached artifacts.");
        return Ok(());
    }
    println!("{:<64} {:<16} Created", "ID", "Digest");
    for (id, manifest) in items {
        println!(
            "{:<64} {:<16} {}",
            id,
            manifest.content_digest.chars().take(16).collect::<String>(),
            format_time(manifest.created_at)
        );
    }
    Ok(())
}

fn cmd_cache_inspect(args: &[String]) -> Result<(), String> {
    let id = args.first().ok_or("missing artifact id\n{USAGE}")?;
    let cache = open_cache()?;
    let manifest = cache
        .inspect(id)
        .ok_or_else(|| format!("artifact {id} not found"))?;
    println!("{:#?}", manifest);
    Ok(())
}

fn cmd_cache_verify() -> Result<(), String> {
    let cache = open_cache()?;
    let results = cache.verify();
    let total = results.len();
    let ok = results.iter().filter(|(_, v)| *v).count();
    for (id, valid) in results {
        println!("{} {}", if valid { "OK  " } else { "FAIL" }, id);
    }
    println!("Verified {ok}/{total} artifacts");
    if ok != total {
        return Err("one or more artifacts failed verification".into());
    }
    Ok(())
}

fn cmd_cache_prune(args: &[String]) -> Result<(), String> {
    let mut max_age = Duration::from_secs(7 * 24 * 60 * 60); // 1 week
    let mut max_size = u64::MAX;

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--max-age" => {
                let v = args.get(i + 1).ok_or("--max-age requires a value")?;
                max_age = Duration::from_secs(v.parse().map_err(|_| "invalid --max-age")?);
                i += 2;
            }
            "--max-size" => {
                let v = args.get(i + 1).ok_or("--max-size requires a value")?;
                max_size = v.parse().map_err(|_| "invalid --max-size")?;
                i += 2;
            }
            other => return Err(format!("unknown prune option: {other}")),
        }
    }

    let cache = open_cache()?;
    let removed = cache.prune(max_age, max_size)?;
    println!("Pruned {removed} artifact(s).");
    Ok(())
}

fn cmd_cache_clear() -> Result<(), String> {
    let cache = open_cache()?;
    cache.clear()?;
    println!("Cache cleared.");
    Ok(())
}

fn cmd_stats() -> Result<(), String> {
    let cache = open_cache()?;
    let t = cache.stats();
    let total = t.hits + t.misses;
    let hit_rate = if total == 0 {
        0.0
    } else {
        (t.hits as f64 / total as f64) * 100.0
    };
    println!("Requests: {}", t.requests);
    println!("Hits: {}", t.hits);
    println!("Misses: {}", t.misses);
    println!("Validation failures: {}", t.validation_failures);
    println!("Bytes stored: {}", t.bytes_stored);
    println!("Hit rate: {:.1}%", hit_rate);
    Ok(())
}

fn cmd_doctor() -> Result<(), String> {
    println!("Boarish Doctor");
    println!("==============");

    let mut ok = true;

    match rustc_fingerprint() {
        Ok(v) => println!("[OK] rustc available: {v}"),
        Err(e) => {
            println!("[FAIL] rustc: {e}");
            ok = false;
        }
    }

    match cargo_version() {
        Ok(v) => println!("[OK] cargo available: {v}"),
        Err(e) => {
            println!("[FAIL] cargo: {e}");
            ok = false;
        }
    }

    match BoarishCache::default_location() {
        Ok(p) => {
            if let Err(e) = std::fs::create_dir_all(&p) {
                println!("[FAIL] cannot create cache dir {}: {e}", p.display());
                ok = false;
            } else {
                println!("[OK] cache directory: {}", p.display());
            }
        }
        Err(e) => {
            println!("[FAIL] cache location: {e}");
            ok = false;
        }
    }

    if ok {
        println!("\nBoarish is ready.");
        Ok(())
    } else {
        Err("doctor found problems".into())
    }
}

fn cmd_build(args: &[String]) -> Result<(), String> {
    let cwd = std::env::current_dir().map_err(|e| format!("current_dir: {e}"))?;
    cmd_cargo_impl("build", args, &cwd)
}

fn cmd_cargo(args: &[String]) -> Result<(), String> {
    let cwd = std::env::current_dir().map_err(|e| format!("current_dir: {e}"))?;
    let subcommand = args.first().cloned().unwrap_or_else(|| "build".to_string());
    cmd_cargo_impl(&subcommand, &args[1..], &cwd)
}

fn cmd_cargo_impl(subcommand: &str, args: &[String], cwd: &Path) -> Result<(), String> {
    let manifest_path = locate_cargo_toml(cwd)?;
    let crate_root = manifest_path.parent().unwrap_or(cwd).to_path_buf();

    let identity =
        identity_for_crate(&crate_root, args).map_err(|e| format!("identity error: {e}"))?;
    println!("{}", identity.explain());

    let cache = open_cache()?;
    let outcome = cache.resolve(&identity, || {
        // Miss path: run Cargo normally, then return the path to the produced artifact.
        println!(
            "cache miss: running cargo {} {}",
            subcommand,
            args.join(" ")
        );
        let invocation = CargoInvocation::new(&crate_root, subcommand)
            .arg("--manifest-path")
            .arg(manifest_path.to_string_lossy().to_string());
        let invocation = args.iter().fold(invocation, |inv, a| inv.arg(a));
        invocation.run()?;
        locate_main_artifact(&crate_root, &identity.inputs.profile)
    });

    println!("{}", outcome.reason.explain(&identity.id));
    if !outcome.hit {
        println!("produced: {}", outcome.artifact_path.display());
    } else {
        println!("reusing: {}", outcome.artifact_path.display());
        // For a hit, copy the cached artifact into the local target directory so
        // downstream Cargo steps can find it.
        if let Err(e) =
            copy_artifact_to_target(&outcome.artifact_path, cwd, &identity.inputs.profile)
        {
            eprintln!("boarish: warning: could not copy cached artifact to target: {e}");
        }
    }

    Ok(())
}

fn locate_cargo_toml(cwd: &Path) -> Result<PathBuf, String> {
    let direct = cwd.join("Cargo.toml");
    if direct.exists() {
        return Ok(direct);
    }
    let mut current = cwd;
    loop {
        let candidate = current.join("Cargo.toml");
        if candidate.exists() {
            return Ok(candidate);
        }
        match current.parent() {
            Some(p) => current = p,
            None => return Err("Cargo.toml not found".into()),
        }
    }
}

fn cargo_profile_dir(profile: &str) -> &str {
    if profile == "dev" { "debug" } else { profile }
}

fn copy_artifact_to_target(artifact: &Path, cwd: &Path, profile: &str) -> Result<(), String> {
    let target = cwd.join("target").join(cargo_profile_dir(profile));
    let dest = target.join(artifact.file_name().ok_or("artifact has no file name")?);
    std::fs::create_dir_all(&target).map_err(|e| format!("create target dir: {e}"))?;
    std::fs::copy(artifact, &dest).map_err(|e| format!("copy artifact: {e}"))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn usage_rejected_for_empty_args() {
        assert!(run(&[]).is_err());
    }

    #[test]
    fn help_is_recognized() {
        assert!(run(&["--help".into()]).is_ok());
    }
}
