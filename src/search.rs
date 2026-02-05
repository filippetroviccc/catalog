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

pub fn search(
    store: &Store,
    _cfg: &Config,
    query: &str,
    ext: Option<&str>,
    tags: &[String],
    after: Option<&str>,
    before: Option<&str>,
    min_size: Option<u64>,
    max_size: Option<u64>,
    root: Option<&str>,
) -> Result<Vec<SearchEntry>> {
    let query_lc = query.to_lowercase();
    let mut root_filter: Option<i64> = None;
    if let Some(root) = root {
        let normalized = normalize_path_allow_missing(root)?;
        let root_str = path_to_string(&normalized);
        if let Some(entry) = store.data.roots.iter().find(|r| r.path == root_str) {
            root_filter = Some(entry.id);
        } else {
            return Ok(Vec::new());
        }
    }

    let ext_set: Option<HashSet<String>> = ext.and_then(|exts| {
        let set: HashSet<String> = exts
            .split(',')
            .map(|s| s.trim().to_lowercase())
            .filter(|s| !s.is_empty())
            .collect();
        if set.is_empty() {
            None
        } else {
            Some(set)
        }
    });

    let after_ts = match after {
        Some(v) => Some(parse_date_start(v)?),
        None => None,
    };
    let before_ts = match before {
        Some(v) => Some(parse_date_end_exclusive(v)?),
        None => None,
    };

    let tag_filter = normalize_tag_list(tags);
    let mut tag_ids = HashSet::new();
    if !tag_filter.is_empty() {
        let mut name_to_id = HashMap::new();
        for tag in &store.data.tags {
            name_to_id.insert(tag.name.clone(), tag.id);
        }
        for name in tag_filter {
            if let Some(id) = name_to_id.get(&name) {
                tag_ids.insert(*id);
            }
        }
        if tag_ids.is_empty() {
            return Ok(Vec::new());
        }
    }

    let mut file_tags = HashMap::new();
    if !tag_ids.is_empty() {
        for ft in &store.data.file_tags {
            file_tags.entry(ft.file_id).or_insert_with(Vec::new).push(ft.tag_id);
        }
    }

    let mut root_map = HashMap::new();
    for root in &store.data.roots {
        root_map.insert(root.id, root.path.clone());
    }

    let mut out = Vec::new();
    for file in &store.data.files {
        if file.status != "active" {
            continue;
        }
        if let Some(root_id) = root_filter {
            if file.root_id != root_id {
                continue;
            }
        }
        if let Some(ref set) = ext_set {
            match &file.ext {
                Some(ext) if set.contains(ext) => {}
                _ => continue,
            }
        }
        if let Some(ts) = after_ts {
            if file.mtime < ts {
                continue;
            }
        }
        if let Some(ts) = before_ts {
            if file.mtime >= ts {
                continue;
            }
        }
        if let Some(min) = min_size {
            if file.size < min as i64 {
                continue;
            }
        }
        if let Some(max) = max_size {
            if file.size > max as i64 {
                continue;
            }
        }
        if !file.abs_path.to_lowercase().contains(&query_lc) {
            continue;
        }
        if !tag_ids.is_empty() {
            let matched = file_tags
                .get(&file.id)
                .map(|ids| ids.iter().any(|id| tag_ids.contains(id)))
                .unwrap_or(false);
            if !matched {
                continue;
            }
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

    out.sort_by(|a, b| b.mtime.cmp(&a.mtime));
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

    out.sort_by(|a, b| b.mtime.cmp(&a.mtime));
    out.truncate(limit as usize);
    Ok(out)
}

fn normalize_tag_list(tags: &[String]) -> Vec<String> {
    let mut out = Vec::new();
    for t in tags {
        for part in t.split(',') {
            let s = part.trim().to_lowercase();
            if !s.is_empty() {
                out.push(s);
            }
        }
    }
    out
}

fn parse_date_start(date: &str) -> Result<i64> {
    let d = NaiveDate::parse_from_str(date, "%Y-%m-%d")
        .with_context(|| "invalid date, expected YYYY-MM-DD")?;
    Ok(Local
        .from_local_datetime(&d.and_hms_opt(0, 0, 0).unwrap())
        .single()
        .unwrap()
        .timestamp())
}

fn parse_date_end_exclusive(date: &str) -> Result<i64> {
    let d = NaiveDate::parse_from_str(date, "%Y-%m-%d")
        .with_context(|| "invalid date, expected YYYY-MM-DD")?;
    let next = d.succ_opt().unwrap_or(d);
    Ok(Local
        .from_local_datetime(&next.and_hms_opt(0, 0, 0).unwrap())
        .single()
        .unwrap()
        .timestamp())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{Config, OutputMode};
    use crate::{indexer, store, tags};
    use std::fs;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_dir(prefix: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir()
            .join(format!("catalog_test_{}_{}_{}", prefix, std::process::id(), nanos));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn write_file(path: &std::path::Path, contents: &str) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(path, contents).unwrap();
    }

    #[test]
    fn search_filters_and_tags_work() {
        let dir = temp_dir("search");
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

        let store_path = dir.join("catalog.json");
        let mut store = store::Store::load(&store_path).unwrap();
        indexer::run(&mut store, &cfg, false, false).unwrap();
        store.save().unwrap();

        let results = search(
            &store,
            &cfg,
            "file",
            Some("rs"),
            &[],
            None,
            None,
            None,
            None,
            None,
        )
        .unwrap();
        assert_eq!(results.len(), 1);
        assert!(results[0].path.ends_with("file2.rs"));

        tags::add_tag(&mut store.data, &file2.to_string_lossy(), "work").unwrap();
        let tagged = search(
            &store,
            &cfg,
            "file",
            None,
            &["work".to_string()],
            None,
            None,
            None,
            None,
            None,
        )
        .unwrap();
        assert_eq!(tagged.len(), 1);
        assert!(tagged[0].path.ends_with("file2.rs"));
    }
}
