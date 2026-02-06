use crate::indexer::{ScanObserver, ScannedFile};
use anyhow::Result;
use serde::Serialize;
use std::cmp::Reverse;
use std::collections::{BinaryHeap, HashMap, HashSet};
use std::path::{Path, PathBuf};

#[derive(Debug, Serialize)]
pub struct UsageEntry {
    pub path: String,
    pub size: u64,
}

#[derive(Debug, Serialize)]
pub struct AnalysisResult {
    pub total_scanned: u64,
    pub roots: Vec<UsageEntry>,
    pub top_dirs: Vec<UsageEntry>,
    pub top_files: Vec<UsageEntry>,
}

#[derive(Debug, Clone)]
pub struct BrowseEntry {
    pub path: PathBuf,
    pub size: u64,
    pub is_dir: bool,
}

#[derive(Debug)]
pub struct BrowseIndex {
    pub total_scanned: u64,
    pub root_entries: Vec<BrowseEntry>,
    pub dir_totals: HashMap<PathBuf, u64>,
    pub file_sizes: HashMap<PathBuf, u64>,
    pub children: HashMap<PathBuf, Vec<BrowseEntry>>,
}

impl BrowseIndex {
    pub fn children_for(&self, path: Option<&Path>) -> Vec<BrowseEntry> {
        match path {
            Some(p) => self.children.get(p).cloned().unwrap_or_default(),
            None => self.root_entries.clone(),
        }
    }

    pub fn total_for(&self, path: Option<&Path>) -> u64 {
        match path {
            Some(p) => self
                .dir_totals
                .get(p)
                .copied()
                .or_else(|| self.file_sizes.get(p).copied())
                .unwrap_or(0),
            None => self.total_scanned,
        }
    }

    pub fn has_dir(&self, path: &Path) -> bool {
        self.dir_totals.contains_key(path)
    }

    pub fn has_file(&self, path: &Path) -> bool {
        self.file_sizes.contains_key(path)
    }
}

pub struct BrowseIndexBuilder {
    filter: Option<PathBuf>,
    total_scanned: u64,
    root_totals: HashMap<PathBuf, u64>,
    dir_totals: HashMap<PathBuf, u64>,
    file_sizes: HashMap<PathBuf, u64>,
    dirs: HashSet<PathBuf>,
}

impl BrowseIndexBuilder {
    pub fn new(filter: Option<PathBuf>, roots: Vec<PathBuf>) -> Self {
        let mut root_totals = HashMap::new();
        for root in roots {
            root_totals.insert(root, 0);
        }
        Self {
            filter,
            total_scanned: 0,
            root_totals,
            dir_totals: HashMap::new(),
            file_sizes: HashMap::new(),
            dirs: HashSet::new(),
        }
    }

    pub fn finalize(mut self) -> BrowseIndex {
        for (root, size) in &self.root_totals {
            self.dir_totals.entry(root.clone()).or_insert(*size);
            self.dirs.insert(root.clone());
        }
        if let Some(filter) = &self.filter {
            self.dir_totals.entry(filter.clone()).or_insert(0);
            self.dirs.insert(filter.clone());
        }

        let mut root_entries = self
            .root_totals
            .into_iter()
            .map(|(path, size)| BrowseEntry {
                path,
                size,
                is_dir: true,
            })
            .collect::<Vec<_>>();
        root_entries.sort_by(|a, b| {
            b.size
                .cmp(&a.size)
                .then_with(|| a.path.to_string_lossy().cmp(&b.path.to_string_lossy()))
        });
        let mut children: HashMap<PathBuf, Vec<BrowseEntry>> = HashMap::new();
        for dir in &self.dirs {
            if let Some(parent) = dir.parent() {
                if self.dir_totals.contains_key(parent) {
                    let size = self.dir_totals.get(dir).copied().unwrap_or(0);
                    children
                        .entry(parent.to_path_buf())
                        .or_default()
                        .push(BrowseEntry {
                            path: dir.clone(),
                            size,
                            is_dir: true,
                        });
                }
            }
        }
        for (path, size) in &self.file_sizes {
            if let Some(parent) = path.parent() {
                if self.dir_totals.contains_key(parent) {
                    children
                        .entry(parent.to_path_buf())
                        .or_default()
                        .push(BrowseEntry {
                            path: path.clone(),
                            size: *size,
                            is_dir: false,
                        });
                }
            }
        }
        for entries in children.values_mut() {
            entries.sort_by(|a, b| {
                b.size
                    .cmp(&a.size)
                    .then_with(|| a.path.to_string_lossy().cmp(&b.path.to_string_lossy()))
            });
        }

        BrowseIndex {
            total_scanned: self.total_scanned,
            root_entries,
            dir_totals: self.dir_totals,
            file_sizes: self.file_sizes,
            children,
        }
    }

