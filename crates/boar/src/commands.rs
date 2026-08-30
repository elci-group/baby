use std::fs;
use std::io::Write;
use std::process::Command;

use form3::table::{Attribute, Cell, Table};

use crate::config::Settings;
use crate::filesystem::{directory_size, discover_project_root, guarded_remove, project_target};
use crate::memory::{human_bytes, human_kib, human_kib_aligned, read_meminfo};
use crate::pressure::{preflight_pressure_reason, read_kernel_pressure};
use crate::style;

type Result<T> = std::result::Result<T, String>;

fn boar_label(support: form3::term::ColorSupport) -> String {
    style::label("BOAR:", support)
}

pub fn print_status(settings: &Settings) -> Result<()> {
    let support = style::stdout_support();
    let memory = read_meminfo()?;
    let root_type = crate::filesystem::filesystem_type(&settings.ram_root)?;
    let free = crate::filesystem::filesystem_available_kib(&settings.ram_root)?;

    println!("{}", style::heading("Memory", support));
    let mut memory_table = Table::new();
    memory_table.set_header(vec![
        Cell::new("Metric").add_attribute(Attribute::Bold),
        Cell::new("Value").add_attribute(Attribute::Bold),
    ]);
    memory_table.add_row(vec![
        Cell::new("System total"),
        Cell::new(human_kib(memory.total_kib)),
    ]);
    memory_table.add_row(vec![
        Cell::new("System available"),
        Cell::new(
            human_kib_aligned(memory.available_kib)
                .trim_start()
                .to_owned(),
        ),
    ]);
    memory_table.add_row(vec![
        Cell::new("Swap used"),
        Cell::new(human_kib(
            memory.swap_total_kib.saturating_sub(memory.swap_free_kib),
        )),
    ]);
    if let Ok(pressure) = read_kernel_pressure() {
        memory_table.add_row(vec![
            Cell::new("Memory PSI (10s)"),
            Cell::new(format!(
                "some {:.1}%, full {:.1}%",
                pressure.memory_some_avg10, pressure.memory_full_avg10
            )),
        ]);
        memory_table.add_row(vec![
            Cell::new("I/O PSI (10s)"),
            Cell::new(format!("some {:.1}%", pressure.io_some_avg10)),
        ]);
    }
    memory_table.add_row(vec![
        Cell::new("RAM root"),
        Cell::new(format!("{} ({root_type})", settings.ram_root.display())),
    ]);
    memory_table.add_row(vec![Cell::new("Root free"), Cell::new(human_kib(free))]);
    print!("{memory_table}");

    println!();
    println!("{}", style::heading("Projects", support));

    if !settings.ram_root.exists() {
        println!("{}", style::dim("No RAM-backed targets.", support));
        return Ok(());
    }

    let mut projects = Vec::new();
    for entry in fs::read_dir(&settings.ram_root)
        .map_err(|error| format!("cannot list {}: {error}", settings.ram_root.display()))?
    {
        let path = entry.map_err(|error| error.to_string())?.path();
        let marker = path.join(".boar-owner");
        if !marker.is_file() {
            continue;
        }
        let owner = fs::read_to_string(marker)
            .unwrap_or_default()
            .lines()
            .next()
            .unwrap_or("unknown")
            .to_owned();
        let size = directory_size(&path).unwrap_or(0);
        projects.push((owner, size));
    }
    projects.sort_by_key(|project| std::cmp::Reverse(project.1));

    if projects.is_empty() {
        println!("{}", style::dim("No RAM-backed targets.", support));
    } else {
        let mut table = Table::new();
        table.set_header(vec![
            Cell::new("Size").add_attribute(Attribute::Bold),
            Cell::new("Project").add_attribute(Attribute::Bold),
        ]);
        for (owner, bytes) in &projects {
            table.add_row(vec![Cell::new(human_bytes(*bytes)), Cell::new(owner)]);
        }
        print!("{table}");
        let total: u64 = projects.iter().map(|(_, bytes)| bytes).sum();
        println!(
            "{} across {} project(s)",
            style::value(human_bytes(total), support),
            style::value(projects.len().to_string(), support)
        );
    }
    Ok(())
}

