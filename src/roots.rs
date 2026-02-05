use crate::config::Config;
use crate::store::{RootEntry, StoreData};
use crate::util::{normalize_path, path_to_string};
use anyhow::Result;
use chrono::Local;
use std::collections::{HashMap, HashSet};

pub fn add_roots(cfg: &mut Config, paths: &[String]) -> Result<usize> {
    let mut added = 0;
    let mut existing: HashSet<String> = cfg.roots.iter().cloned().collect();
    for p in paths {
        let normalized = match normalize_path(p) {
            Ok(path) => path,
            Err(err) => {
                tracing::warn!("skip missing path {} ({})", p, err);
                continue;
            }
        };
        let s = path_to_string(&normalized);
        if existing.insert(s.clone()) {
            cfg.roots.push(s);
            added += 1;
        }
    }
    Ok(added)
}

pub fn remove_roots(cfg: &mut Config, paths: &[String]) -> Result<usize> {
    let mut to_remove = HashSet::new();
    for p in paths {
        let normalized = crate::util::normalize_path_allow_missing(p)?;
        to_remove.insert(path_to_string(&normalized));
    }
    let before = cfg.roots.len();
    cfg.roots.retain(|p| !to_remove.contains(p));
    Ok(before - cfg.roots.len())
}

pub fn sync_roots(store: &mut StoreData, cfg: &Config, preset_name: Option<String>) -> Result<()> {
    let mut existing: HashMap<String, i64> = HashMap::new();
    for root in &store.roots {
        existing.insert(root.path.clone(), root.id);
    }

    let now = Local::now().to_rfc3339();
    for root in &cfg.roots {
        if let Some(id) = existing.get(root) {
            if let Some(entry) = store.roots.iter_mut().find(|r| r.id == *id) {
                entry.one_filesystem = cfg.one_filesystem;
            }
        } else {
            let id = store.next_root_id();
            store.roots.push(RootEntry {
                id,
                path: root.to_string(),
                added_at: now.clone(),
                preset_name: preset_name.clone(),
                last_indexed_at: None,
                one_filesystem: cfg.one_filesystem,
            });
        }
    }

    let desired: HashSet<&String> = cfg.roots.iter().collect();
    let removed_root_ids: HashSet<i64> = store
        .roots
        .iter()
        .filter(|r| !desired.contains(&r.path))
        .map(|r| r.id)
        .collect();

    if !removed_root_ids.is_empty() {
        let removed_file_ids: HashSet<i64> = store
            .files
            .iter()
            .filter(|f| removed_root_ids.contains(&f.root_id))
            .map(|f| f.id)
            .collect();

        store.roots.retain(|r| !removed_root_ids.contains(&r.id));
        store.files.retain(|f| !removed_root_ids.contains(&f.root_id));
        store
            .file_tags
            .retain(|ft| !removed_file_ids.contains(&ft.file_id));

        prune_orphan_tags(store);
    }

    Ok(())
}

pub fn print_roots(store: &StoreData, cfg: &Config) -> Result<()> {
    println!("Roots:");
    for root in &cfg.roots {
        let last_indexed = store
            .roots
            .iter()
            .find(|r| r.path == *root)
            .and_then(|r| r.last_indexed_at.clone());
        match last_indexed {
            Some(ts) => println!("  {} (last indexed {})", root, ts),
            None => println!("  {} (never indexed)", root),
        }
    }

    println!("\nExcludes:");
    for ex in &cfg.excludes {
        println!("  {}", ex);
    }
    println!("\ninclude_hidden: {}", cfg.include_hidden);
    println!("one_filesystem: {}", cfg.one_filesystem);
    Ok(())
}

fn prune_orphan_tags(store: &mut StoreData) {
    let mut used = HashSet::new();
    for ft in &store.file_tags {
        used.insert(ft.tag_id);
    }
    store.tags.retain(|t| used.contains(&t.id));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{Config, OutputMode};

    #[test]
    fn sync_roots_prunes_removed_root_data() {
        let mut store = StoreData::new();
        store.roots.push(RootEntry {
            id: 1,
            path: "/tmp/root-a".to_string(),
            added_at: "now".to_string(),
            preset_name: None,
            last_indexed_at: None,
            one_filesystem: true,
        });
        store.roots.push(RootEntry {
            id: 2,
            path: "/tmp/root-b".to_string(),
            added_at: "now".to_string(),
            preset_name: None,
            last_indexed_at: None,
            one_filesystem: true,
        });
        store.files.push(crate::store::FileEntry {
            id: 10,
            root_id: 1,
            rel_path: "keep.txt".to_string(),
            abs_path: "/tmp/root-a/keep.txt".to_string(),
            is_dir: false,
            is_symlink: false,
            size: 10,
            mtime: 1,
            ext: Some("txt".to_string()),
            status: "active".to_string(),
            last_seen_run: 1,
        });
        store.files.push(crate::store::FileEntry {
            id: 11,
            root_id: 2,
            rel_path: "drop.txt".to_string(),
            abs_path: "/tmp/root-b/drop.txt".to_string(),
            is_dir: false,
            is_symlink: false,
            size: 10,
            mtime: 1,
            ext: Some("txt".to_string()),
            status: "active".to_string(),
            last_seen_run: 1,
        });
        store.tags.push(crate::store::TagEntry {
            id: 1,
            name: "keep".to_string(),
        });
        store.tags.push(crate::store::TagEntry {
            id: 2,
            name: "drop".to_string(),
        });
        store.file_tags.push(crate::store::FileTagEntry {
            file_id: 10,
            tag_id: 1,
        });
        store.file_tags.push(crate::store::FileTagEntry {
            file_id: 11,
            tag_id: 2,
        });

        let cfg = Config {
            version: 1,
            output: OutputMode::Plain,
            include_hidden: false,
            one_filesystem: true,
            roots: vec!["/tmp/root-a".to_string()],
            excludes: vec![],
        };

        sync_roots(&mut store, &cfg, None).unwrap();

        assert_eq!(store.roots.len(), 1);
        assert_eq!(store.roots[0].path, "/tmp/root-a");
        assert_eq!(store.files.len(), 1);
        assert_eq!(store.files[0].abs_path, "/tmp/root-a/keep.txt");
        assert_eq!(store.file_tags.len(), 1);
        assert_eq!(store.file_tags[0].file_id, 10);
        assert_eq!(store.tags.len(), 1);
        assert_eq!(store.tags[0].name, "keep");
    }
}
