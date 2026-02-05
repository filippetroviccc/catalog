use crate::config::Config;
use crate::roots;
use crate::store::{FileEntry, Store};
use crate::util::{normalize_path_allow_missing, path_to_string};
use anyhow::Result;
use chrono::Local;
use indicatif::{MultiProgress, ProgressBar, ProgressStyle};
use ignore::gitignore::{Gitignore, GitignoreBuilder};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use walkdir::{DirEntry, WalkDir};

pub struct IndexStats {
    pub seen: usize,
    pub updated: usize,
    pub deleted: usize,
    pub skipped: usize,
}

#[derive(Debug)]
struct ScannedFile {
    rel_path: String,
    abs_path: String,
    is_dir: bool,
    is_symlink: bool,
    size: i64,
    mtime: i64,
    ext: Option<String>,
}

struct RootScanResult {
    root_id: i64,
    root_path: String,
    files: Vec<ScannedFile>,
    stats: IndexStats,
    duration: Duration,
    root_missing: bool,
}

struct IgnoreMatcher {
    gitignore: Gitignore,
    abs_excludes: Vec<PathBuf>,
    include_hidden: bool,
}

pub fn run(
    store: &mut Store,
    cfg: &Config,
    full: bool,
    one_filesystem_override: bool,
) -> Result<IndexStats> {
    roots::sync_roots(&mut store.data, cfg, None)?;
    let run_id = store.data.next_run_id();

    let mut total_seen = 0;
    let mut total_updated = 0;
    let mut total_deleted = 0;
    let mut total_skipped = 0;

    let mut roots = store.data.roots.clone();
    roots.sort_by(|a, b| a.path.cmp(&b.path));

    let multi = MultiProgress::new();
    let mut bars: HashMap<i64, ProgressBar> = HashMap::new();
    let mut handles = Vec::new();

    for root in roots {
        let pb = multi.add(ProgressBar::new_spinner());
        bars.insert(root.id, pb.clone());
        let cfg = cfg.clone();
        let root_path = root.path.clone();
        let root_id = root.id;
        let one_fs = one_filesystem_override || root.one_filesystem;
        let pb_clone = pb.clone();
        handles.push(std::thread::spawn(move || {
            scan_root(&cfg, &root_path, root_id, one_fs, pb_clone)
        }));
    }

    let mut results = Vec::new();
    for handle in handles {
        let res = handle.join().expect("indexer thread panicked")?;
        results.push(res);
    }

    for result in results {
        let deleted = merge_root(&mut store.data, &result, run_id, full);
        total_seen += result.stats.seen;
        total_updated += result.stats.updated;
        total_deleted += deleted;
        total_skipped += result.stats.skipped;

        if let Some(pb) = bars.get(&result.root_id) {
            if result.root_missing {
                pb.finish_with_message(format!("Root missing: {}", result.root_path));
            } else {
                pb.finish_with_message(format!(
                    "Finished root: {} in {:.2}s (seen {}, updated {}, deleted {}, skipped {})",
                    result.root_path,
                    result.duration.as_secs_f64(),
                    result.stats.seen,
                    result.stats.updated,
                    deleted,
                    result.stats.skipped
                ));
            }
        }
    }

    Ok(IndexStats {
        seen: total_seen,
        updated: total_updated,
        deleted: total_deleted,
        skipped: total_skipped,
    })
}

