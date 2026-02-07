# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

`catalog` is a local-first CLI tool for indexing and searching files on macOS. It stores file metadata in a binary snapshot on disk (no database dependency) and provides fast search capabilities without reading file contents.

**Key principles:**
- Metadata-only indexing (no file content reading in v1)
- Incremental updates with soft deletes
- Local-only, deterministic results
- macOS-first with preset-driven indexing scopes

## Commands

### Building and Running
```bash
# Build the project
cargo build

# Build optimized release binary
cargo build --release

# Install locally so you can run `catalog` directly
cargo install --path .

# Run without installing
cargo run --bin catalog -- <command>

# Run with debug logging (via --debug flag)
cargo run --bin catalog -- --debug <command>

# Or use RUST_LOG for fine-grained control
RUST_LOG=debug cargo run --bin catalog -- <command>
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
cargo run --bin catalog -- init --preset macos-user-additions

# Index configured roots
cargo run --bin catalog -- index

# Full rescan (re-index everything)
cargo run --bin catalog -- index --full

# Search for files
cargo run --bin catalog -- search <query>

# Search with filters
cargo run --bin catalog -- search font --ext ttf,otf
cargo run --bin catalog -- search log --min-size 1000 --max-size 100000
cargo run --bin catalog -- search build --root ~/Projects --after 2024-01-01

# View recent files
cargo run --bin catalog -- recent --days 3 --limit 20

# Analyze disk usage (auto-refreshes if index is >1 day old)
cargo run --bin catalog -- analyze ~/Projects --top 20 --files 20
# Interactive browse (TUI, default)
cargo run --bin catalog -- analyze
# Raw text report
cargo run --bin catalog -- analyze --raw

# Export store as JSON
cargo run --bin catalog -- export --output /tmp/catalog.json

# Add custom root
cargo run --bin catalog -- add ~/path/to/dir

# View configured roots
cargo run --bin catalog -- roots

# Prune (hard reset - removes all index data, keeps config)
cargo run --bin catalog -- prune
```

## CLI Commands Reference

### Core Commands
- `init [--preset <name>]`: Creates config and store. Presets: `macos-user-additions`, `macos-deep`, `macos-full`
- `index [--full] [--one-filesystem]`: Incremental index (or full rescan with `--full`)
- `watch [--interval N] [--full] [--one-filesystem]`: Polling loop that re-runs indexing
- `search <query> [--ext] [--after] [--before] [--min-size] [--max-size] [--root] [--long] [--json]`: Search with filters
- `recent [--days N] [--limit N] [--long] [--json]`: List recently modified files (defaults: 7 days, 50 limit)
- `analyze [path] [--top N] [--files N] [--json] [--raw] [--tui]`: Disk usage analysis; auto-refreshes if index >1 day old

### Configuration Commands
- `roots`: View configured roots and settings
- `add <path>...`: Add one or more roots to config
- `rm <path>...`: Remove roots and purge their store entries

### Maintenance Commands
- `export [--output <path>]`: Export store as JSON (to stdout or file)
- `prune`: Hard reset - removes all index data while keeping config

### Global Flags
- `--debug`: Enable debug logging (alternative to `RUST_LOG=debug`)
- `--json`: Output in JSON format (for `search`, `recent`, `analyze`)
- `--long`: Show additional metadata (for `search`, `recent`)

## Architecture

### Module Responsibilities

- **`cli.rs`**: Command-line argument parsing using `clap`. Defines all CLI commands and their parameters.
- **`config.rs`**: Config file (TOML) load/save, preset expansion, path resolution. Handles `~/Library/Application Support/catalog/` defaults.
- **`store.rs`**: Binary store load/save, atomic writes, ID counters, and JSON export.
- **`indexer.rs`**: Directory walking with incremental update logic. Uses `ignore::WalkBuilder` for parallel traversal and gitignore-style excludes.
- **`search.rs`**: In-memory search with filters (ext, date, size, root). Case-insensitive substring matching.
- **`roots.rs`**: Root path add/remove/sync logic with config and store.
- **`output.rs`**: Plain text and JSON output formatting for search results.
- **`util.rs`**: Shared utilities.
- **`analyze.rs`**: Disk usage analysis; reuses index scan results to avoid duplicate filesystem walks.

### Data Flow

1. **Index**: Load config → Load binary store → Walk roots with ignore rules → Upsert files (keyed by `root_id + rel_path`) → Mark missing files as deleted → Update root timestamps → Save store
2. **Search**: Parse query + filters → In-memory filter over store → Format output (plain or JSON)
3. **Analyze**: Reuse index scan results (or stored index) → Aggregate by directory/file → Report top usage

### Store Schema

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
- Exit code `1` on command/runtime errors
- Exit code `2` on CLI parse/usage errors from clap

### Config Defaults
- Config path: `~/Library/Application Support/catalog/config.toml`
- Store path: `~/Library/Application Support/catalog/catalog.bin`
- Env overrides: `CATALOG_CONFIG`, `CATALOG_STORE`

### Default Excludes
Strong noise filters prevent indexing:
- `~/Library/Caches`, `~/Library/Containers`, `~/Library/Logs`
- `**/.git/**`, `**/node_modules/**`, `**/target/**`, `**/dist/**`, `**/build/**`
- `~/Library/Developer/Xcode/DerivedData`

## Reference Documents

Detailed specifications in `scope/`:
- `prd-001.md`: Product requirements and user stories
- `eng-guidelines.md`: Architecture and implementation rules
- `schema.md`: Binary store schema and versioning strategy
- `cli-spec.md`: CLI commands, flags, and output formats
- `testing.md`: Test plan and fixtures
- `indexing-rules.md`: File traversal and exclude logic
- `config-spec.md`: Configuration file format
- `performance.md`: Performance targets and optimization strategies

Read these documents when working on the corresponding component.
