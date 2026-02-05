use anyhow::{Context, Result};
use rusqlite::{Connection, OptionalExtension};
use std::path::Path;

pub fn connect(path: &Path) -> Result<Connection> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("failed to create db dir: {}", parent.display()))?;
    }
    let conn = Connection::open(path)
        .with_context(|| format!("failed to open db: {}", path.display()))?;
    conn.pragma_update(None, "journal_mode", "WAL")?;
    conn.pragma_update(None, "synchronous", "NORMAL")?;
    conn.pragma_update(None, "foreign_keys", "ON")?;
    Ok(conn)
}

pub fn migrate(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS schema_migrations (version INTEGER NOT NULL);",
    )?;

    let version: Option<i64> = conn
        .query_row("SELECT version FROM schema_migrations LIMIT 1;", [], |row| {
            row.get(0)
        })
        .optional()?;

    match version {
        None => {
            apply_schema_v1(conn)?;
            conn.execute("INSERT INTO schema_migrations (version) VALUES (1);", [])?;
        }
        Some(1) => {}
        Some(v) => anyhow::bail!("unsupported schema version {}", v),
    }

    Ok(())
}

fn apply_schema_v1(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "\
        CREATE TABLE IF NOT EXISTS roots (\
          id INTEGER PRIMARY KEY,\
          path TEXT NOT NULL UNIQUE,\
          added_at TEXT NOT NULL,\
          preset_name TEXT,\
          last_indexed_at TEXT,\
          one_filesystem INTEGER NOT NULL DEFAULT 1\
        );\
        \
        CREATE TABLE IF NOT EXISTS index_runs (\
          id INTEGER PRIMARY KEY,\
          started_at TEXT NOT NULL,\
          finished_at TEXT\
        );\
        \
        CREATE TABLE IF NOT EXISTS files (\
          id INTEGER PRIMARY KEY,\
          root_id INTEGER NOT NULL,\
          rel_path TEXT NOT NULL,\
          abs_path TEXT NOT NULL,\
          is_dir INTEGER NOT NULL,\
          is_symlink INTEGER NOT NULL,\
          size INTEGER NOT NULL,\
          mtime INTEGER NOT NULL,\
          ext TEXT,\
          status TEXT NOT NULL,\
          last_seen_run INTEGER NOT NULL,\
          FOREIGN KEY(root_id) REFERENCES roots(id)\
        );\
        \
        CREATE TABLE IF NOT EXISTS tags (\
          id INTEGER PRIMARY KEY,\
          name TEXT NOT NULL UNIQUE\
        );\
        \
        CREATE TABLE IF NOT EXISTS file_tags (\
          file_id INTEGER NOT NULL,\
          tag_id INTEGER NOT NULL,\
          UNIQUE(file_id, tag_id),\
          FOREIGN KEY(file_id) REFERENCES files(id),\
          FOREIGN KEY(tag_id) REFERENCES tags(id)\
        );\
        \
        CREATE UNIQUE INDEX IF NOT EXISTS idx_files_root_rel ON files(root_id, rel_path);\
        CREATE INDEX IF NOT EXISTS idx_files_status ON files(status);\
        CREATE INDEX IF NOT EXISTS idx_files_mtime ON files(mtime);\
        CREATE INDEX IF NOT EXISTS idx_files_ext ON files(ext);\
        CREATE INDEX IF NOT EXISTS idx_files_path_nocase ON files(abs_path COLLATE NOCASE);\
        CREATE INDEX IF NOT EXISTS idx_file_tags_tag ON file_tags(tag_id);\
        CREATE INDEX IF NOT EXISTS idx_file_tags_file ON file_tags(file_id);\
        ",
    )?;
    Ok(())
}
