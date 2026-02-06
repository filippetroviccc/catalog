use crate::config::Config;
use crate::roots;
use crate::store::{DirSizeEntry, FileEntry, Store, StoreData};
use crate::util::{normalize_path_allow_missing, path_to_string};
use anyhow::Result;
use chrono::Local;
use indicatif::{MultiProgress, ProgressBar, ProgressStyle};
use ignore::gitignore::{Gitignore, GitignoreBuilder};
use ignore::{WalkBuilder, WalkState};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::marker::PhantomData;
use std::sync::mpsc;
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

pub struct IndexStats {
    pub seen: usize,
    pub updated: usize,
    pub deleted: usize,
    pub skipped: usize,
}

#[derive(Debug, Clone)]
pub struct ScannedFile {
    pub rel_path: String,
    pub abs_path: String,
    pub is_dir: bool,
    pub is_symlink: bool,
    pub size: i64,
    pub mtime: i64,
    pub ext: Option<String>,
}

struct RootScanResult {
    stats: IndexStats,
    duration: Duration,
    root_missing: bool,
}

pub trait ScanObserver {
    fn on_file_scanned(&mut self, root_path: &str, file: &ScannedFile);
    fn on_root_finished(&mut self, _root_path: &str) {}
}

#[derive(Copy, Clone)]
struct ObserverPtr<'a> {
    ptr: *mut (dyn ScanObserver + 'a),
    _marker: PhantomData<&'a mut dyn ScanObserver>,
}

impl<'a> ObserverPtr<'a> {
    fn new(observer: &'a mut dyn ScanObserver) -> Self {
        Self {
            ptr: observer,
            _marker: PhantomData,
        }
    }
}

struct IgnoreMatcher {
    gitignore: Gitignore,
    abs_excludes: Vec<PathBuf>,
    include_hidden: bool,
}

enum ScanEvent {
    File(ScannedFile),
    WalkError(String),
    MetadataError {
        path: String,
        error: String,
        permission_denied: bool,
    },
    RelPathError,
}

struct RootMerge {
    root_id: i64,
    run_id: i64,
    file_index: HashMap<String, usize>,
    indices: Vec<usize>,
}

impl RootMerge {
    fn new(store: &mut StoreData, root_id: i64, run_id: i64, full: bool) -> Self {
        let mut file_index = HashMap::new();
        let mut indices = Vec::new();
        for (idx, file) in store.files.iter_mut().enumerate() {
            if file.root_id == root_id {
                if full {
                    file.status = "deleted".to_string();
                }
                file_index.insert(file.rel_path.clone(), idx);
                indices.push(idx);
            }
        }
        Self {
            root_id,
            run_id,
            file_index,
            indices,
        }
    }

    fn apply(&mut self, store: &mut StoreData, scanned: ScannedFile) {
        if let Some(&idx) = self.file_index.get(&scanned.rel_path) {
            let file = &mut store.files[idx];
            file.abs_path = scanned.abs_path;
            file.is_dir = scanned.is_dir;
            file.is_symlink = scanned.is_symlink;
            file.size = scanned.size;
            file.mtime = scanned.mtime;
            file.ext = scanned.ext;
            file.status = "active".to_string();
            file.last_seen_run = self.run_id;
        } else {
            let id = store.next_file_id();
            let rel_key = scanned.rel_path.clone();
            let idx = store.files.len();
            store.files.push(FileEntry {
                id,
                root_id: self.root_id,
                rel_path: scanned.rel_path,
                abs_path: scanned.abs_path,
                is_dir: scanned.is_dir,
                is_symlink: scanned.is_symlink,
                size: scanned.size,
                mtime: scanned.mtime,
                ext: scanned.ext,
                status: "active".to_string(),
                last_seen_run: self.run_id,
            });
            self.file_index.insert(rel_key, idx);
            self.indices.push(idx);
        }
    }

    fn finalize(self, store: &mut StoreData) -> usize {
        let mut deleted = 0;
        for idx in self.indices {
            let file = &mut store.files[idx];
            if file.last_seen_run != self.run_id && file.status != "deleted" {
                file.status = "deleted".to_string();
                deleted += 1;
            }
        }

        let now = Local::now().to_rfc3339();
        if let Some(root_entry) = store.roots.iter_mut().find(|r| r.id == self.root_id) {
            root_entry.last_indexed_at = Some(now);
        }

        deleted
    }
}

pub fn run(
    store: &mut Store,
    cfg: &Config,
    full: bool,
    one_filesystem_override: bool,
) -> Result<IndexStats> {
    run_internal(store, cfg, full, one_filesystem_override, None)
}

