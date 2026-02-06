use crate::cli::Preset;
use crate::util::{expand_tilde, normalize_path, normalize_path_allow_missing, path_to_string};
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum OutputMode {
    Plain,
    Json,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Config {
    pub version: u32,
    pub output: OutputMode,
    pub include_hidden: bool,
    pub one_filesystem: bool,
    pub roots: Vec<String>,
    pub excludes: Vec<String>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            version: 1,
            output: OutputMode::Plain,
            include_hidden: false,
            one_filesystem: true,
            roots: Vec::new(),
            excludes: default_excludes(),
        }
    }
}

pub struct Paths {
    pub config_path: PathBuf,
    pub store_path: PathBuf,
}

impl Paths {
    pub fn resolve(config_override: Option<&str>, store_override: Option<&str>) -> Result<Self> {
        let config_path = match config_override {
            Some(p) => normalize_path_allow_missing(p)?,
            None => match std::env::var("CATALOG_CONFIG").ok() {
                Some(p) => normalize_path_allow_missing(&p)?,
                None => default_config_path()?,
            },
        };
        let store_path = match store_override {
            Some(p) => normalize_path_allow_missing(p)?,
            None => match std::env::var("CATALOG_STORE").ok() {
                Some(p) => normalize_path_allow_missing(&p)?,
                None => match std::env::var("CATALOG_DB").ok() {
                    Some(p) => normalize_path_allow_missing(&p)?,
                    None => default_store_path()?,
                },
            },
        };
        Ok(Self {
            config_path,
            store_path,
        })
    }
}

pub fn init(paths: &Paths, preset: Option<Preset>) -> Result<()> {
    ensure_parent_dir(&paths.config_path)?;
    ensure_parent_dir(&paths.store_path)?;

    let mut cfg = if paths.config_path.exists() {
        load(&paths.config_path)?
    } else {
        Config::default()
    };

    let default_preset = if preset.is_none() && !paths.config_path.exists() {
        Some(Preset::MacosUserAdditions)
    } else {
        None
    };

    if let Some(preset) = preset.or(default_preset) {
        apply_preset(&mut cfg, preset)?;
    }

    save(&paths.config_path, &cfg)?;
    Ok(())
}

pub fn load(path: &Path) -> Result<Config> {
    let data = fs::read_to_string(path)
        .with_context(|| format!("failed to read config: {}", path.display()))?;
    let cfg = toml::from_str(&data).context("failed to parse config")?;
    Ok(cfg)
}

pub fn save(path: &Path, cfg: &Config) -> Result<()> {
    ensure_parent_dir(path)?;
    let data = toml::to_string_pretty(cfg).context("failed to serialize config")?;
    fs::write(path, data).with_context(|| format!("failed to write config: {}", path.display()))?;
    Ok(())
}

pub fn apply_preset(cfg: &mut Config, preset: Preset) -> Result<()> {
    let roots = match preset {
        Preset::MacosUserAdditions => macos_user_additions_roots(),
        Preset::MacosDeep => {
            let mut r = macos_user_additions_roots();
            r.extend(macos_deep_roots());
            r
        }
    };
    let mut normalized = Vec::new();
    for root in roots {
        let expanded = expand_tilde(&root);
        if expanded.exists() {
            let canonical = normalize_path(&root)?;
            normalized.push(path_to_string(&canonical));
        }
    }
    cfg.roots = normalized;
    cfg.excludes = default_excludes();
    Ok(())
}

pub fn default_config_path() -> Result<PathBuf> {
    let home = std::env::var_os("HOME").context("HOME not set")?;
    Ok(PathBuf::from(home).join("Library/Application Support/catalog/config.toml"))
}

pub fn default_store_path() -> Result<PathBuf> {
    let home = std::env::var_os("HOME").context("HOME not set")?;
    Ok(PathBuf::from(home).join("Library/Application Support/catalog/catalog.bin"))
}

fn ensure_parent_dir(path: &Path) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create dir: {}", parent.display()))?;
    }
    Ok(())
}

fn macos_user_additions_roots() -> Vec<String> {
    let mut roots = vec![
        "~/Downloads",
        "~/Desktop",
        "~/Documents",
        "~/Library/Mobile Documents",
        "/Applications",
        "~/Applications",
        "/opt/homebrew",
        "/usr/local",
        "~/bin",
        "~/.local/bin",
        "~/.config",
        "~/Library/Preferences",
        "~/Library/LaunchAgents",
    ]
    .into_iter()
    .map(String::from)
    .collect::<Vec<_>>();

    let dev = expand_tilde("~/Developer");
    let projects = expand_tilde("~/Projects");
    if dev.exists() {
        roots.push(path_to_string(&dev));
    } else if projects.exists() {
        roots.push(path_to_string(&projects));
    }

    roots
}

fn macos_deep_roots() -> Vec<String> {
    vec![
        "/Library/LaunchAgents",
        "/Library/LaunchDaemons",
        "/Library/Fonts",
        "~/Library/Fonts",
        "/Library/PreferencePanes",
        "~/Library/PreferencePanes",
        "/etc",
    ]
    .into_iter()
    .map(String::from)
    .collect()
}

fn default_excludes() -> Vec<String> {
    vec![
        "~/Library/Caches",
        "~/Library/Containers",
        "~/Library/Logs",
        "~/Library/Developer/Xcode/DerivedData",
        "**/.git/**",
        "**/node_modules/**",
        "**/target/**",
        "**/dist/**",
        "**/build/**",
    ]
    .into_iter()
    .map(String::from)
    .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_dir(prefix: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir()
            .join(format!("catalog_test_{}_{}_{}", prefix, std::process::id(), nanos));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn config_round_trip() {
        let dir = temp_dir("config");
        let path = dir.join("config.toml");

        let cfg = Config {
            version: 1,
            output: OutputMode::Json,
            include_hidden: true,
            one_filesystem: false,
            roots: vec!["/tmp".to_string()],
            excludes: vec!["**/node_modules/**".to_string()],
        };

        save(&path, &cfg).unwrap();
        let loaded = load(&path).unwrap();
        assert_eq!(cfg, loaded);
    }
}
