use anyhow::{Context, Result};
use chrono::{DateTime, Duration as ChronoDuration, Utc};
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::ffi::OsStr;
use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};

const STORE_VERSION: u32 = 3;

/// Magic prefix for the v3 segmented manifest. Last byte is the on-disk format
/// version; a legacy v2 single-blob store has no magic (its first bytes are a
/// bincode-encoded `u32` version, never `b"CATALOG"`).
const MAGIC: &[u8] = b"CATALOG\x03";

/// Advisory exclusive lock guarding mutating store operations.
///
/// `catalog` keeps the whole store in memory and rewrites it on `save()`, so two
/// concurrent mutating runs (e.g. `index` + `index`) would race: last writer wins
/// and the other run's changes — plus its `run_id`/`next_*_id` counter advances —
/// are lost. Mutating commands acquire this before loading the store; the OS flock
/// is released automatically when the handle drops.
pub struct StoreLock {
    _file: File,
    path: PathBuf,
}

impl StoreLock {
    /// Try to take the exclusive lock for `store_path`. Returns an error (rather than
    /// blocking) when another process already holds it.
    pub fn acquire(store_path: &Path) -> Result<Self> {
        ensure_parent_dir(store_path)?;
        let path = lock_path(store_path);
        let file = File::create(&path)
            .with_context(|| format!("failed to open lock file: {}", path.display()))?;
        match file.try_lock_exclusive() {
            Ok(()) => Ok(Self { _file: file, path }),
            Err(_) => anyhow::bail!(
                "another catalog process is running (lock held: {})",
                path.display()
            ),
        }
    }
}

