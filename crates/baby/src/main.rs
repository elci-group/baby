// Copyright (c) 2026 sal
// SPDX-License-Identifier: MIT

mod curly_expand;

use baby::error::Result;
use baby::{InstallConfig, build_and_install, run_binary, setup_logging};
use clap::Parser;
use std::path::PathBuf;

const LONG_ABOUT: &str = r#"
BABY — Build And Bin Yield

Build a project from a validated installation recipe and install its binary.

By default the binary is installed to /usr/local/bin. Use --user to install
to ~/.local/bin instead, or --install-dir for a custom location.

Examples:
  baby                      # build + install to /usr/local/bin
  baby --user               # build + install to ~/.local/bin
  baby --run                # build + install + execute
  baby --run -- --help      # pass --help to the installed binary
  baby --dry-run            # preview what would be done
  baby --recipe .baby.toml  # use an explicit versioned installation recipe
  baby update               # check for updates (--stable by default)
  baby update --nightly     # check for nightly builds
  baby update --bleeding    # check for bleeding edge builds
  baby boom                 # discover and update all managed tools
  baby boom --interactive   # select which tools to update interactively
"#;

/// BABY — Build And Bin Yield
#[derive(Parser, Debug)]
#[command(name = "baby")]
#[command(version)]
#[command(about = "Build a project from a recipe and install its binary")]
#[command(long_about = LONG_ABOUT)]
#[command(styles = baby::styles::cli())]
struct Args {
    #[command(subcommand)]
    command: Option<Command>,

    /// Build, install, then execute the binary
    #[arg(long)]
    run: bool,

    /// Strip symbols before installing
    #[arg(long)]
    strip: bool,

    /// Backup existing binary before overwriting
    #[arg(long)]
    backup: bool,

    /// Restart matching systemd service after install
    #[arg(long)]
    service: bool,

    /// Force privileged (sudo) install
    #[arg(long)]
    sudo: bool,

    /// Install to ~/.local/bin instead of /usr/local/bin
    #[arg(long)]
    user: bool,

    /// Show what would happen without executing
    #[arg(long)]
    dry_run: bool,

    /// Keep build artefacts after installation
    #[arg(long)]
    no_clean: bool,

    /// Never delegate a failed build to `boar` for RAM/disk recovery
    #[arg(long)]
    no_boar: bool,

    /// Skip all locksmithd coordination (repo-wide wait and project lease)
    #[arg(long)]
    no_lock: bool,

    /// Seconds to wait for a contended locksmith lease (default: 300)
    #[arg(long, value_name = "SECS")]
    lock_timeout: Option<u64>,

    /// Seconds to request for the locksmith lease duration (default: 1200)
    #[arg(long, value_name = "SECS")]
    lock_lease: Option<u64>,

    /// Seconds to wait for all repo-wide locksmith leases to clear (default: 300)
    #[arg(long, value_name = "SECS")]
    repo_lock_timeout: Option<u64>,

    /// Custom target directory (default: target/release)
    #[arg(long)]
    target_dir: Option<PathBuf>,

    /// Custom install directory (overrides --user)
    #[arg(long)]
    install_dir: Option<PathBuf>,

    /// Versioned installation recipe (default: .baby.toml, then Cargo.toml)
    #[arg(long)]
    recipe: Option<PathBuf>,

    /// Validate and print the resolved recipe without executing it
    #[arg(long)]
    check_recipe: bool,

    /// Pass additional arguments when running with --run
    #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
    run_args: Vec<String>,

    /// Generate a man page and exit
    #[arg(long, hide = true)]
    generate_man: Option<PathBuf>,
}

#[derive(Parser, Debug)]
enum Command {
    /// Check for available updates
    Update {
        /// Check against nightly builds
        #[arg(long, group = "channel")]
        nightly: bool,

        /// Check against bleeding edge builds
        #[arg(long, group = "channel")]
        bleeding: bool,

        /// Check against stable releases (default)
        #[arg(long, group = "channel")]
        stable: bool,
    },
    /// Discover and update all managed tools in parallel
    Boom {
        /// Initialize a new .boom.toml file
        #[arg(long)]
        init: bool,

        /// Show what would be updated without executing
        #[arg(long)]
        dry_run: bool,

        /// Automatically confirm all updates
        #[arg(long)]
        yes: bool,

        /// Interactively select which tools to update
        #[arg(long)]
        interactive: bool,

        /// Number of parallel workers (default: 4)
        #[arg(long)]
        parallelism: Option<usize>,

        /// Only update specific tools (comma-separated)
        #[arg(long)]
        filter: Option<String>,
    },
}

#[tokio::main]
async fn __curly_original_main() {
    setup_logging();
    if let Err(e) = run().await {
        log::error!("{e}");
        std::process::exit(1);
    }
}

