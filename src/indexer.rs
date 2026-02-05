use crate::config::Config;
use crate::roots;
use crate::util::{path_to_string, normalize_path_allow_missing};
use anyhow::Result;
use chrono::Local;
use ignore::gitignore::{Gitignore, GitignoreBuilder};
use rusqlite::Connection;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};
use walkdir::{DirEntry, WalkDir};

pub struct IndexStats {
    pub seen: usize,
    pub updated: usize,
    pub deleted: usize,
    pub skipped: usize,
}

struct IgnoreMatcher {
    gitignore: Gitignore,
    abs_excludes: Vec<PathBuf>,
    include_hidden: bool,
}

pub fn run(
    conn: &Connection,
    cfg: &Config,
    full: bool,
    one_filesystem_override: bool,
) -> Result<IndexStats> {
    roots::sync_roots(conn, cfg, None)?;

    let start = Local::now().to_rfc3339();
    conn.execute("INSERT INTO index_runs (started_at) VALUES (?1)", rusqlite::params![start])?;
    let run_id: i64 = conn.last_insert_rowid();

    let mut total_seen = 0;
    let mut total_updated = 0;
    let mut total_deleted = 0;
    let mut total_skipped = 0;

    let roots = fetch_roots(conn)?;
    for root in roots {
        let matcher = build_matcher(cfg, &root.path)?;
        let mut stats = index_root(
            conn,
            &root.path,
            root.id,
            run_id,
            full,
            one_filesystem_override || root.one_filesystem,
            &matcher,
        )?;
        total_seen += stats.seen;
        total_updated += stats.updated;
        total_deleted += stats.deleted;
        total_skipped += stats.skipped;
    }

    let finish = Local::now().to_rfc3339();
    conn.execute(
        "UPDATE index_runs SET finished_at = ?1 WHERE id = ?2",
        rusqlite::params![finish, run_id],
    )?;

    Ok(IndexStats {
        seen: total_seen,
        updated: total_updated,
        deleted: total_deleted,
        skipped: total_skipped,
    })
}

struct RootRow {
    id: i64,
    path: String,
    one_filesystem: bool,
}

fn fetch_roots(conn: &Connection) -> Result<Vec<RootRow>> {
    let mut stmt = conn.prepare("SELECT id, path, one_filesystem FROM roots ORDER BY path")?;
    let rows = stmt.query_map([], |row| {
        Ok(RootRow {
            id: row.get(0)?,
            path: row.get(1)?,
            one_filesystem: row.get::<_, i64>(2)? != 0,
        })
    })?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row?);
    }
    Ok(out)
}

fn index_root(
    conn: &Connection,
    root: &str,
    root_id: i64,
    run_id: i64,
    full: bool,
    one_filesystem: bool,
    matcher: &IgnoreMatcher,
) -> Result<IndexStats> {
    let root_path = normalize_path_allow_missing(root)?;
    if !root_path.exists() {
        tracing::warn!("root missing: {}", root);
        return Ok(IndexStats {
            seen: 0,
            updated: 0,
            deleted: 0,
            skipped: 0,
        });
    }

    let tx = conn.transaction()?;

    if full {
        tx.execute(
            "UPDATE files SET status = 'deleted' WHERE root_id = ?1",
            rusqlite::params![root_id],
        )?;
    }

    let mut seen = 0;
    let mut updated = 0;
    let mut skipped = 0;
    let mut permission_skips = 0;

    let walker = WalkDir::new(&root_path)
        .follow_links(false)
        .same_file_system(one_filesystem)
        .into_iter()
        .filter_entry(|entry| !should_skip(entry, &root_path, matcher));

    for entry in walker {
        let entry = match entry {
            Ok(e) => e,
            Err(err) => {
                tracing::warn!("walk error: {}", err);
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

        tx.execute(
            "INSERT INTO files (root_id, rel_path, abs_path, is_dir, is_symlink, size, mtime, ext, status, last_seen_run)\
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, 'active', ?9)\
             ON CONFLICT(root_id, rel_path) DO UPDATE SET\
               abs_path=excluded.abs_path,\
               is_dir=excluded.is_dir,\
               is_symlink=excluded.is_symlink,\
               size=excluded.size,\
               mtime=excluded.mtime,\
               ext=excluded.ext,\
               status='active',\
               last_seen_run=excluded.last_seen_run",
            rusqlite::params![
                root_id,
                rel_path,
                abs_path,
                is_dir as i32,
                is_symlink as i32,
                size,
                mtime,
                ext,
                run_id
            ],
        )?;
        seen += 1;
        updated += 1;
    }

    let deleted = tx.execute(
        "UPDATE files SET status='deleted' WHERE root_id = ?1 AND last_seen_run != ?2",
        rusqlite::params![root_id, run_id],
    )?;

    let now = Local::now().to_rfc3339();
    tx.execute(
        "UPDATE roots SET last_indexed_at = ?1 WHERE id = ?2",
        rusqlite::params![now, root_id],
    )?;

    tx.commit()?;

    if permission_skips > 0 {
        tracing::warn!("skipped {} entries due to permissions", permission_skips);
    }

    Ok(IndexStats {
        seen,
        updated,
        deleted,
        skipped,
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
