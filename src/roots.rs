use crate::config::Config;
use crate::util::{normalize_path, path_to_string};
use anyhow::{Context, Result};
use chrono::Local;
use rusqlite::{Connection, OptionalExtension};
use std::collections::HashSet;

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

pub fn sync_roots(conn: &Connection, cfg: &Config, preset_name: Option<String>) -> Result<()> {
    let mut stmt = conn.prepare("SELECT id, path FROM roots")?;
    let rows = stmt.query_map([], |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?)))?;
    let mut existing = HashSet::new();
    let mut existing_map = Vec::new();
    for row in rows {
        let (id, path) = row?;
        existing.insert(path.clone());
        existing_map.push((id, path));
    }

    let now = Local::now().to_rfc3339();
    for root in &cfg.roots {
        if !existing.contains(root) {
            conn.execute(
                "INSERT INTO roots (path, added_at, preset_name, one_filesystem) VALUES (?1, ?2, ?3, ?4)",
                rusqlite::params![root, now, preset_name.as_deref(), cfg.one_filesystem as i32],
            )?;
        } else {
            conn.execute(
                "UPDATE roots SET one_filesystem = ?1 WHERE path = ?2",
                rusqlite::params![cfg.one_filesystem as i32, root],
            )?;
        }
    }

    let desired: HashSet<&String> = cfg.roots.iter().collect();
    for (id, path) in existing_map {
        if !desired.contains(&path) {
            conn.execute("DELETE FROM files WHERE root_id = ?1", rusqlite::params![id])?;
            conn.execute("DELETE FROM roots WHERE id = ?1", rusqlite::params![id])?;
        }
    }
    Ok(())
}

pub fn print_roots(conn: &Connection, cfg: &Config) -> Result<()> {
    println!("Roots:");
    for root in &cfg.roots {
        let last_indexed: Option<String> = conn
            .query_row(
                "SELECT last_indexed_at FROM roots WHERE path = ?1",
                rusqlite::params![root],
                |row| row.get(0),
            )
            .optional()
            .context("failed to read root metadata")?;
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