    fn ingest_file(&mut self, root_path: &Path, file_path: &Path, size: u64) {
        let limit = self
            .filter
            .as_deref()
            .map(|f| {
                if f.starts_with(root_path) {
                    f
                } else {
                    root_path
                }
            })
            .unwrap_or(root_path);

        if let Some(filter) = &self.filter {
            if !file_path.starts_with(filter) {
                return;
            }
        }

        self.total_scanned += size;
        *self.root_totals.entry(root_path.to_path_buf()).or_insert(0) += size;
        self.file_sizes.insert(file_path.to_path_buf(), size);

        let mut current = file_path.parent();
        while let Some(dir) = current {
            if !dir.starts_with(limit) {
                break;
            }
            *self.dir_totals.entry(dir.to_path_buf()).or_insert(0) += size;
            self.dirs.insert(dir.to_path_buf());
            if dir == limit {
                break;
            }
            current = dir.parent();
        }
    }
}

pub struct Analyzer {
    filter: Option<PathBuf>,
    top_dir_limit: usize,
    total_scanned: u64,
    root_totals: HashMap<PathBuf, u64>,
    dir_sizes: HashMap<PathBuf, u64>,
    top_files: TopN,
}

impl Analyzer {
    pub fn new(filter: Option<PathBuf>, top_dirs: usize, top_files: usize) -> Self {
        Self {
            filter,
            top_dir_limit: top_dirs,
            total_scanned: 0,
            root_totals: HashMap::new(),
            dir_sizes: HashMap::new(),
            top_files: TopN::new(top_files),
        }
    }

    pub fn finalize(self) -> AnalysisResult {
        let mut dir_top = TopN::new(self.top_dir_limit);
        for (path, size) in self.dir_sizes {
            dir_top.push(path.to_string_lossy().to_string(), size);
        }
        let mut root_entries = self
            .root_totals
            .into_iter()
            .map(|(path, size)| UsageEntry {
                path: path.to_string_lossy().to_string(),
                size,
            })
            .collect::<Vec<_>>();
        root_entries.sort_by(|a, b| b.size.cmp(&a.size));
        AnalysisResult {
            total_scanned: self.total_scanned,
            roots: root_entries,
            top_dirs: dir_top.into_sorted(),
            top_files: self.top_files.into_sorted(),
        }
    }

    fn ingest_file(&mut self, root_path: &Path, file_path: &Path, size: u64) {
        let limit = self
            .filter
            .as_deref()
            .map(|f| {
                if f.starts_with(root_path) {
                    f
                } else {
                    root_path
                }
            })
            .unwrap_or(root_path);

        if let Some(filter) = &self.filter {
            if !file_path.starts_with(filter) {
                return;
            }
        }

        self.total_scanned += size;
        *self.root_totals.entry(root_path.to_path_buf()).or_insert(0) += size;
        self.top_files
            .push(file_path.to_string_lossy().to_string(), size);

        let mut current = file_path.parent();
        while let Some(dir) = current {
            if !dir.starts_with(limit) {
                break;
            }
            *self.dir_sizes.entry(dir.to_path_buf()).or_insert(0) += size;
            if dir == limit {
                break;
            }
            current = dir.parent();
        }
    }
}

impl ScanObserver for Analyzer {
    fn on_file_scanned(&mut self, root_path: &str, file: &ScannedFile) {
        if file.is_dir {
            return;
        }
        let size = file.size.max(0) as u64;
        if size == 0 {
            return;
        }
        let root_path = Path::new(root_path);
        let file_path = Path::new(&file.abs_path);
        self.ingest_file(root_path, file_path, size);
    }
}

impl ScanObserver for BrowseIndexBuilder {
    fn on_file_scanned(&mut self, root_path: &str, file: &ScannedFile) {
        if file.is_dir {
            return;
        }
        let size = file.size.max(0) as u64;
        if size == 0 {
            return;
        }
        let root_path = Path::new(root_path);
        let file_path = Path::new(&file.abs_path);
        self.ingest_file(root_path, file_path, size);
    }
}

