## Store Schema and Versioning

This document defines the binary store format and versioning strategy. The on-disk snapshot is binary; JSON export is for debugging.

---

## Versioning Strategy

- Store a `version` integer at the top level.
- Increment on breaking changes.
- If an unknown version is found, fail fast with a clear error.
- Current version: **3** (segmented on-disk layout; see below).

---

## On-Disk Layout (Version 3)

Version 3 splits the single binary blob into a small manifest plus per-root
segments, so an index run rewrites only what changed instead of the whole store.

Given a store path `catalog.bin`:

- **`catalog.bin`** — manifest. An 8-byte magic prefix `CATALOG\x03` (last byte =
  format version) followed by a bincode-encoded `StoreData` with `files` and
  `dir_sizes` left empty. Holds `version`, counters, `roots`, `tags`, `file_tags`,
  and `dir_sizes_run_id`. Rewritten on every save (it is tiny).
- **`catalog.bin.d/root-<id>.seg`** — one segment per root, a bincode `Vec<FileEntry>`.
  Rewritten only when that root's file set changed (added/updated/deleted rows).
- **`catalog.bin.d/dirsizes.seg`** — bincode `Vec<DirSizeEntry>`. Rewritten only when
  directory totals changed.

On load, the manifest is read, then each root's segment is concatenated back into a
single in-memory `files` vector (search/analyze/export see the same flat model as
before). Segments for roots no longer present are deleted on the next save.

All writes remain atomic (temp file → fsync → rename), per file.

### Migration from Version 2

A legacy v2 file is a single bincode `StoreData` blob with no magic prefix. On load it
is read whole and flagged for a full rewrite; the next save converts it to the v3
manifest + segments layout in place at the same path.

---

## Base Schema (Version 2, legacy)

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
- `last_seen_run` is retained on `FileEntry` for informational continuity but is no
  longer used to detect deletions: the v3 indexer reconciles each root via a sorted
  merge-join, so a stored key absent from the latest scan is the delete signal.
