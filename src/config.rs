use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use crate::error::{BabyError, Result};

/// Per-project configuration used by `birthd`.
#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct ProjectConfig {
    /// Human-readable project name; must match the binary name.
    pub project: String,
    /// Paths to watch for changes.
    #[serde(default)]
    pub watch: Vec<String>,
    /// Shell command used to build the project.
    #[serde(default = "default_build")]
    pub build: String,
    /// Directory where the built binary is installed.
    #[serde(default = "default_install")]
    pub install: PathBuf,
    /// Optional systemd service to restart after a successful build.
    #[serde(default)]
    pub restart: Option<String>,
    /// Debounce window in milliseconds before triggering a rebuild.
    #[serde(default = "default_debounce")]
    pub debounce_ms: u64,
    /// Strip debug symbols before installing.
    #[serde(default)]
    pub strip: bool,
    /// Backup the existing binary before overwriting.
    #[serde(default)]
    pub backup: bool,
    /// Use `sudo` for privileged install operations.
    #[serde(default)]
    pub sudo: bool,
    /// Install into the user directory instead of the system directory.
    #[serde(default)]
    pub user: bool,
}

fn default_build() -> String {
    "cargo build --release".to_string()
}

fn default_install() -> PathBuf {
    PathBuf::from("/usr/local/bin")
}

fn default_debounce() -> u64 {
    500
}

/// Load all birthd configs from the standard directories and the current directory.
pub fn load_all_configs() -> Vec<(PathBuf, ProjectConfig)> {
    let mut configs = vec![];

    // Current directory .birth.toml
    let local = PathBuf::from(".birth.toml");
    if local.exists()
        && let Ok(cfg) = load_config_file(&local)
    {
        configs.push((local, cfg));
    }

    // XDG and system directories
    let dirs = crate::birthd_config_dirs();
    for dir in dirs {
        if let Ok(entries) = fs::read_dir(&dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().and_then(|s| s.to_str()) == Some("toml")
                    && let Ok(cfg) = load_config_file(&path)
                {
                    configs.push((path, cfg));
                }
            }
        }
    }

    configs
}

/// Parse a single `.birth.toml` file and resolve relative watch paths.
pub fn load_config_file(path: &Path) -> Result<ProjectConfig> {
    let content = fs::read_to_string(path)
        .map_err(|e| BabyError::io(format!("read {}", path.display()), e))?;
    let mut cfg: ProjectConfig = toml::from_str(&content)
        .map_err(|e| BabyError::config_parse(path.display().to_string(), e))?;

    // Resolve watch paths relative to the config file's directory
    let base = path.parent().unwrap_or(Path::new("."));
    let mut resolved = vec![];
    for w in &cfg.watch {
        let p = base.join(w);
        resolved.push(p.to_string_lossy().to_string());
    }
    cfg.watch = resolved;

    Ok(cfg)
}

/// Build a map from watched path to the project config that owns it.
pub fn path_to_project_map(configs: &[(PathBuf, ProjectConfig)]) -> HashMap<PathBuf, Vec<usize>> {
    let mut map: HashMap<PathBuf, Vec<usize>> = HashMap::new();
    for (idx, (_, cfg)) in configs.iter().enumerate() {
        for w in &cfg.watch {
            let p = PathBuf::from(w);
            map.entry(p).or_default().push(idx);
        }
    }
    map
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn parse_minimal_config() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.birth.toml");
        let mut file = fs::File::create(&path).unwrap();
        write!(file, r#"project = "demo""#).unwrap();

        let cfg = load_config_file(&path).unwrap();
        assert_eq!(cfg.project, "demo");
        assert_eq!(cfg.build, "cargo build --release");
        assert_eq!(cfg.install, PathBuf::from("/usr/local/bin"));
        assert_eq!(cfg.debounce_ms, 500);
        assert!(cfg.watch.is_empty());
    }

    #[test]
    fn parse_full_config() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.birth.toml");
        fs::write(
            &path,
            r#"
project = "demo"
watch = ["src"]
build = "cargo build"
install = "/opt/bin"
restart = "demo.service"
debounce_ms = 1000
strip = true
backup = true
sudo = true
user = true
"#,
        )
        .unwrap();

        let cfg = load_config_file(&path).unwrap();
        assert_eq!(cfg.project, "demo");
        assert_eq!(
            cfg.watch,
            vec![dir.path().join("src").to_string_lossy().to_string()]
        );
        assert_eq!(cfg.build, "cargo build");
        assert_eq!(cfg.install, PathBuf::from("/opt/bin"));
        assert_eq!(cfg.restart, Some("demo.service".to_string()));
        assert_eq!(cfg.debounce_ms, 1000);
        assert!(cfg.strip);
        assert!(cfg.backup);
        assert!(cfg.sudo);
        assert!(cfg.user);
    }

    #[test]
    fn parse_invalid_config_returns_error() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("bad.birth.toml");
        fs::write(&path, "not valid toml = [").unwrap();

        let err = load_config_file(&path).unwrap_err();
        assert_eq!(err.kind(), crate::error::ErrorKind::ConfigParse);
    }

    #[test]
    fn path_to_project_map_builds_index() {
        let configs = vec![(
            PathBuf::from("/etc/birth.d/a.toml"),
            ProjectConfig {
                project: "a".into(),
                watch: vec!["src".into(), "Cargo.toml".into()],
                build: default_build(),
                install: default_install(),
                restart: None,
                debounce_ms: default_debounce(),
                strip: false,
                backup: false,
                sudo: false,
                user: false,
            },
        )];
        let map = path_to_project_map(&configs);
        assert_eq!(map.get(&PathBuf::from("src")), Some(&vec![0]));
        assert_eq!(map.get(&PathBuf::from("Cargo.toml")), Some(&vec![0]));
    }
}
