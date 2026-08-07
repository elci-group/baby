use baby::error::Result;
use baby::{InstallConfig, build_and_install, run_binary, setup_logging};
use clap::Parser;
use std::path::PathBuf;

const LONG_ABOUT: &str = r#"
BABY — Build And Bin Yield

Build a Rust project in release mode and install the resulting binary.

By default the binary is installed to /usr/local/bin. Use --user to install
to ~/.local/bin instead, or --install-dir for a custom location.

Examples:
  baby                      # build + install to /usr/local/bin
  baby --user               # build + install to ~/.local/bin
  baby --run                # build + install + execute
  baby --run -- --help      # pass --help to the installed binary
  baby --dry-run            # preview what would be done
"#;

/// BABY — Build And Bin Yield
#[derive(Parser, Debug)]
#[command(name = "baby")]
#[command(version)]
#[command(about = "Build a Rust project and install the release binary")]
#[command(long_about = LONG_ABOUT)]
#[command(styles = baby::styles::cli())]
struct Args {
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

    /// Pass additional arguments when running with --run
    #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
    run_args: Vec<String>,

    /// Generate a man page and exit
    #[arg(long, hide = true)]
    generate_man: Option<PathBuf>,
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

    let config = InstallConfig {
        strip: args.strip,
        backup: args.backup,
        service: args.service,
        sudo: args.sudo,
        user: args.user,
        dry_run: args.dry_run,
        target_dir: args.target_dir,
        install_dir: args.install_dir,
    };

    build_and_install(&config)?;

    if args.run {
        let project = baby::infer_project_name()?;
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
