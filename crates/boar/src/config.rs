use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use crate::memory::parse_u64;
use crate::pressure::parse_bool;

type Result<T> = std::result::Result<T, String>;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Mode {
    Auto,
    Ram,
    Disk,
}

impl Mode {
    pub fn parse(value: &str) -> Result<Self> {
        match value {
            "auto" => Ok(Self::Auto),
            "ram" => Ok(Self::Ram),
            "disk" => Ok(Self::Disk),
            _ => Err(format!(
                "invalid mode '{value}'; expected auto, ram, or disk"
            )),
        }
    }
}

#[derive(Clone, Debug)]
pub struct Settings {
    pub mode: Mode,
    pub reserve_mib: Option<u64>,
    pub max_ram_mib: Option<u64>,
    pub ram_root: PathBuf,
    pub disk_target: Option<PathBuf>,
    pub monitor: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            mode: Mode::Auto,
            reserve_mib: None,
            max_ram_mib: None,
            ram_root: PathBuf::from("/dev/shm/boar"),
            disk_target: None,
            monitor: true,
        }
    }
}

impl Settings {
    pub fn load(path: &Path) -> Result<Self> {
        let mut settings = Self::default();
        if path.exists() {
            let text = fs::read_to_string(path)
                .map_err(|error| format!("cannot read {}: {error}", path.display()))?;
            settings.apply_config(&text)?;
        }
        settings.apply_env()?;
        Ok(settings)
    }

    pub(crate) fn apply_config(&mut self, text: &str) -> Result<()> {
        for (index, raw_line) in text.lines().enumerate() {
            let line = raw_line.split('#').next().unwrap_or_default().trim();
            if line.is_empty() {
                continue;
            }
            let (key, raw_value) = line
                .split_once('=')
                .ok_or_else(|| format!("invalid config on line {}", index + 1))?;
            let key = key.trim();
            let value = raw_value.trim().trim_matches('"');
            match key {
                "mode" => self.mode = Mode::parse(value)?,
                "reserve_mib" => self.reserve_mib = Some(parse_u64(key, value)?),
                "max_ram_mib" => self.max_ram_mib = Some(parse_u64(key, value)?),
                "ram_root" => self.ram_root = PathBuf::from(value),
                "disk_target" => self.disk_target = Some(PathBuf::from(value)),
                "monitor" => self.monitor = parse_bool(key, value)?,
                _ => return Err(format!("unknown config key '{key}' on line {}", index + 1)),
            }
        }
        Ok(())
    }

    fn apply_env(&mut self) -> Result<()> {
        if let Ok(value) = env::var("BOAR_MODE") {
            self.mode = Mode::parse(&value)?;
        }
        if let Ok(value) = env::var("BOAR_RESERVE_MIB") {
            self.reserve_mib = Some(parse_u64("BOAR_RESERVE_MIB", &value)?);
        }
        if let Ok(value) = env::var("BOAR_MAX_RAM_MIB") {
            self.max_ram_mib = Some(parse_u64("BOAR_MAX_RAM_MIB", &value)?);
        }
        if let Ok(value) = env::var("BOAR_RAM_ROOT") {
            self.ram_root = PathBuf::from(value);
        }
        if let Ok(value) = env::var("BOAR_DISK_TARGET") {
            self.disk_target = Some(PathBuf::from(value));
        }
        if let Ok(value) = env::var("BOAR_MONITOR") {
            self.monitor = parse_bool("BOAR_MONITOR", &value)?;
        }
        Ok(())
    }
}

#[derive(Debug, Default)]
pub struct Overrides {
    pub mode: Option<Mode>,
    pub reserve_mib: Option<u64>,
    pub max_ram_mib: Option<u64>,
    pub ram_root: Option<PathBuf>,
    pub disk_target: Option<PathBuf>,
    pub monitor: Option<bool>,
}

impl Overrides {
    pub fn apply(self, settings: &mut Settings) {
        if let Some(value) = self.mode {
            settings.mode = value;
        }
        if let Some(value) = self.reserve_mib {
            settings.reserve_mib = Some(value);
        }
        if let Some(value) = self.max_ram_mib {
            settings.max_ram_mib = Some(value);
        }
        if let Some(value) = self.ram_root {
            settings.ram_root = value;
        }
        if let Some(value) = self.disk_target {
            settings.disk_target = Some(value);
        }
        if let Some(value) = self.monitor {
            settings.monitor = value;
        }
    }
}

