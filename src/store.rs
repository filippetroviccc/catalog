use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};

const STORE_VERSION: u32 = 1;

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

#[derive(Debug)]
pub struct Store {
    pub path: PathBuf,
    pub data: StoreData,
}

impl Store {
    pub fn load(path: &Path) -> Result<Self> {
        if path.exists() {
            let raw = fs::read(path)
                .with_context(|| format!("failed to read store: {}", path.display()))?;
            let mut data: StoreData = match bincode::deserialize(&raw) {
                Ok(data) => data,
                Err(bin_err) => {
                    let text = std::str::from_utf8(&raw).map_err(|_| {
                        anyhow::anyhow!(
                            "failed to decode store as binary; also not valid utf-8 ({})",
                            bin_err
                        )
                    })?;
                    serde_json::from_str(text).context("failed to parse legacy store json")?
                }
            };
            data.ensure_counters();
            Ok(Self {
                path: path.to_path_buf(),
                data,
            })
        } else {
            if let Some(legacy) = legacy_json_path(path) {
                if legacy.exists() {
                    let raw = fs::read_to_string(&legacy).with_context(|| {
                        format!("failed to read legacy store: {}", legacy.display())
                    })?;
                    let mut data: StoreData = serde_json::from_str(&raw)
                        .context("failed to parse legacy store json")?;
                    data.ensure_counters();
                    return Ok(Self {
                        path: path.to_path_buf(),
                        data,
                    });
                }
            }
            Ok(Self {
                path: path.to_path_buf(),
                data: StoreData::new(),
            })
        }
    }

    pub fn init(path: &Path) -> Result<Self> {
        let store = Self::load(path)?;
        store.save()?;
        Ok(store)
    }

    pub fn save(&self) -> Result<()> {
        ensure_parent_dir(&self.path)?;
        let tmp_path = tmp_path(&self.path);
        let data = bincode::serialize(&self.data).context("failed to serialize store")?;
        let mut file = File::create(&tmp_path)
            .with_context(|| format!("failed to write store: {}", tmp_path.display()))?;
        file.write_all(&data)?;
        file.sync_all()?;
        fs::rename(&tmp_path, &self.path)
            .with_context(|| format!("failed to finalize store: {}", self.path.display()))?;
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
    if let Some(legacy) = legacy_json_path(path) {
        if legacy.exists() {
            fs::remove_file(&legacy)
                .with_context(|| format!("failed to remove legacy store: {}", legacy.display()))?;
            removed += 1;
        }
    }
    Ok(removed)
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

fn legacy_json_path(path: &Path) -> Option<PathBuf> {
    match path.extension().and_then(|ext| ext.to_str()) {
        Some("json") => None,
        _ => Some(path.with_extension("json")),
    }
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
            .join(format!("catalog_store_test_{}_{}_{}", prefix, std::process::id(), nanos));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn store_round_trip_preserves_data() {
        let dir = temp_dir("round_trip");
        let path = dir.join("store.json");

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
}
