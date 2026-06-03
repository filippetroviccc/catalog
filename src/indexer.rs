use crate::config::Config;
use crate::roots;
use crate::store::{DirSizeEntry, FileEntry, Store, StoreData};
use crate::util::{normalize_path_allow_missing, path_to_string};
use anyhow::Result;
use chrono::Local;
use ignore::gitignore::{Gitignore, GitignoreBuilder};
use ignore::{WalkBuilder, WalkState};
use indicatif::{MultiProgress, ProgressBar, ProgressStyle};
use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc;
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
    /// Whether this root's file set changed (drives per-root segment writes).
    changed: bool,
}

struct RootMergeStats {
    new_count: usize,
    updated: usize,
    deleted: usize,
    changed: bool,
}

pub trait ScanObserver {
    fn on_file_scanned(&mut self, root_path: &str, file: &ScannedFile);
    fn on_root_finished(&mut self, _root_path: &str) {}
}

/// Per-root inputs for [`scan_root`], grouped to keep the call site readable.
struct ScanParams<'a> {
    cfg: &'a Config,
    root: &'a str,
    root_id: i64,
    run_id: i64,
    full: bool,
    one_filesystem: bool,
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

/// Group `files` row indices by `root_id` in one pass, so each root's merge only
/// touches its own rows instead of re-scanning the whole store.
fn group_indices_by_root(files: &[FileEntry]) -> HashMap<i64, Vec<usize>> {
    let mut map: HashMap<i64, Vec<usize>> = HashMap::new();
    for (idx, file) in files.iter().enumerate() {
        map.entry(file.root_id).or_default().push(idx);
    }
    map
}

/// Reconcile a root's scanned files against its stored rows via a sorted merge-join
/// on `rel_path` — no per-file hashing, deletes fall out of the join, and unchanged
/// rows are left byte-identical (so the segment is only rewritten when something
/// actually changed).
fn merge_root(
    data: &mut StoreData,
    root_id: i64,
    run_id: i64,
    existing_idx: &[usize],
    mut scanned: Vec<ScannedFile>,
    full: bool,
) -> RootMergeStats {
    scanned.sort_by(|a, b| a.rel_path.cmp(&b.rel_path));
    let mut existing: Vec<usize> = existing_idx.to_vec();
    existing.sort_by(|&a, &b| data.files[a].rel_path.cmp(&data.files[b].rel_path));

    let mut new_count = 0;
    let mut updated = 0;
    let mut deleted = 0;
    let mut i = 0; // index into `existing`
    let mut j = 0; // index into `scanned`

    while i < existing.len() && j < scanned.len() {
        let ei = existing[i];
        let ord = data.files[ei]
            .rel_path
            .as_str()
            .cmp(scanned[j].rel_path.as_str());
        match ord {
            Ordering::Less => {
                // Stored key absent from this scan → soft delete.
                if data.files[ei].status != "deleted" {
                    data.files[ei].status = "deleted".to_string();
                    deleted += 1;
                }
                i += 1;
            }
            Ordering::Greater => {
                push_new(data, root_id, run_id, &scanned[j]);
                new_count += 1;
                j += 1;
            }
            Ordering::Equal => {
                if apply_if_changed(&mut data.files[ei], &scanned[j], run_id) {
                    updated += 1;
                }
                i += 1;
                j += 1;
            }
        }
    }
    while i < existing.len() {
        let ei = existing[i];
        if data.files[ei].status != "deleted" {
            data.files[ei].status = "deleted".to_string();
            deleted += 1;
        }
        i += 1;
    }
    while j < scanned.len() {
        push_new(data, root_id, run_id, &scanned[j]);
        new_count += 1;
        j += 1;
    }

    let changed = full || new_count > 0 || updated > 0 || deleted > 0;
    RootMergeStats {
        new_count,
        updated,
        deleted,
        changed,
    }
}

