## Store Schema and Versioning

This document defines the binary store format and versioning strategy. The on-disk snapshot is binary; JSON export is for debugging.

---

## Versioning Strategy

- Store a `version` integer at the top level.
- Increment on breaking changes.
- If an unknown version is found, fail fast with a clear error.

---

## Base Schema (Version 2)

Top-level fields:

```json
{
  "version": 2,
  "last_run_id": 0,
  "next_root_id": 1,
  "next_file_id": 1,
  "next_tag_id": 1,
  "roots": [],
  "files": [],
  "tags": [],
  "file_tags": [],
  "dir_sizes_run_id": 0,
  "dir_sizes": []
}
```

### `roots`

```json
{
  "id": 1,
  "path": "/Users/alice/Downloads",
  "added_at": "2026-02-05T10:00:00-08:00",
  "preset_name": "macos-user-additions",
  "last_indexed_at": "2026-02-05T10:10:00-08:00",
  "one_filesystem": true
}
```

### `files`

```json
{
  "id": 10,
  "root_id": 1,
  "rel_path": "notes/todo.txt",
  "abs_path": "/Users/alice/Downloads/notes/todo.txt",
  "is_dir": false,
  "is_symlink": false,
  "size": 1234,
  "mtime": 1707150000,
  "ext": "txt",
  "status": "active",
  "last_seen_run": 3
}
```

### `tags` and `file_tags` (unused)

These are reserved for potential future use and are not used by the current CLI.

### `dir_sizes`

Cached directory totals computed during indexing for fast `analyze`:

```json
{
  "path": "/Users/alice/Downloads/projects",
  "size": 987654321
}
```

`dir_sizes_run_id` tracks the index run that produced the cache and is compared to `last_run_id` to confirm freshness.

---

## Notes

- `mtime` is stored as integer seconds since epoch for speed.
- `status` values: `active`, `deleted`.
- Store writes are atomic: write to temp, fsync, rename.