pub fn analyze_store_with_progress(
    store: &crate::store::Store,
    filter: Option<PathBuf>,
    top_dirs: usize,
    top_files: usize,
    mut progress: Option<&mut dyn FnMut(usize)>,
) -> AnalysisResult {
    let mut analyzer = Analyzer::new(filter, top_dirs, top_files);
    let mut roots = HashMap::new();
    for root in &store.data.roots {
        roots.insert(root.id, PathBuf::from(&root.path));
    }
    let mut processed = 0usize;
    for file in &store.data.files {
        if file.status != "active" || file.is_dir {
            continue;
        }
        let root_path = match roots.get(&file.root_id) {
            Some(p) => p,
            None => continue,
        };
        let size = file.size.max(0) as u64;
        if size == 0 {
            continue;
        }
        let file_path = Path::new(&file.abs_path);
        analyzer.ingest_file(root_path, file_path, size);
        processed += 1;
        if processed % 50_000 == 0 {
            if let Some(cb) = progress.as_deref_mut() {
                cb(processed);
            }
        }
    }
    if let Some(cb) = progress.as_deref_mut() {
        cb(processed);
    }
    analyzer.finalize()
}

pub fn browse_index_from_store_with_progress(
    store: &crate::store::Store,
    filter: Option<PathBuf>,
    mut progress: Option<&mut dyn FnMut(usize)>,
) -> BrowseIndex {
    let roots = store
        .data
        .roots
        .iter()
        .map(|root| PathBuf::from(&root.path))
        .collect::<Vec<_>>();
    let mut builder = BrowseIndexBuilder::new(filter, roots);
    let mut roots_by_id = HashMap::new();
    for root in &store.data.roots {
        roots_by_id.insert(root.id, PathBuf::from(&root.path));
    }
    let mut processed = 0usize;
    for file in &store.data.files {
        if file.status != "active" || file.is_dir {
            continue;
        }
        let root_path = match roots_by_id.get(&file.root_id) {
            Some(p) => p,
            None => continue,
        };
        let size = file.size.max(0) as u64;
        if size == 0 {
            continue;
        }
        let file_path = Path::new(&file.abs_path);
        builder.ingest_file(root_path, file_path, size);
        processed += 1;
        if processed % 50_000 == 0 {
            if let Some(cb) = progress.as_deref_mut() {
                cb(processed);
            }
        }
    }
    if let Some(cb) = progress.as_deref_mut() {
        cb(processed);
    }
    builder.finalize()
}

pub fn print_report(result: &AnalysisResult, json: bool) -> Result<()> {
    if json {
        let out = serde_json::to_string_pretty(result)?;
        println!("{}", out);
        return Ok(());
    }

    println!("Total scanned: {}", human_size(result.total_scanned));
    println!("\nRoots:");
    if result.roots.is_empty() {
        println!("  (none)");
    } else {
        for (idx, entry) in result.roots.iter().enumerate() {
            println!(
                "  {}. {}  {}",
                idx + 1,
                entry.path,
                human_size(entry.size)
            );
        }
    }
    println!("\nTop folders:");
    if result.top_dirs.is_empty() {
        println!("  (none)");
    } else {
        for (idx, entry) in result.top_dirs.iter().enumerate() {
            println!(
                "  {}. {}  {}",
                idx + 1,
                entry.path,
                human_size(entry.size)
            );
        }
    }

    println!("\nTop files:");
    if result.top_files.is_empty() {
        println!("  (none)");
    } else {
        for (idx, entry) in result.top_files.iter().enumerate() {
            println!(
                "  {}. {}  {}",
                idx + 1,
                entry.path,
                human_size(entry.size)
            );
        }
    }

    Ok(())
}