/// Overwrite a stored row from a fresh scan iff its tracked metadata differs.
/// Returns whether anything changed.
fn apply_if_changed(file: &mut FileEntry, s: &ScannedFile, run_id: i64) -> bool {
    let differs = file.status != "active"
        || file.size != s.size
        || file.mtime != s.mtime
        || file.is_symlink != s.is_symlink
        || file.is_dir != s.is_dir
        || file.ext != s.ext
        || file.abs_path != s.abs_path;
    if !differs {
        return false;
    }
    file.abs_path = s.abs_path.clone();
    file.is_dir = s.is_dir;
    file.is_symlink = s.is_symlink;
    file.size = s.size;
    file.mtime = s.mtime;
    file.ext = s.ext.clone();
    file.status = "active".to_string();
    file.last_seen_run = run_id;
    true
}

fn push_new(data: &mut StoreData, root_id: i64, run_id: i64, s: &ScannedFile) {
    let id = data.next_file_id();
    data.files.push(FileEntry {
        id,
        root_id,
        rel_path: s.rel_path.clone(),
        abs_path: s.abs_path.clone(),
        is_dir: s.is_dir,
        is_symlink: s.is_symlink,
        size: s.size,
        mtime: s.mtime,
        ext: s.ext.clone(),
        status: "active".to_string(),
        last_seen_run: run_id,
    });
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
    run_internal(store, cfg, full, one_filesystem_override, Some(observer))
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
    // One pass to map each root to its stored row indices; merge_root reuses these.
    let by_root = group_indices_by_root(&store.data.files);
    let mut dirty_roots: HashSet<i64> = HashSet::new();

    let mut roots = store.data.roots.clone();
    roots.sort_by(|a, b| a.path.cmp(&b.path));

    let multi = MultiProgress::new();
    let overall = multi.add(ProgressBar::new(roots.len() as u64));
    let overall_style = ProgressStyle::with_template("{bar:40.cyan/blue} {pos}/{len} | {msg}")
        .unwrap_or_else(|_| ProgressStyle::default_bar());
    overall.set_style(overall_style);
    overall.set_message("files 0 (updated 0, deleted 0, skipped 0)");

    // `observer` is only ever touched on this (single) merge thread, so a plain
    // reborrow across loop iterations is sufficient — no raw pointer needed.
    let mut observer = observer;

    for root in roots {
        let pb = multi.add(ProgressBar::new_spinner());
        let one_fs = one_filesystem_override || root.one_filesystem;
        let params = ScanParams {
            cfg,
            root: &root.path,
            root_id: root.id,
            run_id,
            full,
            one_filesystem: one_fs,
        };
        let existing = by_root.get(&root.id).map(|v| v.as_slice()).unwrap_or(&[]);
        let result = scan_root(
            &mut store.data,
            &params,
            pb.clone(),
            Some(&mut dir_sizes),
            observer.as_deref_mut(),
            existing,
        )?;
        if result.changed {
            dirty_roots.insert(root.id);
        }

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

    let mut entries = dir_sizes
        .into_iter()
        .map(|(path, size)| DirSizeEntry {
            path: path_to_string(&path),
            size,
        })
        .collect::<Vec<_>>();
    entries.sort_by(|a, b| a.path.cmp(&b.path));
    let dirsizes_changed = entries != store.data.dir_sizes;
    store.data.dir_sizes = entries;
    store.data.dir_sizes_run_id = run_id;

    // `--full` forces every root's segment to be rewritten even if unchanged.
    if full {
        for r in &store.data.roots {
            dirty_roots.insert(r.id);
        }
    }
    store.mark_dirty(dirty_roots, dirsizes_changed);

    Ok(IndexStats {
        seen: total_seen,
        updated: total_updated,
        deleted: total_deleted,
        skipped: total_skipped,
    })
}

fn scan_root(
    store: &mut StoreData,
    params: &ScanParams<'_>,
    progress: ProgressBar,
    mut dir_sizes: Option<&mut HashMap<PathBuf, u64>>,
    // `+ '_` keeps the trait-object lifetime independent of the `&mut` borrow, so the
    // caller can reborrow the same observer across its per-root loop.
    mut observer: Option<&mut (dyn ScanObserver + '_)>,
    existing_idx: &[usize],
) -> Result<RootScanResult> {
    let ScanParams {
        cfg,
        root,
        root_id,
        run_id,
        full,
        one_filesystem,
    } = *params;
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
            changed: false,
        });
    }

    let matcher = Arc::new(build_matcher(cfg, root)?);
    let (handle, rx) = spawn_walk(root_path.clone(), matcher, one_filesystem);

    // Collect the scan, then reconcile in one merge-join pass after the walk joins.
    let mut scanned: Vec<ScannedFile> = Vec::new();
    let mut seen = 0;
    let mut skipped = 0;
    let mut permission_skips = 0;
    let mut walk_errors = 0;
    let mut first_walk_error: Option<String> = None;

    for event in rx {
        match event {
            ScanEvent::File(file) => {
                if let Some(obs) = observer.as_deref_mut() {
                    obs.on_file_scanned(root, &file);
                }
                if let Some(dir_sizes) = dir_sizes.as_deref_mut() {
                    accumulate_dir_sizes(dir_sizes, &file, &root_path);
                }
                scanned.push(file);
                seen += 1;
                if seen % 5000 == 0 {
                    progress.set_message(format!("{} {}k (s{})", root_label, seen / 1000, skipped));
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
            progress.println(format!(
                "Warning: {} walk errors under {}",
                walk_errors, root
            ));
        }
    }
    if permission_skips > 0 {
        progress.println(format!(
            "Warning: skipped {} entries due to permissions under {}",
            permission_skips, root
        ));
    }

    let merge = merge_root(store, root_id, run_id, existing_idx, scanned, full);

    if let Some(root_entry) = store.roots.iter_mut().find(|r| r.id == root_id) {
        root_entry.last_indexed_at = Some(Local::now().to_rfc3339());
    }
    if let Some(obs) = observer {
        obs.on_root_finished(root);
    }

    let updated = merge.new_count + merge.updated;
    progress.set_message(format!(
        "{} {}k (u{} d{} s{})",
        root_label,
        seen / 1000,
        updated,
        merge.deleted,
        skipped
    ));
    progress.disable_steady_tick();

    Ok(RootScanResult {
        stats: IndexStats {
            seen,
            updated,
            deleted: merge.deleted,
            skipped,
        },
        duration: started.elapsed(),
        root_missing: false,
        changed: merge.changed,
    })
}

/// Spawn the parallel directory walk on a worker thread, streaming [`ScanEvent`]s
/// back over a channel. The walk produces events from many threads; the receiver
/// merges them single-threaded in [`scan_root`].
fn spawn_walk(
    root_path: PathBuf,
    matcher: Arc<IgnoreMatcher>,
    one_filesystem: bool,
) -> (thread::JoinHandle<()>, mpsc::Receiver<ScanEvent>) {
    let (tx, rx) = mpsc::channel();
    let handle = thread::spawn(move || {
        let mut builder = WalkBuilder::new(&root_path);
        builder
            .follow_links(false)
            .same_file_system(one_filesystem)
            .standard_filters(false);
        let walker = builder.build_parallel();
        walker.run(move || {
            let tx = tx.clone();
            let matcher = matcher.clone();
            let root_path = root_path.clone();
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

                let is_dir = entry.file_type().map(|ft| ft.is_dir()).unwrap_or(false);
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
                            permission_denied: err.kind() == std::io::ErrorKind::PermissionDenied,
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
    (handle, rx)
}

/// Add a file's size to every ancestor directory up to (and including) the root.
fn accumulate_dir_sizes(
    dir_sizes: &mut HashMap<PathBuf, u64>,
    file: &ScannedFile,
    root_path: &Path,
) {
    if file.is_dir {
        return;
    }
    let size = file.size.max(0) as u64;
    if size == 0 {
        return;
    }
    let mut current = Path::new(&file.abs_path).parent();
    while let Some(dir) = current {
        if !dir.starts_with(root_path) {
            break;
        }
        *dir_sizes.entry(dir.to_path_buf()).or_insert(0) += size;
        if dir == root_path {
            break;
        }
        current = dir.parent();
    }
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

    fn write_file(path: &Path, contents: &str) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(path, contents).unwrap();
    }

    #[test]
    fn indexer_respects_excludes_and_hidden_and_soft_delete() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
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

    #[test]
    fn reindex_marks_dirty_only_when_changed() {
        use crate::store::Dirty;

        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        let root = dir.join("root");
        fs::create_dir_all(&root).unwrap();
        write_file(&root.join("a.txt"), "a");
        write_file(&root.join("b.txt"), "bb");
        let root_canon = fs::canonicalize(&root).unwrap();

        let cfg = Config {
            version: 1,
            output: OutputMode::Plain,
            include_hidden: false,
            one_filesystem: true,
            roots: vec![path_to_string(&root_canon)],
            excludes: vec![],
        };

        let store_path = dir.join("catalog.bin");
        let mut store = store::Store::load(&store_path).unwrap();
        let root_id = {
            run(&mut store, &cfg, false, false).unwrap();
            store.data.roots[0].id
        };
        // First index: the root is dirty (everything new).
        match &store.dirty {
            Dirty::Parts { roots, .. } => assert!(roots.contains(&root_id)),
            Dirty::All => panic!("expected precise dirty set from indexer"),
        }

        // No filesystem change → nothing dirty.
        run(&mut store, &cfg, false, false).unwrap();
        match &store.dirty {
            Dirty::Parts { roots, dirsizes } => {
                assert!(roots.is_empty(), "no-op reindex should mark no roots dirty");
                assert!(!dirsizes, "no-op reindex should not dirty dir sizes");
            }
            Dirty::All => panic!("expected precise dirty set from indexer"),
        }

        // Change one file → that root becomes dirty again.
        write_file(&root.join("a.txt"), "a-much-longer-body");
        run(&mut store, &cfg, false, false).unwrap();
        match &store.dirty {
            Dirty::Parts { roots, .. } => assert!(roots.contains(&root_id)),
            Dirty::All => panic!("expected precise dirty set from indexer"),
        }
    }

    #[test]
    fn reindex_heals_after_segment_loss() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        let root = dir.join("root");
        fs::create_dir_all(&root).unwrap();
        write_file(&root.join("a.txt"), "a");
        write_file(&root.join("b.txt"), "bb");
        let root_canon = fs::canonicalize(&root).unwrap();

        let cfg = Config {
            version: 1,
            output: OutputMode::Plain,
            include_hidden: false,
            one_filesystem: true,
            roots: vec![path_to_string(&root_canon)],
            excludes: vec![],
        };
        let store_path = dir.join("catalog.bin");

        // Initial index + persist.
        {
            let mut store = store::Store::load(&store_path).unwrap();
            run(&mut store, &cfg, false, false).unwrap();
            store.save().unwrap();
        }

        // Simulate corruption: blow away the segment directory.
        let seg_dir = dir.join("catalog.bin.d");
        assert!(seg_dir.exists());
        fs::remove_dir_all(&seg_dir).unwrap();

        // Reload detects the loss and drops the unreadable rows.
        let mut store = store::Store::load(&store_path).unwrap();
        assert!(store.degraded, "missing segments should degrade load");
        assert!(store.data.files.is_empty());

        // Reindex rebuilds the root, and a fresh load is healthy again.
        run(&mut store, &cfg, false, false).unwrap();
        store.save().unwrap();

        let healed = store::Store::load(&store_path).unwrap();
        assert!(!healed.degraded, "reindex should clear the degraded state");
        let active = healed
            .data
            .files
            .iter()
            .filter(|f| f.status == "active" && !f.is_dir)
            .count();
        assert_eq!(active, 2, "both files repopulated after heal");
    }
}
