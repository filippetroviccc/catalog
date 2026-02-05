## Database Schema and Migrations

This document defines the SQLite schema and migration strategy.

---

## Migration Strategy

- Use a `schema_migrations` table with a single integer version.
- Apply migrations sequentially on startup.
- Fail fast on unknown or missing migrations.

Example migration table:

```sql
CREATE TABLE IF NOT EXISTS schema_migrations (
  version INTEGER NOT NULL
);
```

---

## Base Schema (Version 1)

```sql
CREATE TABLE IF NOT EXISTS roots (
  id INTEGER PRIMARY KEY,
  path TEXT NOT NULL UNIQUE,
  added_at TEXT NOT NULL,
  preset_name TEXT,
  last_indexed_at TEXT,
  one_filesystem INTEGER NOT NULL DEFAULT 1
);

CREATE TABLE IF NOT EXISTS index_runs (
  id INTEGER PRIMARY KEY,
  started_at TEXT NOT NULL,
  finished_at TEXT
);

CREATE TABLE IF NOT EXISTS files (
  id INTEGER PRIMARY KEY,
  root_id INTEGER NOT NULL,
  rel_path TEXT NOT NULL,
  abs_path TEXT NOT NULL,
  is_dir INTEGER NOT NULL,
  is_symlink INTEGER NOT NULL,
  size INTEGER NOT NULL,
  mtime INTEGER NOT NULL,
  ext TEXT,
  status TEXT NOT NULL,
  last_seen_run INTEGER NOT NULL,
  FOREIGN KEY(root_id) REFERENCES roots(id)
);

CREATE TABLE IF NOT EXISTS tags (
  id INTEGER PRIMARY KEY,
  name TEXT NOT NULL UNIQUE
);

CREATE TABLE IF NOT EXISTS file_tags (
  file_id INTEGER NOT NULL,
  tag_id INTEGER NOT NULL,
  UNIQUE(file_id, tag_id),
  FOREIGN KEY(file_id) REFERENCES files(id),
  FOREIGN KEY(tag_id) REFERENCES tags(id)
);
```

---

## Indexes

```sql
CREATE UNIQUE INDEX IF NOT EXISTS idx_files_root_rel ON files(root_id, rel_path);
CREATE INDEX IF NOT EXISTS idx_files_status ON files(status);
CREATE INDEX IF NOT EXISTS idx_files_mtime ON files(mtime);
CREATE INDEX IF NOT EXISTS idx_files_ext ON files(ext);
CREATE INDEX IF NOT EXISTS idx_files_path_nocase ON files(abs_path COLLATE NOCASE);
CREATE INDEX IF NOT EXISTS idx_file_tags_tag ON file_tags(tag_id);
CREATE INDEX IF NOT EXISTS idx_file_tags_file ON file_tags(file_id);
```

---

## Notes

- `mtime` is stored as integer seconds since epoch for speed.
- `status` values: `active`, `deleted`.
- Use WAL mode on connection setup.

