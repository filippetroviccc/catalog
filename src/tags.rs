use crate::util::{normalize_path_allow_missing, path_to_string};
use anyhow::{Context, Result};
use rusqlite::{Connection, OptionalExtension};

pub fn add_tag(conn: &Connection, target: &str, tag: &str) -> Result<()> {
    let file_id = resolve_file_id(conn, target)?;
    let tag = tag.trim().to_lowercase();
    if tag.is_empty() {
        anyhow::bail!("tag cannot be empty");
    }
    conn.execute("INSERT OR IGNORE INTO tags (name) VALUES (?1)", rusqlite::params![tag])?;
    let tag_id: i64 = conn.query_row(
        "SELECT id FROM tags WHERE name = ?1",
        rusqlite::params![tag],
        |row| row.get(0),
    )?;
    conn.execute(
        "INSERT OR IGNORE INTO file_tags (file_id, tag_id) VALUES (?1, ?2)",
        rusqlite::params![file_id, tag_id],
    )?;
    Ok(())
}

pub fn remove_tag(conn: &Connection, target: &str, tag: &str) -> Result<()> {
    let file_id = resolve_file_id(conn, target)?;
    let tag = tag.trim().to_lowercase();
    if tag.is_empty() {
        anyhow::bail!("tag cannot be empty");
    }
    if let Ok(tag_id) = conn.query_row(
        "SELECT id FROM tags WHERE name = ?1",
        rusqlite::params![tag],
        |row| row.get(0),
    ) {
        conn.execute(
            "DELETE FROM file_tags WHERE file_id = ?1 AND tag_id = ?2",
            rusqlite::params![file_id, tag_id],
        )?;
        conn.execute(
            "DELETE FROM tags WHERE id = ?1 AND NOT EXISTS (SELECT 1 FROM file_tags WHERE tag_id = ?1)",
            rusqlite::params![tag_id],
        )?;
    }
    Ok(())
}

pub fn list_tags(conn: &Connection) -> Result<()> {
    let mut stmt = conn.prepare(
        "SELECT t.name, COUNT(ft.file_id) \
         FROM tags t \
         LEFT JOIN file_tags ft ON t.id = ft.tag_id \
         GROUP BY t.id \
         ORDER BY t.name",
    )?;
    let rows = stmt.query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?)))?;
    for row in rows {
        let (name, count) = row?;
        println!("{}  {}", name, count);
    }
    Ok(())
}

fn resolve_file_id(conn: &Connection, target: &str) -> Result<i64> {
    if let Ok(id) = target.parse::<i64>() {
        let exists: Option<i64> = conn
            .query_row(
                "SELECT id FROM files WHERE id = ?1",
                rusqlite::params![id],
                |row| row.get(0),
            )
            .optional()?;
        if exists.is_some() {
            return Ok(id);
        }
    }

    let normalized = normalize_path_allow_missing(target)?;
    let path = path_to_string(&normalized);
    let file_id: Option<i64> = conn
        .query_row(
            "SELECT id FROM files WHERE abs_path = ?1",
            rusqlite::params![path],
            |row| row.get(0),
        )
        .optional()
        .with_context(|| "failed to resolve file id")?;
    file_id.context("file not found")
}