pub fn clean(settings: &Settings, all: bool) -> Result<()> {
    let support = style::stdout_support();
    if all {
        if !settings.ram_root.exists() {
            println!("{} nothing to clean", boar_label(support));
            return Ok(());
        }
        let mut removed = 0;
        for entry in fs::read_dir(&settings.ram_root)
            .map_err(|error| format!("cannot list {}: {error}", settings.ram_root.display()))?
        {
            let path = entry.map_err(|error| error.to_string())?.path();
            if path.join(".boar-owner").is_file() && guarded_remove(&settings.ram_root, &path)? {
                removed += 1;
            }
        }
        println!(
            "{} cleaned {} RAM target(s)",
            boar_label(support),
            style::value(removed.to_string(), support)
        );
    } else {
        let project_root = discover_project_root()?;
        let target = project_target(&settings.ram_root, &project_root);
        if guarded_remove(&settings.ram_root, &target)? {
            println!(
                "{} cleaned {}",
                boar_label(support),
                style::path(target.display().to_string(), support)
            );
        } else {
            println!(
                "{} no RAM target for {}",
                boar_label(support),
                style::path(project_root.display().to_string(), support)
            );
        }
    }
    Ok(())
}

pub fn doctor(settings: &Settings) -> Result<()> {
    let support = style::stdout_support();
    println!("{}", style::heading("BOAR Doctor", support));

    if cfg!(target_os = "linux") {
        println!("{}", style::ok("[ok] Linux", support));
    } else {
        println!(
            "{}",
            style::error("[fail] BOAR currently requires Linux", support)
        );
    }

    match Command::new("cargo").arg("--version").output() {
        Ok(output) if output.status.success() => {
            println!(
                "{} {}",
                style::ok("[ok]", support),
                String::from_utf8_lossy(&output.stdout).trim()
            )
        }
        _ => println!("{}", style::error("[fail] cargo is unavailable", support)),
    }

    match read_meminfo() {
        Ok(memory) => {
            println!(
                "{} memory: {} available of {}",
                style::ok("[ok]", support),
                style::value(human_kib(memory.available_kib), support),
                style::value(human_kib(memory.total_kib), support)
            );
            println!(
                "{} swap: {} used of {}",
                style::ok("[ok]", support),
                style::value(
                    human_kib(memory.swap_total_kib.saturating_sub(memory.swap_free_kib)),
                    support
                ),
                style::value(human_kib(memory.swap_total_kib), support)
            );
        }
        Err(error) => println!("{}", style::error(format!("[fail] {error}"), support)),
    }

    match read_kernel_pressure() {
        Ok(pressure) => {
            if let Some(reason) = preflight_pressure_reason(pressure) {
                println!(
                    "{} {reason}; automatic builds will use disk",
                    style::warn("[warn]", support)
                );
            } else {
                println!(
                    "{} pressure: memory some {:.1}%, full {:.1}%; I/O some {:.1}%",
                    style::ok("[ok]", support),
                    pressure.memory_some_avg10,
                    pressure.memory_full_avg10,
                    pressure.io_some_avg10
                );
            }
        }
        Err(error) => println!(
            "{} pressure metrics unavailable: {error}",
            style::warn("[warn]", support)
        ),
    }

    match crate::filesystem::filesystem_type(&settings.ram_root) {
        Ok(kind) if matches!(kind.as_str(), "tmpfs" | "ramfs") => {
            println!(
                "{} {} is backed by {kind}",
                style::ok("[ok]", support),
                style::path(settings.ram_root.display().to_string(), support)
            )
        }
        Ok(kind) => println!(
            "{} {} is backed by {kind}",
            style::warn("[warn]", support),
            style::path(settings.ram_root.display().to_string(), support)
        ),
        Err(error) => println!("{}", style::error(format!("[fail] {error}"), support)),
    }

    match crate::filesystem::filesystem_available_kib(&settings.ram_root) {
        Ok(free) => println!(
            "{} {} free in RAM root",
            style::ok("[ok]", support),
            style::value(human_kib(free), support)
        ),
        Err(error) => println!("{}", style::error(format!("[fail] {error}"), support)),
    }
    Ok(())
}

pub fn init() -> Result<()> {
    let support = style::stdout_support();
    let root = discover_project_root()?;
    let path = root.join(".boar.toml");
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
        .map_err(|error| {
            if error.kind() == std::io::ErrorKind::AlreadyExists {
                format!("{} already exists", path.display())
            } else {
                format!("cannot create {}: {error}", path.display())
            }
        })?;
    file.write_all(
        b"# BOAR project settings\nmode = \"auto\"\n# reserve_mib = 2048\n# max_ram_mib = 8192\nram_root = \"/dev/shm/boar\"\ndisk_target = \"target\"\nmonitor = true\n",
    )
    .map_err(|error| format!("cannot write {}: {error}", path.display()))?;
    println!(
        "{} created {}",
        boar_label(support),
        style::path(path.display().to_string(), support)
    );
    Ok(())
}