#[derive(Debug, Eq, PartialEq)]
pub enum Action {
    Cargo { command: String, args: Vec<String> },
    Status,
    Clean { all: bool },
    Doctor,
    Init,
    Help,
    Version,
}

#[derive(Debug)]
pub struct Cli {
    pub action: Action,
    pub overrides: Overrides,
}

pub fn parse_cli(args: Vec<String>) -> Result<Cli> {
    let mut overrides = Overrides::default();
    let mut index = 0;

    while index < args.len() {
        let arg = &args[index];
        let value = |name: &str, index: &mut usize| -> Result<String> {
            *index += 1;
            args.get(*index)
                .cloned()
                .ok_or_else(|| format!("{name} requires a value"))
        };

        match arg.as_str() {
            "-h" | "--help" => {
                return Ok(Cli {
                    action: Action::Help,
                    overrides,
                });
            }
            "-V" | "--version" => {
                return Ok(Cli {
                    action: Action::Version,
                    overrides,
                });
            }
            "--mode" => overrides.mode = Some(Mode::parse(&value("--mode", &mut index)?)?),
            "--reserve-mib" => {
                overrides.reserve_mib = Some(parse_u64(
                    "--reserve-mib",
                    &value("--reserve-mib", &mut index)?,
                )?)
            }
            "--max-ram-mib" => {
                overrides.max_ram_mib = Some(parse_u64(
                    "--max-ram-mib",
                    &value("--max-ram-mib", &mut index)?,
                )?)
            }
            "--ram-root" => {
                overrides.ram_root = Some(PathBuf::from(value("--ram-root", &mut index)?))
            }
            "--disk-target" => {
                overrides.disk_target = Some(PathBuf::from(value("--disk-target", &mut index)?))
            }
            "--no-monitor" => overrides.monitor = Some(false),
            "--monitor" => overrides.monitor = Some(true),
            _ if arg.starts_with("--mode=") => overrides.mode = Some(Mode::parse(&arg[7..])?),
            _ if arg.starts_with("--reserve-mib=") => {
                overrides.reserve_mib = Some(parse_u64("--reserve-mib", &arg[14..])?)
            }
            _ if arg.starts_with("--max-ram-mib=") => {
                overrides.max_ram_mib = Some(parse_u64("--max-ram-mib", &arg[14..])?)
            }
            _ if arg.starts_with("--ram-root=") => {
                overrides.ram_root = Some(PathBuf::from(&arg[11..]))
            }
            _ if arg.starts_with("--disk-target=") => {
                overrides.disk_target = Some(PathBuf::from(&arg[14..]))
            }
            _ if arg.starts_with('-') => return Err(format!("unknown BOAR option '{arg}'")),
            _ => break,
        }
        index += 1;
    }

    let Some(command) = args.get(index).map(String::as_str) else {
        return Ok(Cli {
            action: Action::Help,
            overrides,
        });
    };
    let tail = args[index + 1..].to_vec();
    let action = match command {
        "status" => no_args(Action::Status, command, &tail)?,
        "doctor" => no_args(Action::Doctor, command, &tail)?,
        "init" => no_args(Action::Init, command, &tail)?,
        "clean" => match tail.as_slice() {
            [] => Action::Clean { all: false },
            [flag] if flag == "--all" => Action::Clean { all: true },
            _ => return Err("usage: boar clean [--all]".into()),
        },
        "cargo" => {
            let Some((cargo_command, cargo_args)) = tail.split_first() else {
                return Err("boar cargo requires a Cargo subcommand".into());
            };
            Action::Cargo {
                command: cargo_command.clone(),
                args: cargo_args.to_vec(),
            }
        }
        "build" | "check" | "test" | "run" | "bench" | "clippy" | "doc" => Action::Cargo {
            command: command.to_owned(),
            args: tail,
        },
        _ => return Err(format!("unknown command '{command}'; run 'boar --help'")),
    };

    Ok(Cli { action, overrides })
}

fn no_args(action: Action, command: &str, args: &[String]) -> Result<Action> {
    if args.is_empty() {
        Ok(action)
    } else {
        Err(format!("boar {command} does not accept arguments"))
    }
}