async fn run() -> Result<()> {
    let args = Args::parse();

    if let Some(path) = args.generate_man {
        let cmd = <Args as clap::CommandFactory>::command();
        baby::generate_man(&cmd, &path)?;
        log::info!("man page written to {}", path.display());
        return Ok(());
    }

    if let Some(cmd) = args.command {
        return handle_command(cmd).await;
    }

    let config = InstallConfig {
        strip: args.strip,
        backup: args.backup,
        service: args.service,
        sudo: args.sudo,
        user: args.user,
        dry_run: args.dry_run,
        no_clean: args.no_clean,
        no_boar: args.no_boar,
        no_lock: args.no_lock,
        lock_timeout_secs: args
            .lock_timeout
            .unwrap_or(baby::lock::DEFAULT_TIMEOUT_SECS),
        lock_lease_secs: args.lock_lease.unwrap_or(baby::lock::DEFAULT_LEASE_SECS),
        repo_lock_timeout_secs: args
            .repo_lock_timeout
            .unwrap_or(baby::lock::DEFAULT_TIMEOUT_SECS),
        target_dir: args.target_dir,
        install_dir: args.install_dir,
        recipe: args.recipe,
    };

    if args.check_recipe {
        let (recipe, root) = baby::resolve_install_recipe(&config)?;
        println!(
            "{} {:?} {} {}",
            recipe.schema,
            recipe.build_system,
            recipe.binary,
            root.join(recipe.artifact).display()
        );
        return Ok(());
    }

    build_and_install(&config)?;

    if args.run {
        let (recipe, _) = baby::resolve_install_recipe(&config)?;
        let project = recipe.binary;
        let install_dir = if let Some(ref dir) = config.install_dir {
            dir.clone()
        } else if config.user {
            baby::home_local_bin()?
        } else {
            PathBuf::from("/usr/local/bin")
        };
        let install_path = install_dir.join(&project);
        run_binary(&install_path, &args.run_args)?;
    }

    Ok(())
}

async fn handle_command(cmd: Command) -> Result<()> {
    match cmd {
        Command::Update {
            nightly,
            bleeding,
            stable: _,
        } => {
            let channel = if bleeding {
                baby::versioning::Channel::Bleeding
            } else if nightly {
                baby::versioning::Channel::Nightly
            } else {
                baby::versioning::Channel::Stable
            };
            baby::check_for_updates(channel)?;
            Ok(())
        }
        Command::Boom {
            init,
            dry_run,
            yes,
            interactive,
            parallelism,
            filter,
        } => {
            if init {
                return baby::boom::init_boom_config().await;
            }
            let filter_vec = filter.map(|f| f.split(',').map(|s| s.trim().to_string()).collect());
            baby::boom::run_boom(dry_run, yes, interactive, parallelism, filter_vec).await
        }
    }
}

fn main() {
    let raw_args: Vec<String> = std::env::args().collect();
    let mut positions: Vec<usize> = Vec::new();
    let mut fields: Vec<Vec<String>> = Vec::new();
    for (__i, __a) in raw_args.iter().enumerate() {
        if __a == "--target-dir" {
            if let Some(__v) = raw_args.get(__i + 1) {
                positions.push(__i + 1);
                fields.push(curly_expand::expand_or_literal(__v));
            }
            break;
        } else if let Some(__v) = __a.strip_prefix("--target-dir=") {
            positions.push(__i);
            fields.push(
                curly_expand::expand_or_literal(__v)
                    .into_iter()
                    .map(|v| format!("--target-dir={}", v))
                    .collect(),
            );
            break;
        }
    }
    for (__i, __a) in raw_args.iter().enumerate() {
        if __a == "--install-dir" {
            if let Some(__v) = raw_args.get(__i + 1) {
                positions.push(__i + 1);
                fields.push(curly_expand::expand_or_literal(__v));
            }
            break;
        } else if let Some(__v) = __a.strip_prefix("--install-dir=") {
            positions.push(__i);
            fields.push(
                curly_expand::expand_or_literal(__v)
                    .into_iter()
                    .map(|v| format!("--install-dir={}", v))
                    .collect(),
            );
            break;
        }
    }
    for (__i, __a) in raw_args.iter().enumerate() {
        if __a == "--recipe" {
            if let Some(__v) = raw_args.get(__i + 1) {
                positions.push(__i + 1);
                fields.push(curly_expand::expand_or_literal(__v));
            }
            break;
        } else if let Some(__v) = __a.strip_prefix("--recipe=") {
            positions.push(__i);
            fields.push(
                curly_expand::expand_or_literal(__v)
                    .into_iter()
                    .map(|v| format!("--recipe={}", v))
                    .collect(),
            );
            break;
        }
    }
    for (__i, __a) in raw_args.iter().enumerate() {
        if __a == "--generate-man" {
            if let Some(__v) = raw_args.get(__i + 1) {
                positions.push(__i + 1);
                fields.push(curly_expand::expand_or_literal(__v));
            }
            break;
        } else if let Some(__v) = __a.strip_prefix("--generate-man=") {
            positions.push(__i);
            fields.push(
                curly_expand::expand_or_literal(__v)
                    .into_iter()
                    .map(|v| format!("--generate-man={}", v))
                    .collect(),
            );
            break;
        }
    }
    if let Some(__v) = raw_args.get(1) {
        if !__v.starts_with('-') {
            positions.push(1);
            fields.push(curly_expand::expand_or_literal(__v));
        }
    }

    if fields.is_empty() || fields.iter().all(|f| f.len() <= 1) {
        __curly_original_main();
        return;
    }

    let combos = curly_expand::cartesian(&fields);
    let exe = std::env::current_exe().expect("resolve current exe");
    let mut had_failure = false;
    for combo in &combos {
        let mut new_args = raw_args.clone();
        for (slot, value) in positions.iter().zip(combo.iter()) {
            new_args[*slot] = value.clone();
        }
        let status = std::process::Command::new(&exe)
            .args(&new_args[1..])
            .status()
            .expect("failed to re-exec self");
        if !status.success() {
            had_failure = true;
        }
    }
    if had_failure {
        std::process::exit(1);
    }
}
