## Acceptance Criteria and Scope Boundaries

This document defines MVP acceptance criteria for `catalog` and explicit scope boundaries.

---

## MVP Acceptance Criteria

The MVP is complete when all items below are met.

### Core Commands

- `catalog init` creates config and store in default locations.
- `catalog init --preset macos-user-additions` expands preset roots and writes them to config.
- `catalog init` defaults to a full-system preset (`macos-full`).
- `catalog roots` prints configured roots, excludes, and last index time.
- `catalog add <path>...` adds roots to config and persists.
- `catalog rm <path>...` removes roots from config and persists.
- `catalog index` indexes configured roots incrementally.
- `catalog index --full` forces a full rescan.
- `catalog search <query>` returns case-insensitive substring matches on filename and path.
- `catalog recent` returns the most recently modified files.
- `catalog analyze` reports largest folders/files.

### Functional Behavior

- Indexing is metadata-only. No file contents are read.
- Indexing is incremental based on `size` and `mtime`.
- Missing files are soft deleted, not removed from store.
- Symlinks are not followed by default; the symlink itself may be indexed.
- Excludes are applied before descending into directories.
- Permission errors do not abort indexing; they are logged and summarized.

### Output Behavior

- Plain output format is stable: `id  mtime  size  path`.
- `--json` produces stable machine-readable output.
- Search filters work with `--ext`, `--after`, `--before`, `--min-size`, `--max-size`, `--root`.

### Performance Targets

- `search` median latency is under 100ms on 100k to 500k entries.
- `index` incremental run with no changes completes in seconds on typical laptops.

---

## Scope Boundaries

### In Scope (MVP)

- macOS-first CLI with binary store on disk.
- Presets for macOS user additions.
- Incremental indexing, search, recent.
- Config in TOML.

### Out of Scope (MVP)

- Cloud sync or multi-device.
- Full system indexing of `/System`, `/private/var`, or equivalent.
- OCR or PDF extraction.
- AI ranking or embeddings.
- Content indexing or FTS.

### Post-MVP (V1.1 and beyond)

- `catalog watch` for filesystem notifications.
- Rename and move detection using inode/device.
- Optional FTS for content search.
