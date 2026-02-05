use anyhow::{Context, Result};
use std::path::{Path, PathBuf};

pub fn expand_tilde(input: &str) -> PathBuf {
    if let Some(stripped) = input.strip_prefix("~/") {
        if let Some(home) = home_dir() {
            return home.join(stripped);
        }
    }
    if input == "~" {
        if let Some(home) = home_dir() {
            return home;
        }
    }
    PathBuf::from(input)
}

pub fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from)
}

pub fn normalize_path(input: &str) -> Result<PathBuf> {
    let expanded = expand_tilde(input);
    let absolute = if expanded.is_absolute() {
        expanded
    } else {
        std::env::current_dir().context("failed to read current dir")?.join(expanded)
    };
    let canonical = std::fs::canonicalize(&absolute)
        .with_context(|| format!("path does not exist: {}", absolute.display()))?;
    Ok(canonical)
}

pub fn normalize_path_allow_missing(input: &str) -> Result<PathBuf> {
    let expanded = expand_tilde(input);
    let absolute = if expanded.is_absolute() {
        expanded
    } else {
        std::env::current_dir().context("failed to read current dir")?.join(expanded)
    };
    if absolute.exists() {
        Ok(std::fs::canonicalize(&absolute)
            .with_context(|| format!("path does not exist: {}", absolute.display()))?)
    } else {
        Ok(absolute)
    }
}

pub fn path_to_string(path: &Path) -> String {
    path.to_string_lossy().to_string()
}
