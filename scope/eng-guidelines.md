## Engineering Guidelines for `catalog`

This document translates `scope/prd-001.md` into an engineer-ready architecture, system design, and rulebook for implementation.

---

## Architecture Overview

Core components and responsibilities:

- CLI layer: argument parsing, command routing, exit codes, and output formatting.
- Config + presets: config file load/save, preset expansion, validation, and defaults.
- Roots manager: add/remove/list roots, one-filesystem policy, excludes, last index time.
- Indexer: walks files, computes metadata, performs incremental updates, soft deletes.
- DB layer: SQLite schema, migrations, prepared queries, transactions, indexing.
- Search engine: SQL queries for substring search + filters (ext, tag, size, time, root).
- Tagging: tags table + join table, add/remove/list.
- Output: plain or JSON, stable schema for scripting.
- Logging: warnings, permission errors, summary per index run.

---

## System Design

### Data Flow

1. CLI parses command -> loads config + DB -> invokes component.
2. Index run: compile ignore rules -> walk roots -> upsert file rows -> mark missing as deleted -> update root timestamps.
3. Search: parse query + filters -> SQL -> format output.

### Config

Suggested defaults (explicit, macOS-first):

- Config path: `~/Library/Application Support/catalog/config.toml`
- DB path: `~/Library/Application Support/catalog/catalog.db`
- Env overrides: `CATALOG_CONFIG`, `CATALOG_DB`
- Config schema (TOML):
  - `roots = ["..."]`
  - `excludes = ["**/.git/**", "**/node_modules/**", ...]`
  - `include_hidden = false`
  - `one_filesystem = true`
  - `output = "plain" | "json"`

### Preset Expansion

On `init --preset`:

- Resolve roots by checking filesystem existence.
- For `~/Developer` and `~/Projects`, include the first existing.
- Expand `~` to home.
- Ignore missing roots; only include existing paths in config.

### SQLite Schema

Minimal schema from PRD plus two operational fields for incremental indexing:

- `roots(id, path, added_at, preset_name, last_indexed_at, one_filesystem)`
- `files(id, root_id, rel_path, abs_path, is_dir, is_symlink, size, mtime, ext, status, last_seen_run)`
- `tags(id, name UNIQUE)`
- `file_tags(file_id, tag_id, UNIQUE(file_id, tag_id))`
- `index_runs(id, started_at, finished_at)`

### Indexes

Required for speed:

- `files(root_id, rel_path)` UNIQUE
- `files(status)`
- `files(mtime)`
- `files(ext)`
- `files(abs_path)` or `files(rel_path)` with `COLLATE NOCASE`
- `file_tags(tag_id)`
- `file_tags(file_id)`

### Search Query Strategy

Case-insensitive substring on filename/path:

- Use `LOWER(abs_path) LIKE ?` with `COLLATE NOCASE`, or store `abs_path_lc`.
- Filter by:
  - `ext IN (...)`
  - `mtime` range
  - `size` range
  - `root_id`
  - tag join via `file_tags`
- Always filter `status='active'`.

### Indexing Algorithm (Incremental)

Per root, single transaction per root or per run:

1. Start `index_run` row -> get `run_id`.
2. Walk root with `walkdir`, `follow_links(false)`.
3. Skip entries using ignore rules and `include_hidden`.
4. For each entry:
   - Compute `rel_path`, `abs_path`, `ext`, `is_dir`, `is_symlink`, `size`, `mtime`.
   - Upsert row keyed by `(root_id, rel_path)`.
   - If unchanged `(size, mtime)` -> update `last_seen_run = run_id` only.
   - Else update metadata fields + `last_seen_run = run_id`.
5. After walk, mark missing:
   - `UPDATE files SET status='deleted' WHERE root_id=? AND last_seen_run != run_id`
6. Update `roots.last_indexed_at`.

### Deletions

Soft delete only. Never remove rows automatically. Tag associations remain. `search` excludes deleted by default.

