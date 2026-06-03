use anyhow::{Context, Result};
use std::io::{self, IsTerminal, Write};
use std::path::{Path, PathBuf};

pub fn expand_tilde(input: &str) -> PathBuf {
    if let Some(stripped) = input.strip_prefix("~/")
        && let Some(home) = home_dir()
    {
        return home.join(stripped);
    }
    if input == "~"
        && let Some(home) = home_dir()
    {
        return home;
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
        std::env::current_dir()
            .context("failed to read current dir")?
            .join(expanded)
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
        std::env::current_dir()
            .context("failed to read current dir")?
            .join(expanded)
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

/// Format a byte count as a compact human-readable size (e.g. `1.5MB`).
pub fn human_size(bytes: u64) -> String {
    let units = ["B", "KB", "MB", "GB", "TB"];
    let mut value = bytes as f64;
    let mut idx = 0;
    while value >= 1024.0 && idx < units.len() - 1 {
        value /= 1024.0;
        idx += 1;
    }
    if idx == 0 {
        format!("{}{}", bytes, units[idx])
    } else {
        format!("{:.1}{}", value, units[idx])
    }
}

/// Ask the user to confirm a destructive action on an interactive terminal.
///
/// Returns `false` (decline) when stdin is not a TTY, so piped/CI invocations are
/// never silently confirmed — callers should require `--yes` for non-interactive use.
pub fn confirm(prompt: &str) -> Result<bool> {
    if !io::stdin().is_terminal() {
        return Ok(false);
    }
    print!("{prompt} [y/N] ");
    io::stdout().flush().ok();
    let mut input = String::new();
    io::stdin()
        .read_line(&mut input)
        .context("failed to read confirmation from stdin")?;
    let ans = input.trim().to_lowercase();
    Ok(ans == "y" || ans == "yes")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn human_size_formats_units() {
        assert_eq!(human_size(0), "0B");
        assert_eq!(human_size(512), "512B");
        assert_eq!(human_size(1023), "1023B");
        assert_eq!(human_size(1024), "1.0KB");
        assert_eq!(human_size(1536), "1.5KB");
        assert_eq!(human_size(1024 * 1024), "1.0MB");
        assert_eq!(human_size(1024 * 1024 * 1024), "1.0GB");
    }

    #[test]
    fn expand_tilde_handles_home_and_absolute_and_relative() {
        let home = home_dir().expect("HOME set in test env");
        assert_eq!(expand_tilde("~/foo/bar"), home.join("foo/bar"));
        assert_eq!(expand_tilde("~"), home);
        assert_eq!(expand_tilde("/abs/path"), PathBuf::from("/abs/path"));
        // A path that merely starts with '~' but isn't the home shorthand is literal.
        assert_eq!(expand_tilde("~user/foo"), PathBuf::from("~user/foo"));
        assert_eq!(expand_tilde("rel/path"), PathBuf::from("rel/path"));
    }

    #[test]
    fn normalize_path_allow_missing_keeps_nonexistent_absolute() {
        let p = normalize_path_allow_missing("/no/such/path/xyz123").unwrap();
        assert_eq!(p, PathBuf::from("/no/such/path/xyz123"));
    }

    #[test]
    fn normalize_path_rejects_missing() {
        assert!(normalize_path("/no/such/path/xyz123").is_err());
    }
}