pub fn run_with_observer(
    store: &mut Store,
    cfg: &Config,
    full: bool,
    one_filesystem_override: bool,
    observer: &mut dyn ScanObserver,
) -> Result<IndexStats> {
    run_internal(
        store,
        cfg,
        full,
        one_filesystem_override,
        Some(observer),
    )
}

fn run_internal(
    store: &mut Store,
    cfg: &Config,
    full: bool,
    one_filesystem_override: bool,
    observer: Option<&mut dyn ScanObserver>,
) -> Result<IndexStats> {
    roots::sync_roots(&mut store.data, cfg, None)?;
    let run_id = store.data.next_run_id();

    let mut total_seen = 0;
    let mut total_updated = 0;
    let mut total_deleted = 0;
    let mut total_skipped = 0;
    let mut dir_sizes: HashMap<PathBuf, u64> = HashMap::new();

    let mut roots = store.data.roots.clone();
    roots.sort_by(|a, b| a.path.cmp(&b.path));

    let multi = MultiProgress::new();
    let overall = multi.add(ProgressBar::new(roots.len() as u64));
    let overall_style =
        ProgressStyle::with_template("{bar:40.cyan/blue} {pos}/{len} | {msg}")
            .unwrap_or_else(|_| ProgressStyle::default_bar());
    overall.set_style(overall_style);
    overall.set_message("files 0 (updated 0, deleted 0, skipped 0)");

    let observer_ptr = observer.map(ObserverPtr::new);

    for root in roots {
        let pb = multi.add(ProgressBar::new_spinner());
        let one_fs = one_filesystem_override || root.one_filesystem;
        let result = scan_root(
            &mut store.data,
            cfg,
            &root.path,
            root.id,
            run_id,
            full,
            one_fs,
            pb.clone(),
            Some(&mut dir_sizes),
            observer_ptr,
        )?;

        total_seen += result.stats.seen;
        total_updated += result.stats.updated;
        total_deleted += result.stats.deleted;
        total_skipped += result.stats.skipped;
        overall.inc(1);
        overall.set_message(format!(
            "files {} (updated {}, deleted {}, skipped {})",
            total_seen, total_updated, total_deleted, total_skipped
        ));

        if result.root_missing {
            pb.finish_with_message("missing");
        } else {
            pb.finish_with_message(format!("{:.2}s", result.duration.as_secs_f64()));
        }

        let root_path = normalize_path_allow_missing(&root.path)?;
        dir_sizes.entry(root_path).or_insert(0);
    }

    overall.finish_with_message(format!(
        "files {} (updated {}, deleted {}, skipped {})",
        total_seen, total_updated, total_deleted, total_skipped
    ));

    if !dir_sizes.is_empty() {
        let mut entries = dir_sizes
            .into_iter()
            .map(|(path, size)| DirSizeEntry {
                path: path_to_string(&path),
                size,
            })
            .collect::<Vec<_>>();
        entries.sort_by(|a, b| a.path.cmp(&b.path));
        store.data.dir_sizes = entries;
        store.data.dir_sizes_run_id = run_id;
    } else {
        store.data.dir_sizes.clear();
        store.data.dir_sizes_run_id = run_id;
    }

    Ok(IndexStats {
        seen: total_seen,
        updated: total_updated,
        deleted: total_deleted,
        skipped: total_skipped,
    })
}