fn scan_root(
    cfg: &Config,
    root: &str,
    root_id: i64,
    one_filesystem: bool,
    progress: ProgressBar,
) -> Result<RootScanResult> {
    let root_path = normalize_path_allow_missing(root)?;
    let started = Instant::now();

    let style = ProgressStyle::with_template("{spinner:.green} {msg}")
        .unwrap_or_else(|_| ProgressStyle::default_spinner());
    progress.set_style(style);
    progress.set_message(format!("Indexing root: {}", root));
    progress.enable_steady_tick(Duration::from_millis(120));

    if !root_path.exists() {
        tracing::warn!("root missing: {}", root);
        progress.set_message(format!("Root missing: {}", root));
        progress.disable_steady_tick();
        return Ok(RootScanResult {
            root_id,
            root_path: root.to_string(),
            files: Vec::new(),
            stats: IndexStats {
                seen: 0,
                updated: 0,
                deleted: 0,
                skipped: 0,
            },
            duration: started.elapsed(),
            root_missing: true,
        });
    }

    let matcher = build_matcher(cfg, root)?;

    let mut files = Vec::new();
    let mut seen = 0;
    let mut updated = 0;
    let mut skipped = 0;
    let mut permission_skips = 0;
    let mut walk_errors = 0;
    let mut first_walk_error: Option<String> = None;

    let walker = WalkDir::new(&root_path)
        .follow_links(false)
        .same_file_system(one_filesystem)
        .into_iter()
        .filter_entry(|entry| !should_skip(entry, &root_path, &matcher));

    for entry in walker {
        let entry = match entry {
            Ok(e) => e,
            Err(err) => {
                walk_errors += 1;
                if first_walk_error.is_none() {
                    first_walk_error = Some(err.to_string());
                }
                tracing::debug!("walk error: {}", err);
                skipped += 1;
                continue;
            }
        };

        if entry.path() == root_path {
            continue;
        }

        let ft = entry.file_type();
        let meta = match std::fs::symlink_metadata(entry.path()) {
            Ok(m) => m,
            Err(err) => {
                if err.kind() == std::io::ErrorKind::PermissionDenied {
                    permission_skips += 1;
                } else {
                    tracing::warn!("metadata error: {} ({})", entry.path().display(), err);
                }
                skipped += 1;
                continue;
            }
        };

        let rel = match entry.path().strip_prefix(&root_path) {
            Ok(p) => p,
            Err(_) => {
                skipped += 1;
                continue;
            }
        };

        let is_dir = ft.is_dir();
        let is_symlink = ft.is_symlink();
        let size = if is_dir { 0 } else { meta.len() as i64 };
        let mtime = meta
            .modified()
            .unwrap_or(SystemTime::UNIX_EPOCH)
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;
        let ext = rel
            .extension()
            .and_then(|s| s.to_str())
            .map(|s| s.to_lowercase());

        let abs_path = path_to_string(entry.path());
        let rel_path = path_to_string(rel);

        files.push(ScannedFile {
            rel_path,
            abs_path,
            is_dir,
            is_symlink,
            size,
            mtime,
            ext,
        });

        seen += 1;
        updated += 1;
        if seen % 5000 == 0 {
            progress.set_message(format!(
                "Indexing root: {} (seen {}, updated {}, skipped {})",
                root, seen, updated, skipped
            ));
        }
    }

    if walk_errors > 0 {
        if let Some(sample) = &first_walk_error {
            progress.println(format!(
                "Warning: {} walk errors under {} (e.g. {})",
                walk_errors, root, sample
            ));
        } else {
            progress.println(format!("Warning: {} walk errors under {}", walk_errors, root));
        }
    }
    if permission_skips > 0 {
        progress.println(format!(
            "Warning: skipped {} entries due to permissions under {}",
            permission_skips, root
        ));
    }

    progress.set_message(format!(
        "Scanned root: {} (seen {}, updated {}, skipped {})",
        root, seen, updated, skipped
    ));
    progress.disable_steady_tick();

    Ok(RootScanResult {
        root_id,
        root_path: root.to_string(),
        files,
        stats: IndexStats {
            seen,
            updated,
            deleted: 0,
            skipped,
        },
        duration: started.elapsed(),
        root_missing: false,
    })
}

fn merge_root(
    store: &mut crate::store::StoreData,
    result: &RootScanResult,
    run_id: i64,
    full: bool,
) -> usize {
    if result.root_missing {
        return 0;
    }

    let root_id = result.root_id;

    if full {
        for file in store.files.iter_mut().filter(|f| f.root_id == root_id) {
            file.status = "deleted".to_string();
        }
    }

    let mut file_index = HashMap::new();
    for (idx, file) in store.files.iter().enumerate() {
        if file.root_id == root_id {
            file_index.insert(file.rel_path.clone(), idx);
        }
    }

    for scanned in &result.files {
        if let Some(&idx) = file_index.get(&scanned.rel_path) {
            let file = &mut store.files[idx];
            file.abs_path = scanned.abs_path.clone();
            file.is_dir = scanned.is_dir;
            file.is_symlink = scanned.is_symlink;
            file.size = scanned.size;
            file.mtime = scanned.mtime;
            file.ext = scanned.ext.clone();
            file.status = "active".to_string();
            file.last_seen_run = run_id;
        } else {
            let id = store.next_file_id();
            store.files.push(FileEntry {
                id,
                root_id,
                rel_path: scanned.rel_path.clone(),
                abs_path: scanned.abs_path.clone(),
                is_dir: scanned.is_dir,
                is_symlink: scanned.is_symlink,
                size: scanned.size,
                mtime: scanned.mtime,
                ext: scanned.ext.clone(),
                status: "active".to_string(),
                last_seen_run: run_id,
            });
        }
    }

    let mut deleted = 0;
    for file in store.files.iter_mut().filter(|f| f.root_id == root_id) {
        if file.last_seen_run != run_id && file.status != "deleted" {
            file.status = "deleted".to_string();
            deleted += 1;
        }
    }

    let now = Local::now().to_rfc3339();
    if let Some(root_entry) = store.roots.iter_mut().find(|r| r.id == root_id) {
        root_entry.last_indexed_at = Some(now);
    }

    deleted
}