pub fn help() {
    let support = style::stdout_support();
    let c = |text: &str| style::command(text, support);
    let o = |text: &str| style::option(text, support);
    let v = |text: &str| style::value(text, support);
    let p = |text: &str| style::path(text, support);
    let d = |text: &str| style::dim(text, support);
    let h = |text: &str| style::heading(text, support);

    const CMD_WIDTH: usize = 14;

    let command = |name: &str, desc: &str, example: &str| {
        let padding = " ".repeat(CMD_WIDTH.saturating_sub(name.len()));
        println!("    {}{} {}", c(name), padding, d(desc));
        println!("{}{}", " ".repeat(4 + CMD_WIDTH + 1), d(example));
    };

    println!("{}", style::title("BOAR — Build On Available RAM", support));
    println!();
    println!("{}", h("Description"));
    println!(
        "    BOAR is a RAM-aware Cargo wrapper for Linux. It places a project's\n    \
         Cargo target directory on tmpfs when the machine has enough safe headroom,\n    \
         and falls back to disk when it does not."
    );
    println!();
    println!("{}", h("Usage"));
    println!(
        "    {} [{}] <{}> [{}]",
        c("boar"),
        o("OPTIONS"),
        c("COMMAND"),
        d("CARGO ARGS...")
    );
    println!(
        "    {} [{}] {} <{}> [{}]",
        c("boar"),
        o("OPTIONS"),
        c("cargo"),
        c("CARGO-COMMAND"),
        d("CARGO ARGS...")
    );
    println!();
    println!("{}", h("Commands"));
    command(
        "build",
        "Run `cargo build` through BOAR",
        "Example: boar build --release",
    );
    command(
        "check",
        "Run `cargo check` through BOAR",
        "Example: boar check",
    );
    command(
        "test",
        "Run `cargo test` through BOAR",
        "Example: boar test --workspace",
    );
    command(
        "run",
        "Run `cargo run` through BOAR",
        "Example: boar run --release",
    );
    command(
        "bench",
        "Run `cargo bench` through BOAR",
        "Example: boar bench",
    );
    command(
        "clippy",
        "Run `cargo clippy` through BOAR",
        "Example: boar clippy -- -D warnings",
    );
    command(
        "doc",
        "Run `cargo doc` through BOAR",
        "Example: boar doc --open",
    );
    command(
        "cargo <CMD>",
        "Run any Cargo subcommand through BOAR",
        "Example: boar cargo metadata --no-deps",
    );
    command(
        "status",
        "Show memory capacity and active RAM targets",
        "Example: boar status",
    );
    command(
        "clean [--all]",
        "Remove this project's RAM target, or all BOAR-owned targets",
        "Example: boar clean --all",
    );
    command(
        "doctor",
        "Check Linux, Cargo, memory, pressure, and tmpfs readiness",
        "Example: boar doctor",
    );
    command(
        "init",
        "Create a documented .boar.toml in the current workspace",
        "Example: boar init",
    );
    println!();
    println!("{}", h("Options"));
    println!(
        "    {} <auto|ram|disk>\n        Placement mode. Default: {}. Env: {}\n",
        o("--mode"),
        v("auto"),
        v("BOAR_MODE")
    );
    println!(
        "    {} <MIB>\n        Minimum RAM to leave for the OS. Default: {}. Env: {}\n",
        o("--reserve-mib"),
        v("adaptive (25% of RAM, 2 GiB floor)"),
        v("BOAR_RESERVE_MIB")
    );
    println!(
        "    {} <MIB>\n        Cap RAM BOAR may use for the target. Default: {}. Env: {}\n",
        o("--max-ram-mib"),
        v("unlimited"),
        v("BOAR_MAX_RAM_MIB")
    );
    println!(
        "    {} <PATH>\n        Directory for RAM-backed targets. Default: {}. Env: {}\n",
        o("--ram-root"),
        p("/dev/shm/boar"),
        v("BOAR_RAM_ROOT")
    );
    println!(
        "    {} <PATH>\n        Disk fallback target directory. Default: {}. Env: {}\n",
        o("--disk-target"),
        p("workspace target/"),
        v("BOAR_DISK_TARGET")
    );
    println!(
        "    {}\n        Disable the live pressure monitor. Default: {}. Env: {}\n",
        o("--no-monitor"),
        v("monitor on"),
        v("BOAR_MONITOR")
    );
    println!(
        "    {}\n        Enable the live pressure monitor (default).\n",
        o("--monitor")
    );
    println!(
        "    {}, {}\n        Show this help message and exit.\n",
        o("-h"),
        o("--help")
    );
    println!(
        "    {}, {}\n        Show version information and exit.\n",
        o("-V"),
        o("--version")
    );
    println!("{}", h("Configuration"));
    println!(
        "    Precedence is {} options, then {}, then {}, then built-in {}.\n    Run {} to create a documented configuration file.",
        h("CLI"),
        h("environment variables"),
        p(".boar.toml"),
        h("defaults"),
        c("boar init")
    );
}