fn scan_root(
    store: &mut StoreData,
    cfg: &Config,
    root: &str,
    root_id: i64,
    run_id: i64,
    full: bool,
    one_filesystem: bool,
    progress: ProgressBar,
    mut dir_sizes: Option<&mut HashMap<PathBuf, u64>>,
    observer: Option<ObserverPtr<'_>>,
) -> Result<RootScanResult> {
    let root_path = normalize_path_allow_missing(root)?;
    let started = Instant::now();

    let style = ProgressStyle::with_template("{spinner:.green} {msg}")
        .unwrap_or_else(|_| ProgressStyle::default_spinner());
    progress.set_style(style);
    let root_label = root_path
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or(root);
    progress.set_message(format!("Indexing {}", root_label));
    progress.enable_steady_tick(Duration::from_millis(120));

    if !root_path.exists() {
        tracing::warn!("root missing: {}", root);
        progress.set_message(format!("Root missing: {}", root));
        progress.disable_steady_tick();
        return Ok(RootScanResult {
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

    let matcher = Arc::new(build_matcher(cfg, root)?);
    let mut merger = RootMerge::new(store, root_id, run_id, full);

    let (tx, rx) = mpsc::channel();
    let worker_root = root_path.clone();
    let worker_matcher = matcher.clone();
    let handle = thread::spawn(move || {
        let mut builder = WalkBuilder::new(&worker_root);
        builder
            .follow_links(false)
            .same_file_system(one_filesystem)
            .standard_filters(false);
        let walker = builder.build_parallel();
        walker.run(move || {
            let tx = tx.clone();
            let matcher = worker_matcher.clone();
            let root_path = worker_root.clone();
            Box::new(move |entry| {
                let entry = match entry {
                    Ok(e) => e,
                    Err(err) => {
                        let _ = tx.send(ScanEvent::WalkError(err.to_string()));
                        return WalkState::Continue;
                    }
                };

                let path = entry.path();
                if path == root_path.as_path() {
                    return WalkState::Continue;
                }

                let is_dir = entry
                    .file_type()
                    .map(|ft| ft.is_dir())
                    .unwrap_or(false);
                if should_skip(path, is_dir, &root_path, &matcher) {
                    return if is_dir {
                        WalkState::Skip
                    } else {
                        WalkState::Continue
                    };
                }

                let meta = match std::fs::symlink_metadata(path) {
                    Ok(m) => m,
                    Err(err) => {
                        let _ = tx.send(ScanEvent::MetadataError {
                            path: path_to_string(path),
                            error: err.to_string(),
                            permission_denied: err.kind()
                                == std::io::ErrorKind::PermissionDenied,
                        });
                        return WalkState::Continue;
                    }
                };

                let rel = match path.strip_prefix(&root_path) {
                    Ok(p) => p,
                    Err(_) => {
                        let _ = tx.send(ScanEvent::RelPathError);
                        return WalkState::Continue;
                    }
                };

                let is_symlink = entry.path_is_symlink();
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

                let abs_path = path_to_string(path);
                let rel_path = path_to_string(rel);

                let _ = tx.send(ScanEvent::File(ScannedFile {
                    rel_path,
                    abs_path,
                    is_dir,
                    is_symlink,
                    size,
                    mtime,
                    ext,
                }));

                WalkState::Continue
            })
        });
    });

    let mut seen = 0;
    let mut updated = 0;
    let mut skipped = 0;
    let mut permission_skips = 0;
    let mut walk_errors = 0;
    let mut first_walk_error: Option<String> = None;

    for event in rx {
        match event {
            ScanEvent::File(file) => {
                if let Some(obs) = observer {
                    unsafe {
                        (&mut *obs.ptr).on_file_scanned(root, &file);
                    }
                }
                if let Some(dir_sizes) = dir_sizes.as_deref_mut() {
                    if !file.is_dir {
                        let size = file.size.max(0) as u64;
                        if size > 0 {
                            let mut current = Path::new(&file.abs_path).parent();
                            while let Some(dir) = current {
                                if !dir.starts_with(&root_path) {
                                    break;
                                }
                                *dir_sizes.entry(dir.to_path_buf()).or_insert(0) += size;
                                if dir == root_path.as_path() {
                                    break;
                                }
                                current = dir.parent();
                            }
                        }
                    }
                }
                merger.apply(store, file);
                seen += 1;
                updated += 1;
                if seen % 5000 == 0 {
                    progress.set_message(format!(
                        "{} {}k (u{} s{})",
                        root_label,
                        seen / 1000,
                        updated / 1000,
                        skipped
                    ));
                }
            }
            ScanEvent::WalkError(err) => {
                walk_errors += 1;
                skipped += 1;
                if first_walk_error.is_none() {
                    first_walk_error = Some(err.clone());
                }
                tracing::debug!("walk error: {}", err);
            }
            ScanEvent::MetadataError {
                path,
                error,
                permission_denied,
            } => {
                skipped += 1;
                if permission_denied {
                    permission_skips += 1;
                } else {
                    tracing::warn!("metadata error: {} ({})", path, error);
                }
            }
            ScanEvent::RelPathError => {
                skipped += 1;
            }
        }
    }

    handle.join().expect("indexer worker panicked");

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
        "{} {}k (u{} s{})",
        root_label,
        seen / 1000,
        updated / 1000,
        skipped
    ));
    progress.disable_steady_tick();

    let deleted = merger.finalize(store);
    if let Some(obs) = observer {
        unsafe {
            (&mut *obs.ptr).on_root_finished(root);
        }
    }

    Ok(RootScanResult {
        stats: IndexStats {
            seen,
            updated,
            deleted,
            skipped,
        },
        duration: started.elapsed(),
        root_missing: false,
    })
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

fn should_skip(path: &Path, is_dir: bool, root: &Path, matcher: &IgnoreMatcher) -> bool {
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
        .matched_path_or_any_parents(rel, is_dir)
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

        let store_path = dir.join("catalog.bin");
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