impl Drop for StoreLock {
    fn drop(&mut self) {
        // Best-effort explicit unlock; the OS also releases on fd close.
        let _ = self._file.unlock();
        let _ = &self.path;
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoreData {
    #[serde(default = "default_version")]
    pub version: u32,
    #[serde(default)]
    pub last_run_id: i64,
    #[serde(default = "default_next_id")]
    pub next_root_id: i64,
    #[serde(default = "default_next_id")]
    pub next_file_id: i64,
    #[serde(default = "default_next_id")]
    pub next_tag_id: i64,
    #[serde(default)]
    pub roots: Vec<RootEntry>,
    #[serde(default)]
    pub files: Vec<FileEntry>,
    #[serde(default)]
    pub tags: Vec<TagEntry>,
    #[serde(default)]
    pub file_tags: Vec<FileTagEntry>,
    #[serde(default)]
    pub dir_sizes_run_id: i64,
    #[serde(default)]
    pub dir_sizes: Vec<DirSizeEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RootEntry {
    pub id: i64,
    pub path: String,
    pub added_at: String,
    pub preset_name: Option<String>,
    pub last_indexed_at: Option<String>,
    pub one_filesystem: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileEntry {
    pub id: i64,
    pub root_id: i64,
    pub rel_path: String,
    pub abs_path: String,
    pub is_dir: bool,
    pub is_symlink: bool,
    pub size: i64,
    pub mtime: i64,
    pub ext: Option<String>,
    pub status: String,
    pub last_seen_run: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TagEntry {
    pub id: i64,
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileTagEntry {
    pub file_id: i64,
    pub tag_id: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DirSizeEntry {
    pub path: String,
    pub size: u64,
}

/// What part of the store changed since load, so `save()` rewrites only that.
///
/// `All` (the default after `load`) is the safe fallback — any caller that mutates
/// `data` directly without declaring intent gets a full rewrite. The indexer instead
/// reports precise `Parts`, so a no-op reindex writes only the tiny manifest.
#[derive(Debug, Clone)]
pub enum Dirty {
    All,
    Parts { roots: HashSet<i64>, dirsizes: bool },
}

impl Dirty {
    fn clean() -> Self {
        Dirty::Parts {
            roots: HashSet::new(),
            dirsizes: false,
        }
    }
}

/// Serialize-only mirror of [`StoreData`] used to write the manifest without
/// cloning the (potentially huge) `files`/`dir_sizes` vectors — those live in
/// separate segment files. Field order MUST match `StoreData` exactly: the
/// manifest is read back as a `StoreData`, and bincode is positional.
#[derive(Serialize)]
struct ManifestRef<'a> {
    version: u32,
    last_run_id: i64,
    next_root_id: i64,
    next_file_id: i64,
    next_tag_id: i64,
    roots: &'a [RootEntry],
    files: &'a [FileEntry],
    tags: &'a [TagEntry],
    file_tags: &'a [FileTagEntry],
    dir_sizes_run_id: i64,
    dir_sizes: &'a [DirSizeEntry],
}

#[derive(Debug)]
pub struct Store {
    pub path: PathBuf,
    pub data: StoreData,
    /// Tracks which segments changed since load; consumed by `save()`.
    pub dirty: Dirty,
    /// Set when `load` recovered from a corrupt/missing manifest or segment.
    /// Affected roots are flagged stale so the next index rebuilds them.
    pub degraded: bool,
}

impl Store {
    pub fn load(path: &Path) -> Result<Self> {
        if !path.exists() {
            // No store yet: first save() writes everything.
            return Ok(Self {
                path: path.to_path_buf(),
                data: StoreData::new(),
                dirty: Dirty::All,
                degraded: false,
            });
        }

        let raw =
            fs::read(path).with_context(|| format!("failed to read store: {}", path.display()))?;

        if raw.starts_with(b"CATALOG") {
            Self::load_v3(path, &raw)
        } else {
            // Legacy v2 single-blob store: load, then migrate to v3 on next save.
            let mut data: StoreData =
                bincode::deserialize(&raw).context("failed to parse store binary")?;
            if data.version > 2 {
                anyhow::bail!("unsupported legacy store version {}", data.version);
            }
            data.version = STORE_VERSION;
            data.ensure_counters();
            Ok(Self {
                path: path.to_path_buf(),
                data,
                dirty: Dirty::All,
                degraded: false,
            })
        }
    }

    /// Load a v3 segmented store, self-healing around corruption rather than failing
    /// hard. A bad manifest resets to empty (rebuilt from config on next index); a bad
    /// or missing segment drops that root's rows and flags it stale for reindex. The
    /// persisted `next_*_id` counters are preserved so dropped ids are never reused.
    fn load_v3(path: &Path, raw: &[u8]) -> Result<Self> {
        // raw[7] is the on-disk format version. A newer format is a hard error
        // (don't silently rebuild a store written by a future version).
        let fmt = raw.get(7).copied().unwrap_or(0);
        if u32::from(fmt) > STORE_VERSION {
            anyhow::bail!(
                "unsupported store version {} (expected <= {})",
                fmt,
                STORE_VERSION
            );
        }

        let mut data: StoreData = match bincode::deserialize(&raw[MAGIC.len()..]) {
            Ok(d) => d,
            Err(_) => {
                // Manifest unreadable: nothing on disk is trustworthy. Start empty;
                // `index` rebuilds every configured root from scratch.
                tracing::warn!(
                    "store manifest is corrupt ({}); will rebuild on next index",
                    path.display()
                );
                let mut data = StoreData::new();
                data.ensure_counters();
                return Ok(Self {
                    path: path.to_path_buf(),
                    data,
                    dirty: Dirty::All,
                    degraded: true,
                });
            }
        };

        let dir = segments_dir(path);
        let mut files = Vec::new();
        let mut degraded = false;
        let roots = std::mem::take(&mut data.roots);
        let mut healed_roots = Vec::with_capacity(roots.len());
        for mut root in roots {
            match read_segment(&root_segment_path(&dir, root.id)) {
                Ok(mut rows) => files.append(&mut rows),
                Err(err) => {
                    // Keep the root, drop its (unreadable) rows, and mark it stale so
                    // the next index/analyze re-scans and rewrites a clean segment.
                    tracing::warn!(
                        "segment for root {} ({}) unreadable ({}); will reindex",
                        root.id,
                        root.path,
                        err
                    );
                    degraded = true;
                    root.last_indexed_at = None;
                }
            }
            healed_roots.push(root);
        }
        data.roots = healed_roots;
        data.files = files;

        match read_dirsizes(&dirsizes_segment_path(&dir)) {
            Ok(ds) => data.dir_sizes = ds,
            Err(_) => {
                // Non-fatal: the dir-size cache is derived, so just invalidate it.
                data.dir_sizes.clear();
                data.dir_sizes_run_id = 0;
            }
        }

        data.version = STORE_VERSION;
        data.ensure_counters();
        Ok(Self {
            path: path.to_path_buf(),
            data,
            // A degraded load forces a full, clean rewrite on the next save.
            dirty: if degraded { Dirty::All } else { Dirty::clean() },
            degraded,
        })
    }

    pub fn init(path: &Path) -> Result<Self> {
        let store = Self::load(path)?;
        store.save()?;
        Ok(store)
    }

    /// Declare precisely which segments changed (set by the indexer). `save()` then
    /// rewrites only those, plus the always-tiny manifest.
    pub fn mark_dirty(&mut self, roots: HashSet<i64>, dirsizes: bool) {
        self.dirty = Dirty::Parts { roots, dirsizes };
    }

    pub fn save(&self) -> Result<()> {
        let dir = segments_dir(&self.path);
        fs::create_dir_all(&dir)
            .with_context(|| format!("failed to create store dir: {}", dir.display()))?;

        let (write_all, dirty_roots, write_dirsizes) = match &self.dirty {
            Dirty::All => (true, None, true),
            Dirty::Parts { roots, dirsizes } => (false, Some(roots), *dirsizes),
        };
        let wants_root =
            |id: i64| write_all || dirty_roots.map(|s| s.contains(&id)).unwrap_or(false);

        // Group this run's writable roots' rows for segment serialization.
        let mut groups: HashMap<i64, Vec<&FileEntry>> = HashMap::new();
        for f in &self.data.files {
            if wants_root(f.root_id) {
                groups.entry(f.root_id).or_default().push(f);
            }
        }
        for root in &self.data.roots {
            let seg = root_segment_path(&dir, root.id);
            // Also write when the segment is absent, so every manifest root always has
            // a segment on disk — that makes a missing segment unambiguously corruption.
            if wants_root(root.id) || !seg.exists() {
                let rows = groups.remove(&root.id).unwrap_or_default();
                let bytes = bincode::serialize(&rows).context("failed to serialize segment")?;
                atomic_write(&seg, &bytes)?;
            }
        }

        let dirsizes_seg = dirsizes_segment_path(&dir);
        if write_dirsizes || !dirsizes_seg.exists() {
            let bytes = bincode::serialize(&self.data.dir_sizes)
                .context("failed to serialize dir sizes")?;
            atomic_write(&dirsizes_seg, &bytes)?;
        }

        // Drop segments for roots that no longer exist (e.g. after `rm`).
        let live: HashSet<i64> = self.data.roots.iter().map(|r| r.id).collect();
        if let Ok(entries) = fs::read_dir(&dir) {
            for entry in entries.flatten() {
                if let Some(id) = parse_root_segment(&entry.file_name())
                    && !live.contains(&id)
                {
                    let _ = fs::remove_file(entry.path());
                }
            }
        }

        // Manifest is always rewritten — it is small (no file rows) and its counters
        // / timestamps change on essentially every mutation.
        let manifest = ManifestRef {
            version: STORE_VERSION,
            last_run_id: self.data.last_run_id,
            next_root_id: self.data.next_root_id,
            next_file_id: self.data.next_file_id,
            next_tag_id: self.data.next_tag_id,
            roots: &self.data.roots,
            files: &[],
            tags: &self.data.tags,
            file_tags: &self.data.file_tags,
            dir_sizes_run_id: self.data.dir_sizes_run_id,
            dir_sizes: &[],
        };
        let mut bytes = MAGIC.to_vec();
        bytes.extend_from_slice(
            &bincode::serialize(&manifest).context("failed to serialize manifest")?,
        );
        atomic_write(&self.path, &bytes)?;
        Ok(())
    }

    pub fn export_json(&self) -> Result<String> {
        let json =
            serde_json::to_string_pretty(&self.data).context("failed to serialize store json")?;
        Ok(json)
    }
}

pub fn prune_store(path: &Path) -> Result<usize> {
    let mut removed = 0;
    if path.exists() {
        fs::remove_file(path)
            .with_context(|| format!("failed to remove store: {}", path.display()))?;
        removed += 1;
    }
    let dir = segments_dir(path);
    if dir.exists() {
        fs::remove_dir_all(&dir)
            .with_context(|| format!("failed to remove store segments: {}", dir.display()))?;
        removed += 1;
    }
    Ok(removed)
}

pub fn index_is_stale(store: &StoreData, filter: Option<&Path>, max_age: ChronoDuration) -> bool {
    let now = Utc::now();
    let mut any_relevant = false;

    for root in &store.roots {
        let root_path = Path::new(&root.path);
        if let Some(filter_path) = filter
            && !filter_path.starts_with(root_path)
            && !root_path.starts_with(filter_path)
        {
            continue;
        }
        any_relevant = true;
        let ts = match &root.last_indexed_at {
            Some(ts) => ts,
            None => return true,
        };
        let parsed: DateTime<Utc> = match DateTime::parse_from_rfc3339(ts) {
            Ok(dt) => dt.with_timezone(&Utc),
            Err(_) => return true,
        };
        if now.signed_duration_since(parsed) > max_age {
            return true;
        }
    }

    !any_relevant
}

impl Default for StoreData {
    fn default() -> Self {
        Self::new()
    }
}

impl StoreData {
    pub fn new() -> Self {
        Self {
            version: STORE_VERSION,
            last_run_id: 0,
            next_root_id: 1,
            next_file_id: 1,
            next_tag_id: 1,
            roots: Vec::new(),
            files: Vec::new(),
            tags: Vec::new(),
            file_tags: Vec::new(),
            dir_sizes_run_id: 0,
            dir_sizes: Vec::new(),
        }
    }

    pub fn ensure_counters(&mut self) {
        let max_root = self.roots.iter().map(|r| r.id).max().unwrap_or(0);
        let max_file = self.files.iter().map(|f| f.id).max().unwrap_or(0);
        let max_tag = self.tags.iter().map(|t| t.id).max().unwrap_or(0);
        if self.next_root_id <= max_root {
            self.next_root_id = max_root + 1;
        }
        if self.next_file_id <= max_file {
            self.next_file_id = max_file + 1;
        }
        if self.next_tag_id <= max_tag {
            self.next_tag_id = max_tag + 1;
        }
        if self.version == 0 {
            self.version = STORE_VERSION;
        }
    }

    pub fn next_root_id(&mut self) -> i64 {
        let id = self.next_root_id;
        self.next_root_id += 1;
        id
    }

    pub fn next_file_id(&mut self) -> i64 {
        let id = self.next_file_id;
        self.next_file_id += 1;
        id
    }

    pub fn next_run_id(&mut self) -> i64 {
        self.last_run_id += 1;
        self.last_run_id
    }
}

fn default_version() -> u32 {
    STORE_VERSION
}

fn default_next_id() -> i64 {
    1
}

fn ensure_parent_dir(path: &Path) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create dir: {}", parent.display()))?;
    }
    Ok(())
}

fn tmp_path(path: &Path) -> PathBuf {
    let mut tmp = path.to_path_buf();
    if let Some(name) = path.file_name() {
        let mut file = name.to_os_string();
        file.push(".tmp");
        tmp.set_file_name(file);
    } else {
        tmp.set_file_name("catalog.tmp");
    }
    tmp
}

fn lock_path(path: &Path) -> PathBuf {
    let mut lock = path.to_path_buf();
    if let Some(name) = path.file_name() {
        let mut file = name.to_os_string();
        file.push(".lock");
        lock.set_file_name(file);
    } else {
        lock.set_file_name("catalog.lock");
    }
    lock
}

/// Directory holding per-root + dir-size segment files (sibling of the manifest).
fn segments_dir(path: &Path) -> PathBuf {
    let mut dir = path.to_path_buf();
    if let Some(name) = path.file_name() {
        let mut file = name.to_os_string();
        file.push(".d");
        dir.set_file_name(file);
    } else {
        dir.set_file_name("catalog.d");
    }
    dir
}

fn root_segment_path(dir: &Path, root_id: i64) -> PathBuf {
    dir.join(format!("root-{root_id}.seg"))
}

fn dirsizes_segment_path(dir: &Path) -> PathBuf {
    dir.join("dirsizes.seg")
}

/// Parse a `root-<id>.seg` filename back to its root id.
fn parse_root_segment(name: &OsStr) -> Option<i64> {
    let s = name.to_str()?;
    s.strip_prefix("root-")?.strip_suffix(".seg")?.parse().ok()
}

/// Read a root segment (`Vec<FileEntry>`). Errors on a missing or unparseable file
/// so the caller can treat it as corruption and trigger a reindex.
fn read_segment(seg: &Path) -> Result<Vec<FileEntry>> {
    let bytes =
        fs::read(seg).with_context(|| format!("missing/unreadable segment: {}", seg.display()))?;
    bincode::deserialize(&bytes).with_context(|| format!("corrupt segment: {}", seg.display()))
}

fn read_dirsizes(seg: &Path) -> Result<Vec<DirSizeEntry>> {
    let bytes = fs::read(seg)
        .with_context(|| format!("missing/unreadable dir sizes: {}", seg.display()))?;
    bincode::deserialize(&bytes).with_context(|| format!("corrupt dir sizes: {}", seg.display()))
}

/// Write `bytes` to `path` atomically: temp file → fsync → rename.
fn atomic_write(path: &Path, bytes: &[u8]) -> Result<()> {
    ensure_parent_dir(path)?;
    let tmp = tmp_path(path);
    let mut file =
        File::create(&tmp).with_context(|| format!("failed to write: {}", tmp.display()))?;
    file.write_all(bytes)?;
    file.sync_all()?;
    fs::rename(&tmp, path).with_context(|| format!("failed to finalize: {}", path.display()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn store_round_trip_preserves_data() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("store.bin");

        let mut store = Store::load(&path).unwrap();
        let root_id = store.data.next_root_id();
        store.data.roots.push(RootEntry {
            id: root_id,
            path: "/tmp/root".to_string(),
            added_at: "now".to_string(),
            preset_name: Some("preset".to_string()),
            last_indexed_at: None,
            one_filesystem: true,
        });
        let file_id = store.data.next_file_id();
        store.data.files.push(FileEntry {
            id: file_id,
            root_id,
            rel_path: "file.txt".to_string(),
            abs_path: "/tmp/root/file.txt".to_string(),
            is_dir: false,
            is_symlink: false,
            size: 12,
            mtime: 123,
            ext: Some("txt".to_string()),
            status: "active".to_string(),
            last_seen_run: 1,
        });

        store.save().unwrap();

        let loaded = Store::load(&path).unwrap();
        assert_eq!(loaded.data.roots.len(), 1);
        assert_eq!(loaded.data.files.len(), 1);
        assert_eq!(loaded.data.roots[0].path, "/tmp/root");
        assert_eq!(loaded.data.files[0].abs_path, "/tmp/root/file.txt");
    }

    #[test]
    fn ensure_counters_advances_ids() {
        let mut data = StoreData::new();
        data.next_root_id = 1;
        data.next_file_id = 1;
        data.roots.push(RootEntry {
            id: 5,
            path: "/tmp/root".to_string(),
            added_at: "now".to_string(),
            preset_name: None,
            last_indexed_at: None,
            one_filesystem: true,
        });
        data.files.push(FileEntry {
            id: 7,
            root_id: 5,
            rel_path: "file.txt".to_string(),
            abs_path: "/tmp/root/file.txt".to_string(),
            is_dir: false,
            is_symlink: false,
            size: 12,
            mtime: 123,
            ext: Some("txt".to_string()),
            status: "active".to_string(),
            last_seen_run: 1,
        });
        data.ensure_counters();
        assert_eq!(data.next_root_id, 6);
        assert_eq!(data.next_file_id, 8);
    }

    #[test]
    fn export_json_round_trip() {
        let mut store = Store {
            path: PathBuf::from("/tmp/catalog.bin"),
            data: StoreData::new(),
            dirty: Dirty::All,
            degraded: false,
        };
        let root_id = store.data.next_root_id();
        let file_id = store.data.next_file_id();
        store.data.roots.push(RootEntry {
            id: root_id,
            path: "/tmp/root".to_string(),
            added_at: "now".to_string(),
            preset_name: None,
            last_indexed_at: None,
            one_filesystem: true,
        });
        store.data.files.push(FileEntry {
            id: file_id,
            root_id,
            rel_path: "file.txt".to_string(),
            abs_path: "/tmp/root/file.txt".to_string(),
            is_dir: false,
            is_symlink: false,
            size: 1,
            mtime: 2,
            ext: Some("txt".to_string()),
            status: "active".to_string(),
            last_seen_run: 1,
        });

        let json = store.export_json().unwrap();
        let decoded: StoreData = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded.roots.len(), 1);
        assert_eq!(decoded.files.len(), 1);
        assert_eq!(decoded.roots[0].path, "/tmp/root");
        assert_eq!(decoded.files[0].abs_path, "/tmp/root/file.txt");
    }

    #[test]
    fn index_is_stale_checks_age_and_missing() {
        let mut data = StoreData::new();
        data.roots.push(RootEntry {
            id: 1,
            path: "/root".to_string(),
            added_at: "now".to_string(),
            preset_name: None,
            last_indexed_at: Some((Utc::now() - ChronoDuration::hours(2)).to_rfc3339()),
            one_filesystem: true,
        });
        assert!(
            !index_is_stale(&data, None, ChronoDuration::days(1)),
            "recent index should not be stale"
        );
        assert!(
            index_is_stale(&data, None, ChronoDuration::hours(1)),
            "older than threshold should be stale"
        );

        data.roots[0].last_indexed_at = None;
        assert!(
            index_is_stale(&data, None, ChronoDuration::days(1)),
            "missing timestamp should be stale"
        );
    }

    #[test]
    fn index_is_stale_respects_filter() {
        let mut data = StoreData::new();
        data.roots.push(RootEntry {
            id: 1,
            path: "/root".to_string(),
            added_at: "now".to_string(),
            preset_name: None,
            last_indexed_at: Some((Utc::now() - ChronoDuration::hours(2)).to_rfc3339()),
            one_filesystem: true,
        });
        let filter = Path::new("/root/sub");
        assert!(
            !index_is_stale(&data, Some(filter), ChronoDuration::days(1)),
            "filter within root should use root timestamp"
        );
        let filter = Path::new("/other");
        assert!(
            index_is_stale(&data, Some(filter), ChronoDuration::days(1)),
            "no relevant roots should be stale"
        );
    }

    #[test]
    fn loads_and_migrates_legacy_v2_blob() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("catalog.bin");

        // Hand-write a legacy v2 single-blob store (no magic prefix).
        let mut legacy = StoreData::new();
        legacy.version = 2;
        legacy.roots.push(RootEntry {
            id: 1,
            path: "/tmp/root".to_string(),
            added_at: "now".to_string(),
            preset_name: None,
            last_indexed_at: None,
            one_filesystem: true,
        });
        legacy.files.push(FileEntry {
            id: 1,
            root_id: 1,
            rel_path: "a.txt".to_string(),
            abs_path: "/tmp/root/a.txt".to_string(),
            is_dir: false,
            is_symlink: false,
            size: 5,
            mtime: 1,
            ext: Some("txt".to_string()),
            status: "active".to_string(),
            last_seen_run: 1,
        });
        fs::write(&path, bincode::serialize(&legacy).unwrap()).unwrap();

        // Load migrates in-memory and flags a full rewrite.
        let store = Store::load(&path).unwrap();
        assert_eq!(store.data.version, STORE_VERSION);
        assert_eq!(store.data.files.len(), 1);
        assert!(matches!(store.dirty, Dirty::All));

        // Saving converts to v3 on disk (magic prefix + segment).
        store.save().unwrap();
        let raw = fs::read(&path).unwrap();
        assert!(raw.starts_with(b"CATALOG"));
        assert!(segments_dir(&path).join("root-1.seg").exists());

        // Reloading the v3 layout preserves the data.
        let reloaded = Store::load(&path).unwrap();
        assert_eq!(reloaded.data.files.len(), 1);
        assert_eq!(reloaded.data.files[0].abs_path, "/tmp/root/a.txt");
    }

    /// Build and persist a minimal one-root, one-file v3 store at `path`.
    fn seed_v3(path: &Path) -> i64 {
        let mut store = Store::load(path).unwrap();
        let rid = store.data.next_root_id();
        store.data.roots.push(RootEntry {
            id: rid,
            path: "/r".to_string(),
            added_at: "now".to_string(),
            preset_name: None,
            last_indexed_at: Some("2026-01-01T00:00:00Z".to_string()),
            one_filesystem: true,
        });
        let fid = store.data.next_file_id();
        store.data.files.push(FileEntry {
            id: fid,
            root_id: rid,
            rel_path: "a".to_string(),
            abs_path: "/r/a".to_string(),
            is_dir: false,
            is_symlink: false,
            size: 1,
            mtime: 1,
            ext: None,
            status: "active".to_string(),
            last_seen_run: 1,
        });
        store.save().unwrap();
        rid
    }

    #[test]
    fn recovers_from_missing_segment() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("catalog.bin");
        let rid = seed_v3(&path);
        let next_file_id_before = Store::load(&path).unwrap().data.next_file_id;

        let seg = segments_dir(&path).join(format!("root-{rid}.seg"));
        assert!(seg.exists());
        fs::remove_file(&seg).unwrap();

        let reloaded = Store::load(&path).unwrap();
        assert!(reloaded.degraded, "missing segment should degrade");
        assert!(reloaded.data.files.is_empty(), "unreadable rows dropped");
        assert_eq!(reloaded.data.roots.len(), 1, "root kept for reindex");
        assert!(
            reloaded.data.roots[0].last_indexed_at.is_none(),
            "degraded root flagged stale"
        );
        assert!(matches!(reloaded.dirty, Dirty::All));
        assert_eq!(
            reloaded.data.next_file_id, next_file_id_before,
            "persisted counter preserved so dropped ids are never reused"
        );
    }

    #[test]
    fn recovers_from_corrupt_segment() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("catalog.bin");
        let rid = seed_v3(&path);

        fs::write(
            segments_dir(&path).join(format!("root-{rid}.seg")),
            b"not valid bincode",
        )
        .unwrap();

        let reloaded = Store::load(&path).unwrap();
        assert!(reloaded.degraded, "corrupt segment should degrade");
        assert!(reloaded.data.files.is_empty());
        assert_eq!(reloaded.data.roots.len(), 1);
    }

    #[test]
    fn recovers_from_corrupt_manifest() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("catalog.bin");
        seed_v3(&path);

        // Keep the magic so it is read as v3, but garble the manifest body.
        let mut bytes = b"CATALOG\x03".to_vec();
        bytes.extend_from_slice(b"garbage-not-a-manifest");
        fs::write(&path, bytes).unwrap();

        let reloaded = Store::load(&path).unwrap();
        assert!(reloaded.degraded, "corrupt manifest should degrade");
        assert!(
            reloaded.data.roots.is_empty(),
            "manifest loss resets to empty; index rebuilds from config"
        );
        assert!(matches!(reloaded.dirty, Dirty::All));
    }

    #[test]
    fn rejects_newer_format_version() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("catalog.bin");
        seed_v3(&path);
        // Bump the on-disk format byte past what we support.
        let mut bytes = fs::read(&path).unwrap();
        bytes[7] = 9;
        fs::write(&path, bytes).unwrap();
        assert!(Store::load(&path).is_err(), "newer format must fail fast");
    }

    #[test]
    fn store_lock_is_exclusive() {
        let tmp = tempfile::tempdir().unwrap();
        let store_path = tmp.path().join("catalog.bin");
        let lock = StoreLock::acquire(&store_path).expect("first acquire succeeds");
        assert!(
            StoreLock::acquire(&store_path).is_err(),
            "second acquire while held must fail"
        );
        drop(lock);
        assert!(
            StoreLock::acquire(&store_path).is_ok(),
            "acquire after release succeeds"
        );
    }
}