### Symlinks

Default: do not follow. Index symlink itself with `is_symlink = true`. Never traverse into it.

---

## Rulebook (Engineering Guidelines)

### Core Invariants

- Never read file contents in v1. Metadata only.
- Never follow symlinks by default.
- Never index outside configured roots.
- Always apply excludes before descending into a directory.
- `index` must be deterministic and local-only.
- `search` must be <100ms for typical DB sizes.

### Behavioral Rules

- Permission errors: log + continue. Summarize at end.
- Hidden files: excluded unless `include_hidden=true`.
- One-filesystem: enforced per root unless user opts out.
- `--full` index: treat as fresh run, but still soft delete rather than dropping rows.

### CLI UX Rules

- Stable output fields in plain mode: `id  mtime  size  path`.
- `--json` must be stable for scripting and include all fields shown in plain mode.
- Use exit code `0` on success, `1` on user error, `2` on internal error.

### Performance Rules

- Use SQLite WAL mode.
- Use prepared statements.
- Use transactions for batch updates.
- Avoid reading huge directory metadata into memory. Streaming walk.

### Extensibility Rules

- Keep schema migrations explicit and versioned.
- Add new fields with migrations only.
- Do not repurpose columns across versions.

---

## Module Layout (Suggested)

- `src/main.rs`
  - Entry point, CLI wiring, top-level error handling.
- `src/cli.rs`
  - `clap` definitions and argument structs.
- `src/config.rs`
  - Config load/save, defaults, env overrides, presets.
- `src/roots.rs`
  - Add/remove/list roots and validation logic.
- `src/indexer.rs`
  - Directory walk and DB updates.
- `src/ignore.rs`
  - Exclude matcher using `ignore` crate.
- `src/db.rs`
  - Connection setup, migrations, prepared statements.
- `src/search.rs`
  - Search SQL builder and query execution.
- `src/tags.rs`
  - Tag CRUD.
- `src/output.rs`
  - Plain + JSON formatting.

---

## Command Behavior Details

### `catalog init [--preset ...]`

- Create config + DB in default locations.
- Store preset name in config for traceability.

### `catalog roots`

- Show roots, excludes, `include_hidden`, `one_filesystem`, `last_indexed_at`.

### `catalog add <path>...`

- Normalize paths and prevent duplicates.
- Reject non-existent paths with a warning.

### `catalog rm <path>...`

- Remove from config only, do not delete DB rows.

### `catalog index [--full] [--one-filesystem]`

- `--full` resets file statuses to `deleted` before walk.
- `--one-filesystem` overrides config for this run.

### `catalog search <query> ...`

- `query` is substring match across filename + path.
- `--ext` is comma-separated.
- `--tag` can be repeated or comma-separated.

### `catalog recent [--days N] [--limit N]`

- Sort by `mtime DESC`.
- Default `days=7`, `limit=50`.

### `catalog tag add|rm <path|id> <tag>`

- Resolve `path` to `file_id` via `abs_path`.
- Tags are case-insensitive for uniqueness.

---

## Error Handling Rules

- User errors: return `1` with a concise message and example usage.
- Permission errors: warn once per root or aggregate count.
- DB errors: return `2` and print the sqlite error code.

---

## Testing Checklist (V1)

- Config round-trip (serialize/deserialize).
- Preset expansion picks correct roots.
- Ignore matcher filters `node_modules` and `.git`.
- Indexer inserts and updates files correctly.
- Soft deletes are applied after missing file.
- Search returns case-insensitive results.
- Filters for ext, date, size, tag, root.
- Tag add/remove works with path and id.
- Permission errors don't crash.

---

## Open Decisions (Make Explicit Early)

- Config + DB location: recommended `~/Library/Application Support/catalog/`.
- Path normalization: store `abs_path` as joined root + rel path without canonicalizing.
- Tag normalization: lowercase on insert for uniqueness.

