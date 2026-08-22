// Copyright (c) 2026 sal
// SPDX-License-Identifier: MIT

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
}

fn main() {
    setup_logging();
    if let Err(e) = run() {
        log::error!("{e}");
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    let args = Args::parse();

    if let Some(path) = args.generate_man {
        let cmd = <Args as clap::CommandFactory>::command();
        baby::generate_man(&cmd, &path)?;
        log::info!("man page written to {}", path.display());
        return Ok(());
    }

    if let Some(cmd) = args.command {
        return handle_command(cmd);
    }

    let config = InstallConfig {
        strip: args.strip,
        backup: args.backup,
        service: args.service,
        sudo: args.sudo,
        user: args.user,
        dry_run: args.dry_run,
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

fn handle_command(cmd: Command) -> Result<()> {
    match cmd {
        Command::Update {
            nightly,
            bleeding,
            stable,
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
    }
}
