use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct ProjectConfig {
    pub project: String,
    #[serde(default)]
    pub watch: Vec<String>,
    #[serde(default = "default_build")]
    pub build: String,
    #[serde(default = "default_install")]
    pub install: PathBuf,
    #[serde(default)]
    pub restart: Option<String>,
    #[serde(default = "default_debounce")]
    pub debounce_ms: u64,
    #[serde(default)]
    pub strip: bool,
    #[serde(default)]
    pub backup: bool,
    #[serde(default)]
    pub sudo: bool,
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

pub fn load_config_file(path: &Path) -> Result<ProjectConfig, String> {
    let content = fs::read_to_string(path)
        .map_err(|e| format!("failed to read {}: {e}", path.display()))?;
    let mut cfg: ProjectConfig = toml::from_str(&content)
        .map_err(|e| format!("failed to parse {}: {e}", path.display()))?;

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
pub fn path_to_project_map(
    configs: &[(PathBuf, ProjectConfig)],
) -> HashMap<PathBuf, Vec<usize>> {
    let mut map: HashMap<PathBuf, Vec<usize>> = HashMap::new();
    for (idx, (_, cfg)) in configs.iter().enumerate() {
        for w in &cfg.watch {
            let p = PathBuf::from(w);
            map.entry(p).or_default().push(idx);
        }
    }
    map
}