fn build_matcher(cfg: &Config, root: &str) -> Result<IgnoreMatcher> {
    let mut builder = GitignoreBuilder::new(root);
    let mut abs_excludes = Vec::new();

    for ex in &cfg.excludes {
        if ex.starts_with("~/") || ex.starts_with('/') {
            let abs = normalize_path_allow_missing(ex)?;
            abs_excludes.push(abs);
        } else {
            builder.add_line(None, ex)?;
        }
    }

    let gitignore = builder.build()?;
    Ok(IgnoreMatcher {
        gitignore,
        abs_excludes,
        include_hidden: cfg.include_hidden,
    })
}

fn should_skip(entry: &DirEntry, root: &Path, matcher: &IgnoreMatcher) -> bool {
    let path = entry.path();
    if !matcher.include_hidden && is_hidden(path, root) {
        return true;
    }

    for abs in &matcher.abs_excludes {
        if path == abs || path.starts_with(abs) {
            return true;
        }
    }

    let rel = match path.strip_prefix(root) {
        Ok(p) => p,
        Err(_) => path,
    };
    if matcher
        .gitignore
        .matched_path_or_any_parents(rel, entry.file_type().is_dir())
        .is_ignore()
    {
        return true;
    }

    false
}

fn is_hidden(path: &Path, root: &Path) -> bool {
    let rel = path.strip_prefix(root).unwrap_or(path);
    rel.components().any(|c| {
        let part = c.as_os_str().to_string_lossy();
        part.starts_with('.') && part != "." && part != ".."
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{Config, OutputMode};
    use crate::store;
    use std::fs;
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

    fn write_file(path: &Path, contents: &str) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(path, contents).unwrap();
    }

    #[test]
    fn indexer_respects_excludes_and_hidden_and_soft_delete() {
        let dir = temp_dir("indexer");
        let root = dir.join("root");
        fs::create_dir_all(&root).unwrap();

        let file1 = root.join("file1.txt");
        let file2 = root.join("sub/file2.rs");
        let ignored = root.join("node_modules/ignore.js");
        let hidden = root.join(".hidden/secret.txt");

        write_file(&file1, "a");
        write_file(&file2, "b");
        write_file(&ignored, "c");
        write_file(&hidden, "d");

        let root_canon = fs::canonicalize(&root).unwrap();
        let file1_canon = root_canon.join("file1.txt");
        let file2_canon = root_canon.join("sub/file2.rs");
        let ignored_canon = root_canon.join("node_modules/ignore.js");
        let hidden_canon = root_canon.join(".hidden/secret.txt");

        let cfg = Config {
            version: 1,
            output: OutputMode::Plain,
            include_hidden: false,
            one_filesystem: true,
            roots: vec![path_to_string(&root_canon)],
            excludes: vec!["**/node_modules/**".to_string()],
        };

        let store_path = dir.join("catalog.json");
        let mut store = store::Store::load(&store_path).unwrap();

        let stats = run(&mut store, &cfg, false, false).unwrap();
        assert!(stats.seen >= 2);

        let paths = store
            .data
            .files
            .iter()
            .filter(|f| f.status == "active" && !f.is_dir)
            .map(|f| f.abs_path.clone())
            .collect::<Vec<_>>();

        assert!(paths.contains(&path_to_string(&file1_canon)));
        assert!(paths.contains(&path_to_string(&file2_canon)));
        assert!(!paths.contains(&path_to_string(&ignored_canon)));
        assert!(!paths.contains(&path_to_string(&hidden_canon)));

        fs::remove_file(&file1).unwrap();
        let _ = run(&mut store, &cfg, false, false).unwrap();

        let status = store
            .data
            .files
            .iter()
            .find(|f| f.abs_path == path_to_string(&file1_canon))
            .map(|f| f.status.clone())
            .unwrap();
        assert_eq!(status, "deleted");
    }
}
