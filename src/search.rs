use crate::config::Config;
use crate::util::{normalize_path_allow_missing, path_to_string};
use anyhow::{Context, Result};
use chrono::{Local, NaiveDate, TimeZone};
use rusqlite::{params_from_iter, types::Value, Connection};

#[derive(Debug, serde::Serialize)]
pub struct SearchEntry {
    pub id: i64,
    pub path: String,
    pub mtime: i64,
    pub size: i64,
    pub is_dir: bool,
    pub is_symlink: bool,
    pub ext: Option<String>,
    pub root: String,
    pub status: String,
}

pub fn search(
    conn: &Connection,
    _cfg: &Config,
    query: &str,
    ext: Option<&str>,
    tags: &[String],
    after: Option<&str>,
    before: Option<&str>,
    min_size: Option<u64>,
    max_size: Option<u64>,
    root: Option<&str>,
) -> Result<Vec<SearchEntry>> {
    let mut sql = String::from(
        "SELECT f.id, f.abs_path, f.mtime, f.size, f.is_dir, f.is_symlink, f.ext, r.path, f.status \
         FROM files f \
         JOIN roots r ON f.root_id = r.id \
         WHERE f.status = 'active' AND f.abs_path LIKE ?1 COLLATE NOCASE",
    );
    let mut params: Vec<Value> = Vec::new();
    params.push(Value::from(format!("%{}%", query)));

    if let Some(exts) = ext {
        let list: Vec<String> = exts
            .split(',')
            .map(|s| s.trim().to_lowercase())
            .filter(|s| !s.is_empty())
            .collect();
        if !list.is_empty() {
            sql.push_str(" AND f.ext IN (");
            append_placeholders(&mut sql, list.len(), params.len());
            sql.push(')');
            for e in list {
                params.push(Value::from(e));
            }
        }
    }

    if let Some(after) = after {
        let ts = parse_date_start(after)?;
        sql.push_str(" AND f.mtime >= ");
        append_placeholder(&mut sql, params.len());
        params.push(Value::from(ts));
    }
    if let Some(before) = before {
        let ts = parse_date_end_exclusive(before)?;
        sql.push_str(" AND f.mtime < ");
        append_placeholder(&mut sql, params.len());
        params.push(Value::from(ts));
    }
    if let Some(min) = min_size {
        sql.push_str(" AND f.size >= ");
        append_placeholder(&mut sql, params.len());
        params.push(Value::from(min as i64));
    }
    if let Some(max) = max_size {
        sql.push_str(" AND f.size <= ");
        append_placeholder(&mut sql, params.len());
        params.push(Value::from(max as i64));
    }
    if let Some(root) = root {
        let normalized = normalize_path_allow_missing(root)?;
        let root_str = path_to_string(&normalized);
        sql.push_str(" AND r.path = ");
        append_placeholder(&mut sql, params.len());
        params.push(Value::from(root_str));
    }
    if !tags.is_empty() {
        let mut tag_list = Vec::new();
        for t in tags {
            for part in t.split(',') {
                let s = part.trim().to_lowercase();
                if !s.is_empty() {
                    tag_list.push(s);
                }
            }
        }
        if !tag_list.is_empty() {
            sql.push_str(
                " AND EXISTS (SELECT 1 FROM file_tags ft \
                 JOIN tags t ON t.id = ft.tag_id \
                 WHERE ft.file_id = f.id AND t.name IN (",
            );
            append_placeholders(&mut sql, tag_list.len(), params.len());
            sql.push_str("))");
            for t in tag_list {
                params.push(Value::from(t));
            }
        }
    }

    sql.push_str(" ORDER BY f.mtime DESC");

    let mut stmt = conn.prepare(&sql)?;
    let mut rows = stmt.query(params_from_iter(params))?;
    let mut out = Vec::new();
    while let Some(row) = rows.next()? {
        out.push(SearchEntry {
            id: row.get(0)?,
            path: row.get(1)?,
            mtime: row.get(2)?,
            size: row.get(3)?,
            is_dir: row.get::<_, i64>(4)? != 0,
            is_symlink: row.get::<_, i64>(5)? != 0,
            ext: row.get(6)?,
            root: row.get(7)?,
            status: row.get(8)?,
        });
    }
    Ok(out)
}

pub fn recent(
    conn: &Connection,
    _cfg: &Config,
    days: Option<u32>,
    limit: Option<u32>,
) -> Result<Vec<SearchEntry>> {
    let days = days.unwrap_or(7) as i64;
    let limit = limit.unwrap_or(50) as i64;
    let now = Local::now().timestamp();
    let threshold = now - (days * 86400);

    let sql = "SELECT f.id, f.abs_path, f.mtime, f.size, f.is_dir, f.is_symlink, f.ext, r.path, f.status \
        FROM files f \
        JOIN roots r ON f.root_id = r.id \
        WHERE f.status='active' AND f.mtime >= ?1 \
        ORDER BY f.mtime DESC \
        LIMIT ?2";

    let mut stmt = conn.prepare(sql)?;
    let mut rows = stmt.query(rusqlite::params![threshold, limit])?;
    let mut out = Vec::new();
    while let Some(row) = rows.next()? {
        out.push(SearchEntry {
            id: row.get(0)?,
            path: row.get(1)?,
            mtime: row.get(2)?,
            size: row.get(3)?,
            is_dir: row.get::<_, i64>(4)? != 0,
            is_symlink: row.get::<_, i64>(5)? != 0,
            ext: row.get(6)?,
            root: row.get(7)?,
            status: row.get(8)?,
        });
    }
    Ok(out)
}

fn append_placeholders(sql: &mut String, count: usize, start_index: usize) {
    for i in 0..count {
        if i > 0 {
            sql.push_str(", ");
        }
        append_placeholder(sql, start_index + i);
    }
}

fn append_placeholder(sql: &mut String, index: usize) {
    let idx = index + 1;
    sql.push('?');
    sql.push_str(&idx.to_string());
}

fn parse_date_start(date: &str) -> Result<i64> {
    let d = NaiveDate::parse_from_str(date, "%Y-%m-%d")
        .with_context(|| "invalid date, expected YYYY-MM-DD")?;
    Ok(Local
        .from_local_datetime(&d.and_hms_opt(0, 0, 0).unwrap())
        .single()
        .unwrap()
        .timestamp())
}

fn parse_date_end_exclusive(date: &str) -> Result<i64> {
    let d = NaiveDate::parse_from_str(date, "%Y-%m-%d")
        .with_context(|| "invalid date, expected YYYY-MM-DD")?;
    let next = d.succ_opt().unwrap_or(d);
    Ok(Local
        .from_local_datetime(&next.and_hms_opt(0, 0, 0).unwrap())
        .single()
        .unwrap()
        .timestamp())
}
