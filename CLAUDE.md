# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

`catalog` is a local-first CLI tool for indexing and searching files on macOS. It stores file metadata in a JSON snapshot on disk (no database dependency) and provides fast search capabilities without reading file contents.

**Key principles:**
- Metadata-only indexing (no file content reading in v1)
- Incremental updates with soft deletes
- Local-only, deterministic results
- macOS-first with strong defaults for user-added files

## Commands

### Building and Running
```bash
# Build the project
cargo build

# Build optimized release binary
cargo build --release

# Run without installing
cargo run -- <command>

# Run with debug logging
RUST_LOG=debug cargo run -- <command>
```

### Testing
```bash
# Run all tests
cargo test

# Run a specific test
cargo test <test_name>

# Run tests with output
cargo test -- --nocapture

# Run tests for a specific module
cargo test <module_name>::
```

### Development Workflow
```bash
# Initialize catalog (creates config + store)
cargo run -- init --preset macos-user-additions

# Index configured roots
cargo run -- index

# Search for files
cargo run -- search <query>

# Add custom root
cargo run -- add ~/path/to/dir

# View configured roots
cargo run -- roots
```

## Architecture

### Module Responsibilities

- **`cli.rs`**: Command-line argument parsing using `clap`. Defines all CLI commands and their parameters.
- **`config.rs`**: Config file (TOML) load/save, preset expansion, path resolution. Handles `~/Library/Application Support/catalog/` defaults.
- **`store.rs`**: Binary store load/save, atomic writes, ID counters, and JSON export.
- **`indexer.rs`**: Directory walking with incremental update logic. Uses `walkdir` for traversal and `ignore` crate for gitignore-style excludes.
- **`search.rs`**: In-memory search with filters (ext, date, size, root). Case-insensitive substring matching.
- **`roots.rs`**: Root path add/remove/sync logic with config and store.
- **`output.rs`**: Plain text and JSON output formatting for search results.
- **`util.rs`**: Shared utilities.

### Data Flow

1. **Index**: Load config → Load binary store → Walk roots with ignore rules → Upsert files (keyed by `root_id + rel_path`) → Mark missing files as deleted → Update root timestamps → Save store
2. **Search**: Parse query + filters → In-memory filter over store → Format output (plain or JSON)

### Store Schema (JSON)

Top-level fields:
- `version`, `last_run_id`, and `next_*_id` counters
- `roots`: Indexed directory paths with `last_indexed_at` timestamps
- `files`: File metadata (path, size, mtime, type) with status (`active`/`deleted`) and `last_seen_run`

### Incremental Indexing

Files are identified by `(root_id, rel_path)`. Each index run:
1. Increments a `run_id`
2. Walks directories and upserts files
3. Updates `last_seen_run` and sets status `active`
4. After walk, marks files not seen in this run as deleted (`status='deleted'`)
5. Search queries filter by `status='active'` by default

## Implementation Guidelines

### Core Invariants
- Never read file contents in v1
- Never follow symlinks by default
- Never index outside configured roots
- Always apply excludes before descending into directories
- Indexing must be deterministic and local-only

### Behavioral Rules
- Permission errors: log and continue, summarize at end
- Hidden files: excluded unless `include_hidden=true`
- One-filesystem: enforced per root unless user opts out
- Soft delete only—never remove rows automatically

### Performance Targets
- Search response: <100ms for typical store sizes (100k-500k entries)
- Re-index with no changes: very fast (seconds)
- Prefer in-memory filtering and avoid repeated full-store rewrites when no changes are detected

### Error Handling
- Exit code `0` on success
- Exit code `1` on user error (show concise message + usage hint)
- Exit code `2` on internal error (include storage path/context when relevant)

### Config Defaults
- Config path: `~/Library/Application Support/catalog/config.toml`
- Store path: `~/Library/Application Support/catalog/catalog.bin`
- Env overrides: `CATALOG_CONFIG`, `CATALOG_STORE` (and legacy `CATALOG_DB`)

### Default Excludes
Strong noise filters prevent indexing:
- `~/Library/Caches`, `~/Library/Containers`, `~/Library/Logs`
- `**/.git/**`, `**/node_modules/**`, `**/target/**`, `**/dist/**`, `**/build/**`
- `~/Library/Developer/Xcode/DerivedData`

## Reference Documents

Detailed specifications in `scope/`:
- `prd-001.md`: Product requirements and user stories
- `eng-guidelines.md`: Architecture and implementation rules
- `schema.md`: Database schema and migration strategy
- `cli-spec.md`: CLI commands, flags, and output formats
- `testing.md`: Test plan and fixtures
- `indexing-rules.md`: File traversal and exclude logic
- `config-spec.md`: Configuration file format
- `performance.md`: Performance targets and optimization strategies

Read these documents when working on the corresponding component.
