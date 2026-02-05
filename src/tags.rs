use crate::store::StoreData;
use crate::util::{normalize_path_allow_missing, path_to_string};
use anyhow::{Context, Result};
use std::collections::HashMap;

pub fn add_tag(store: &mut StoreData, target: &str, tag: &str) -> Result<()> {
    let file_id = resolve_file_id(store, target)?;
    let tag = tag.trim().to_lowercase();
    if tag.is_empty() {
        anyhow::bail!("tag cannot be empty");
    }
    let tag_id = match store.tags.iter().find(|t| t.name == tag) {
        Some(t) => t.id,
        None => {
            let id = store.next_tag_id();
            store.tags.push(crate::store::TagEntry { id, name: tag });
            id
        }
    };
    if !store
        .file_tags
        .iter()
        .any(|ft| ft.file_id == file_id && ft.tag_id == tag_id)
    {
        store
            .file_tags
            .push(crate::store::FileTagEntry { file_id, tag_id });
    }
    Ok(())
}

pub fn remove_tag(store: &mut StoreData, target: &str, tag: &str) -> Result<()> {
    let file_id = resolve_file_id(store, target)?;
    let tag = tag.trim().to_lowercase();
    if tag.is_empty() {
        anyhow::bail!("tag cannot be empty");
    }
    let tag_id = match store.tags.iter().find(|t| t.name == tag) {
        Some(t) => t.id,
        None => return Ok(()),
    };

    store
        .file_tags
        .retain(|ft| !(ft.file_id == file_id && ft.tag_id == tag_id));

    let still_used = store.file_tags.iter().any(|ft| ft.tag_id == tag_id);
    if !still_used {
        store.tags.retain(|t| t.id != tag_id);
    }
    Ok(())
}

pub fn list_tags(store: &StoreData) -> Result<()> {
    let mut counts: HashMap<i64, i64> = HashMap::new();
    for ft in &store.file_tags {
        *counts.entry(ft.tag_id).or_insert(0) += 1;
    }
    let mut entries = store
        .tags
        .iter()
        .map(|t| (t.name.clone(), *counts.get(&t.id).unwrap_or(&0)))
        .collect::<Vec<_>>();
    entries.sort_by(|a, b| a.0.cmp(&b.0));
    for (name, count) in entries {
        println!("{}  {}", name, count);
    }
    Ok(())
}

fn resolve_file_id(store: &StoreData, target: &str) -> Result<i64> {
    if let Ok(id) = target.parse::<i64>() {
        if store.files.iter().any(|f| f.id == id) {
            return Ok(id);
        }
    }

    let normalized = normalize_path_allow_missing(target)?;
    let path = path_to_string(&normalized);
    let file_id = store
        .files
        .iter()
        .find(|f| f.abs_path == path)
        .map(|f| f.id);
    file_id.context("file not found")
}