pub(crate) fn human_size(bytes: u64) -> String {
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

struct TopN {
    limit: usize,
    heap: BinaryHeap<(Reverse<u64>, String)>,
}

impl TopN {
    fn new(limit: usize) -> Self {
        Self {
            limit,
            heap: BinaryHeap::new(),
        }
    }

    fn push(&mut self, path: String, size: u64) {
        if self.limit == 0 {
            return;
        }
        if self.heap.len() < self.limit {
            self.heap.push((Reverse(size), path));
            return;
        }
        if let Some((Reverse(min), _)) = self.heap.peek() {
            if size > *min {
                self.heap.pop();
                self.heap.push((Reverse(size), path));
            }
        }
    }

    fn into_sorted(self) -> Vec<UsageEntry> {
        let mut items = self
            .heap
            .into_iter()
            .map(|(Reverse(size), path)| UsageEntry { path, size })
            .collect::<Vec<_>>();
        items.sort_by(|a, b| b.size.cmp(&a.size));
        items
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::{FileEntry, RootEntry, Store, StoreData};
    use chrono::Utc;
    use std::path::PathBuf;

    #[test]
    fn analyzer_top_n_and_totals() {
        let mut analyzer = Analyzer::new(None, 2, 2);
        let files = vec![
            ScannedFile {
                rel_path: "a.txt".to_string(),
                abs_path: "/root/a.txt".to_string(),
                is_dir: false,
                is_symlink: false,
                size: 100,
                mtime: 0,
                ext: Some("txt".to_string()),
            },
            ScannedFile {
                rel_path: "b.txt".to_string(),
                abs_path: "/root/sub/b.txt".to_string(),
                is_dir: false,
                is_symlink: false,
                size: 300,
                mtime: 0,
                ext: Some("txt".to_string()),
            },
            ScannedFile {
                rel_path: "c.txt".to_string(),
                abs_path: "/root/sub/deep/c.txt".to_string(),
                is_dir: false,
                is_symlink: false,
                size: 200,
                mtime: 0,
                ext: Some("txt".to_string()),
            },
        ];
        for file in &files {
            analyzer.on_file_scanned("/root", file);
        }
        let result = analyzer.finalize();
        assert_eq!(result.total_scanned, 600);
        assert_eq!(result.roots.len(), 1);
        assert_eq!(result.roots[0].path, "/root");
        assert_eq!(result.top_files.len(), 2);
        assert_eq!(result.top_files[0].size, 300);
        assert_eq!(result.top_files[1].size, 200);
        assert!(!result.top_dirs.is_empty());
    }

    #[test]
    fn analyze_store_uses_existing_index() {
        let mut store = Store {
            path: PathBuf::from("/tmp/catalog.bin"),
            data: StoreData::new(),
        };
        store.data.roots.push(RootEntry {
            id: 1,
            path: "/root".to_string(),
            added_at: Utc::now().to_rfc3339(),
            preset_name: None,
            last_indexed_at: Some(Utc::now().to_rfc3339()),
            one_filesystem: true,
        });
        store.data.files.push(FileEntry {
            id: 1,
            root_id: 1,
            rel_path: "big.bin".to_string(),
            abs_path: "/root/big.bin".to_string(),
            is_dir: false,
            is_symlink: false,
            size: 1024,
            mtime: 0,
            ext: Some("bin".to_string()),
            status: "active".to_string(),
            last_seen_run: 1,
        });
        let result = analyze_store_with_progress(&store, None, 5, 5, None);
        assert_eq!(result.total_scanned, 1024);
        assert_eq!(result.roots.len(), 1);
        assert_eq!(result.roots[0].path, "/root");
        assert_eq!(result.top_files.len(), 1);
        assert_eq!(result.top_files[0].path, "/root/big.bin");
    }

    #[test]
    fn analyze_store_respects_filter() {
        let mut store = Store {
            path: PathBuf::from("/tmp/catalog.bin"),
            data: StoreData::new(),
        };
        store.data.roots.push(RootEntry {
            id: 1,
            path: "/root".to_string(),
            added_at: Utc::now().to_rfc3339(),
            preset_name: None,
            last_indexed_at: Some(Utc::now().to_rfc3339()),
            one_filesystem: true,
        });
        store.data.files.push(FileEntry {
            id: 1,
            root_id: 1,
            rel_path: "keep/big.bin".to_string(),
            abs_path: "/root/keep/big.bin".to_string(),
            is_dir: false,
            is_symlink: false,
            size: 2048,
            mtime: 0,
            ext: Some("bin".to_string()),
            status: "active".to_string(),
            last_seen_run: 1,
        });
        store.data.files.push(FileEntry {
            id: 2,
            root_id: 1,
            rel_path: "drop/small.bin".to_string(),
            abs_path: "/root/drop/small.bin".to_string(),
            is_dir: false,
            is_symlink: false,
            size: 128,
            mtime: 0,
            ext: Some("bin".to_string()),
            status: "active".to_string(),
            last_seen_run: 1,
        });

        let result = analyze_store_with_progress(&store, Some(PathBuf::from("/root/keep")), 5, 5, None);
        assert_eq!(result.total_scanned, 2048);
        assert_eq!(result.roots.len(), 1);
        assert_eq!(result.roots[0].path, "/root");
        assert_eq!(result.top_files.len(), 1);
        assert_eq!(result.top_files[0].path, "/root/keep/big.bin");
    }
}
