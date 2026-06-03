use crate::config::Config;
use crate::store::Store;
use crate::util::{normalize_path_allow_missing, path_to_string};
use anyhow::{Context, Result};
use chrono::{Local, NaiveDate, TimeZone};
use std::collections::{HashMap, HashSet};

#[derive(Debug, serde::Serialize)]
pub struct SearchEntry {
    pub id: i64,
    pub path: String,
    pub mtime: i64,
    pub size: i64,
    pub is_dir: bool,
    pub is_symlink: bool,
    pub ext: Option<String>,
    pub root: String,
    pub status: String,
}

/// Optional filters applied on top of the substring query in [`search`].
#[derive(Debug, Default)]
pub struct SearchFilters<'a> {
    pub ext: Option<&'a str>,
    pub after: Option<&'a str>,
    pub before: Option<&'a str>,
    pub min_size: Option<u64>,
    pub max_size: Option<u64>,
    pub root: Option<&'a str>,
}

pub fn search(
    store: &Store,
    _cfg: &Config,
    query: &str,
    filters: &SearchFilters,
) -> Result<Vec<SearchEntry>> {
    let query_lc = query.to_lowercase();
    let mut root_filter: Option<i64> = None;
    if let Some(root) = filters.root {
        let normalized = normalize_path_allow_missing(root)?;
        let root_str = path_to_string(&normalized);
        if let Some(entry) = store.data.roots.iter().find(|r| r.path == root_str) {
            root_filter = Some(entry.id);
        } else {
            return Ok(Vec::new());
        }
    }

    let ext_set: Option<HashSet<String>> = filters.ext.and_then(|exts| {
        let set: HashSet<String> = exts
            .split(',')
            .map(|s| s.trim().to_lowercase())
            .filter(|s| !s.is_empty())
            .collect();
        if set.is_empty() { None } else { Some(set) }
    });

    let after_ts = match filters.after {
        Some(v) => Some(parse_date_start(v)?),
        None => None,
    };
    let before_ts = match filters.before {
        Some(v) => Some(parse_date_end_exclusive(v)?),
        None => None,
    };

    let mut root_map = HashMap::new();
    for root in &store.data.roots {
        root_map.insert(root.id, root.path.clone());
    }

    let mut out = Vec::new();
    for file in &store.data.files {
        if file.status != "active" {
            continue;
        }
        if let Some(root_id) = root_filter
            && file.root_id != root_id
        {
            continue;
        }
        if let Some(ref set) = ext_set {
            match &file.ext {
                Some(ext) if set.contains(ext) => {}
                _ => continue,
            }
        }
        if let Some(ts) = after_ts
            && file.mtime < ts
        {
            continue;
        }
        if let Some(ts) = before_ts
            && file.mtime >= ts
        {
            continue;
        }
        if let Some(min) = filters.min_size
            && file.size < min as i64
        {
            continue;
        }
        if let Some(max) = filters.max_size
            && file.size > max as i64
        {
            continue;
        }
        if !matches_query(&file.abs_path, &query_lc) {
            continue;
        }
        let root_path = root_map
            .get(&file.root_id)
            .cloned()
            .unwrap_or_else(|| "-".to_string());

        out.push(SearchEntry {
            id: file.id,
            path: file.abs_path.clone(),
            mtime: file.mtime,
            size: file.size,
            is_dir: file.is_dir,
            is_symlink: file.is_symlink,
            ext: file.ext.clone(),
            root: root_path,
            status: file.status.clone(),
        });
    }

    out.sort_by_key(|e| std::cmp::Reverse(e.mtime));
    Ok(out)
}

pub fn recent(
    store: &Store,
    _cfg: &Config,
    days: Option<u32>,
    limit: Option<u32>,
) -> Result<Vec<SearchEntry>> {
    let days = days.unwrap_or(7) as i64;
    let limit = limit.unwrap_or(50) as i64;
    let now = Local::now().timestamp();
    let threshold = now - (days * 86400);
    let mut root_map = HashMap::new();
    for root in &store.data.roots {
        root_map.insert(root.id, root.path.clone());
    }

    let mut out = Vec::new();
    for file in &store.data.files {
        if file.status != "active" || file.mtime < threshold {
            continue;
        }
        let root_path = root_map
            .get(&file.root_id)
            .cloned()
            .unwrap_or_else(|| "-".to_string());
        out.push(SearchEntry {
            id: file.id,
            path: file.abs_path.clone(),
            mtime: file.mtime,
            size: file.size,
            is_dir: file.is_dir,
            is_symlink: file.is_symlink,
            ext: file.ext.clone(),
            root: root_path,
            status: file.status.clone(),
        });
    }

    out.sort_by_key(|e| std::cmp::Reverse(e.mtime));
    out.truncate(limit as usize);
    Ok(out)
}

