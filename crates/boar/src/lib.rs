// BOAR - Build On Available RAM
// Modular architecture for better maintainability

mod cargo;
mod commands;
mod config;
mod estimate;
mod filesystem;
mod memory;
mod pressure;
pub mod style;

pub use cargo::{CargoResult, run_cargo};
pub use commands::{clean, doctor, help, init, print_status};
pub use config::{Action, Cli, Mode, Overrides, Settings, parse_cli};
pub use estimate::{load_estimate, load_or_seed_estimate, save_estimate};
pub use filesystem::{
    discover_project_root, disk_target, guarded_remove, project_target, write_marker,
};
pub use memory::{Allocation, MemInfo, calculate_allocation, human_bytes, human_kib, read_meminfo};
pub use pressure::{KernelPressure, PressureMonitor, monitor_pressure, read_kernel_pressure};

const VERSION: &str = env!("CARGO_PKG_VERSION");

type Result<T> = std::result::Result<T, String>;

pub fn run(args: Vec<String>) -> Result<i32> {
    if !cfg!(target_os = "linux") {
        return Err("BOAR currently supports Linux only".into());
    }
    let cli = parse_cli(args)?;
    if cli.action == Action::Help {
        help();
        return Ok(0);
    }
    if cli.action == Action::Version {
        let support = crate::style::stdout_support();
        println!(
            "{} {}",
            crate::style::title("boar", support),
            crate::style::value(VERSION, support)
        );
        return Ok(0);
    }

    let project_root = discover_project_root()?;
    let mut settings = Settings::load(&project_root.join(".boar.toml"))?;
    cli.overrides.apply(&mut settings);
    match cli.action {
        Action::Cargo { command, args } => run_cargo(&settings, &project_root, &command, &args),
        Action::Status => print_status(&settings).map(|()| 0),
        Action::Clean { all } => clean(&settings, all).map(|()| 0),
        Action::Doctor => doctor(&settings).map(|()| 0),
        Action::Init => init().map(|()| 0),
        Action::Help | Action::Version => unreachable!(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_linux_memory_information() {
        let parsed =
            crate::memory::parse_meminfo(
                "MemTotal: 16384000 kB\nMemFree: 1 kB\nMemAvailable: 8192000 kB\nSwapTotal: 4096000 kB\nSwapFree: 1024000 kB\n",
            )
                .expect("valid meminfo");
        assert_eq!(
            parsed,
            MemInfo {
                total_kib: 16_384_000,
                available_kib: 8_192_000,
                swap_total_kib: 4_096_000,
                swap_free_kib: 1_024_000,
            }
        );
    }

    #[test]
    fn allocation_reserves_memory_and_filesystem_headroom() {
        let allocation = calculate_allocation(
            MemInfo {
                total_kib: 16 * 1024 * 1024,
                available_kib: 12 * 1024 * 1024,
                swap_total_kib: 0,
                swap_free_kib: 0,
            },
            10 * 1024 * 1024,
            None,
            Some(6 * 1024),
            1024 * 1024,
        );
        assert_eq!(allocation.reserve_kib, 4 * 1024 * 1024);
        assert_eq!(allocation.budget_kib, 6 * 1024 * 1024);
        assert!(allocation.enough());
    }

    #[test]
    fn allocation_falls_short_when_ram_is_tight() {
        let allocation = calculate_allocation(
            MemInfo {
                total_kib: 4 * 1024 * 1024,
                available_kib: 2 * 1024 * 1024,
                swap_total_kib: 0,
                swap_free_kib: 0,
            },
            4 * 1024 * 1024,
            None,
            None,
            1024 * 1024,
        );
        assert_eq!(allocation.budget_kib, 0);
        assert!(!allocation.enough());
    }

    #[test]
    fn config_is_strict_and_applies_values() {
        let mut settings = Settings::default();
        settings
            .apply_config("mode = \"disk\"\nreserve_mib = 42\nmax_ram_mib = 99\nmonitor = false\n")
            .expect("valid config");
        assert_eq!(settings.mode, Mode::Disk);
        assert_eq!(settings.reserve_mib, Some(42));
        assert_eq!(settings.max_ram_mib, Some(99));
        assert!(!settings.monitor);
        assert!(settings.apply_config("mystery = true").is_err());
    }

    #[test]
    fn parses_direct_and_generic_cargo_commands() {
        let direct = parse_cli(vec![
            "--mode=ram".into(),
            "build".into(),
            "--release".into(),
        ])
        .expect("direct command");
        assert_eq!(direct.overrides.mode, Some(Mode::Ram));
        assert_eq!(
            direct.action,
            Action::Cargo {
                command: "build".into(),
                args: vec!["--release".into()]
            }
        );
        let generic = parse_cli(vec!["cargo".into(), "metadata".into(), "--no-deps".into()])
            .expect("generic command");
        assert_eq!(
            generic.action,
            Action::Cargo {
                command: "metadata".into(),
                args: vec!["--no-deps".into()]
            }
        );
    }

    #[test]
    fn rejects_confusing_clean_arguments() {
        assert!(parse_cli(vec!["clean".into(), "--force".into()]).is_err());
    }

    #[test]
    fn project_keys_are_safe_and_stable() {
        assert_eq!(
            crate::filesystem::sanitize("hello world/thing"),
            "hello-world-thing"
        );
        assert_eq!(
            crate::filesystem::stable_hash("/work/boar"),
            crate::filesystem::stable_hash("/work/boar")
        );
        assert_ne!(
            crate::filesystem::stable_hash("/work/boar"),
            crate::filesystem::stable_hash("/work/other")
        );
    }

    #[test]
    fn directory_size_does_not_follow_symlinks() {
        let root = std::env::temp_dir().join(format!("boar-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("temp dir");
        std::fs::write(root.join("data"), b"12345").expect("test data");
        std::os::unix::fs::symlink(root.join("data"), root.join("link")).expect("test link");
        assert_eq!(
            crate::filesystem::directory_size(&root).expect("directory size"),
            5
        );
        std::fs::remove_dir_all(root).expect("clean temp dir");
    }

    #[test]
    fn mount_escaping_is_decoded() {
        assert_eq!(
            crate::filesystem::unescape_mount("/media/a\\040b"),
            "/media/a b"
        );
    }

    #[test]
    fn parses_pressure_stall_information() {
        let parsed = crate::pressure::parse_psi(
            "some avg10=12.50 avg60=4.00 avg300=1.00 total=42\nfull avg10=2.25 avg60=1.00 avg300=0.50 total=7\n",
        )
        .expect("valid PSI");
        assert_eq!(parsed, (12.5, 2.25));
    }

    #[test]
    fn parses_swap_counters() {
        assert_eq!(
            crate::pressure::parse_vmstat("pgfault 1\npswpin 123\npswpout 456\n")
                .expect("valid vmstat"),
            (123, 456)
        );
    }

    #[test]
    fn elevated_pressure_blocks_automatic_ram_placement() {
        let pressure = KernelPressure {
            memory_some_avg10: 10.0,
            memory_full_avg10: 0.0,
            io_some_avg10: 80.0,
            swap_in_pages: 0,
            swap_out_pages: 0,
        };
        assert_eq!(
            pressure::preflight_pressure_reason(pressure).as_deref(),
            Some("memory some PSI is 10.0%")
        );
    }

    #[test]
    fn live_swap_activity_triggers_pressure() {
        let previous = KernelPressure {
            memory_some_avg10: 0.0,
            memory_full_avg10: 0.0,
            io_some_avg10: 0.0,
            swap_in_pages: 100,
            swap_out_pages: 100,
        };
        let current = KernelPressure {
            swap_out_pages: 100
                + crate::pressure::SWAP_ACTIVITY_LIMIT_KIB / crate::memory::FALLBACK_PAGE_KIB,
            ..previous
        };
        assert!(
            crate::pressure::live_pressure_reason(
                current,
                Some(previous),
                crate::memory::FALLBACK_PAGE_KIB
            )
            .expect("swap pressure")
            .contains("swap activity")
        );
    }

    #[test]
    fn parses_persisted_estimates() {
        assert_eq!(
            crate::estimate::parse_estimate("size_kib=654321\nupdated_unix=1\n"),
            Some(654_321)
        );
        assert_eq!(crate::estimate::parse_estimate("size_kib=0\n"), None);
        assert_eq!(crate::estimate::parse_estimate("broken\n"), None);
    }
}