/// Case-insensitive substring test. `needle_lc` must already be lowercased.
///
/// Hot path in the linear search scan: the previous `haystack.to_lowercase()`
/// allocated a `String` for every file on every query. When both sides are ASCII
/// (the common case for paths) we fold case byte-by-byte with no allocation; the
/// rarer non-ASCII case falls back to the allocating Unicode path, which keeps the
/// matching semantics identical.
fn matches_query(haystack: &str, needle_lc: &str) -> bool {
    if needle_lc.is_empty() {
        return true;
    }
    if haystack.is_ascii() && needle_lc.is_ascii() {
        let hay = haystack.as_bytes();
        let needle = needle_lc.as_bytes();
        if needle.len() > hay.len() {
            return false;
        }
        (0..=hay.len() - needle.len()).any(|i| {
            hay[i..i + needle.len()]
                .iter()
                .zip(needle)
                .all(|(h, n)| h.to_ascii_lowercase() == *n)
        })
    } else {
        haystack.to_lowercase().contains(needle_lc)
    }
}

fn parse_date_start(date: &str) -> Result<i64> {
    let d = NaiveDate::parse_from_str(date, "%Y-%m-%d")
        .with_context(|| "invalid date, expected YYYY-MM-DD")?;
    let naive = d
        .and_hms_opt(0, 0, 0)
        .context("invalid time-of-day for date")?;
    // `.earliest()` (vs `.single().unwrap()`) avoids a panic on DST spring-forward
    // boundaries where local midnight may be skipped or ambiguous.
    Ok(Local
        .from_local_datetime(&naive)
        .earliest()
        .with_context(|| format!("no valid local time for date {date}"))?
        .timestamp())
}

fn parse_date_end_exclusive(date: &str) -> Result<i64> {
    let d = NaiveDate::parse_from_str(date, "%Y-%m-%d")
        .with_context(|| "invalid date, expected YYYY-MM-DD")?;
    let next = d.succ_opt().unwrap_or(d);
    let naive = next
        .and_hms_opt(0, 0, 0)
        .context("invalid time-of-day for date")?;
    Ok(Local
        .from_local_datetime(&naive)
        .latest()
        .with_context(|| format!("no valid local time for date {date}"))?
        .timestamp())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{Config, OutputMode};
    use crate::{indexer, store};
    use std::fs;

    fn write_file(path: &std::path::Path, contents: &str) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(path, contents).unwrap();
    }

    #[test]
    fn search_filters_work() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        let root = dir.join("root");
        fs::create_dir_all(&root).unwrap();

        let file1 = root.join("file1.txt");
        let file2 = root.join("sub/file2.rs");
        write_file(&file1, "hello");
        write_file(&file2, "world");

        let cfg = Config {
            version: 1,
            output: OutputMode::Plain,
            include_hidden: false,
            one_filesystem: true,
            roots: vec![root.to_string_lossy().to_string()],
            excludes: vec![],
        };

        let store_path = dir.join("catalog.bin");
        let mut store = store::Store::load(&store_path).unwrap();
        indexer::run(&mut store, &cfg, false, false).unwrap();
        store.save().unwrap();

        let filters = SearchFilters {
            ext: Some("rs"),
            ..Default::default()
        };
        let results = search(&store, &cfg, "file", &filters).unwrap();
        assert_eq!(results.len(), 1);
        assert!(results[0].path.ends_with("file2.rs"));
    }

    #[test]
    fn matches_query_is_case_insensitive() {
        assert!(matches_query("/Users/me/Alpha.TXT", "alpha"));
        assert!(matches_query("/Users/me/Alpha.TXT", ".txt"));
        assert!(!matches_query("/Users/me/Alpha.TXT", "beta"));
        assert!(matches_query("anything", "")); // empty needle matches all
        assert!(!matches_query("ab", "abc")); // needle longer than haystack
        // Unicode fallback path (non-ASCII) still folds case.
        assert!(matches_query("/tmp/CafÉ.txt", "café"));
    }

    #[test]
    fn date_parsing_does_not_panic_on_dst_boundary() {
        // US spring-forward 2024-03-10 (local midnight may be skipped/ambiguous in
        // some zones). These must return a Result, never panic.
        for date in ["2024-03-10", "2024-03-11", "2024-01-01", "2024-12-31"] {
            assert!(parse_date_start(date).is_ok(), "start {date}");
            assert!(parse_date_end_exclusive(date).is_ok(), "end {date}");
        }
        assert!(parse_date_start("not-a-date").is_err());
    }
}
