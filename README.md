# catalog

**Fast local file system indexing and analysis for macOS.**

`catalog` helps you:
- **Analyze disk usage** — Find what's consuming the most storage space
- **Search files instantly** — Query metadata without reading file contents
- **Track changes** — Incremental indexing with soft deletes keeps your index fresh

All metadata is stored locally in a compact snapshot (no database required). Privacy-first: file contents are never read.

## Features

### Fast File Search
Search across all indexed files with powerful filters:
```sh
catalog search report --ext pdf,docx --after 2024-01-01
catalog search build --root ~/Projects --min-size 1000000
```

### Storage Analysis
Discover what's consuming disk space with interactive or text reports:
```sh
catalog analyze ~/Projects          # Auto-refreshes if >1 day old
catalog analyze --top 20 --files 20 # Show top directories and files
```

### Incremental Indexing
Only scans what's changed since the last run:
```sh
catalog index              # Fast incremental update
catalog index --full       # Complete rescan
```

## Install

Build and install the CLI so you can run `catalog` directly:

```sh
cargo install --path .
```

Make sure `~/.cargo/bin` is in your `PATH`.

## Quick Start

```sh
# Initialize with smart defaults for user-added files
catalog init --preset macos-user-additions

# Index configured roots
catalog index

# Search for files
catalog search <query>

# Analyze disk usage
catalog analyze
```

## Core Commands

### Storage Analysis

Find what's consuming the most disk space:

```sh
# Interactive TUI (default)
catalog analyze

# Analyze specific path with top 20 directories and files
catalog analyze ~/Projects --top 20 --files 20

# Text report (no TUI)
catalog analyze --raw

# JSON output for scripting
catalog analyze --json
```

**Note:** Auto-refreshes the index if it's older than 1 day.

### Search

Search across all indexed files with powerful filters:

```sh
# Basic search
catalog search font

# Filter by extension
catalog search font --ext ttf,otf

# Date range (YYYY-MM-DD)
catalog search launch --after 2024-01-01 --before 2025-01-01

# Size filters (bytes)
catalog search log --min-size 1000 --max-size 100000

# Restrict to a specific directory
catalog search build --root ~/Projects

# Show additional metadata
catalog search report --long

# JSON output
catalog search report --json
```

### Recent Files

View recently modified files:

```sh
catalog recent                    # Last 7 days, 50 files
catalog recent --days 3 --limit 20
catalog recent --long --json
```

### Indexing

Keep your file index up to date:

```sh
# Incremental index (fast, only scans changes)
catalog index

# Full rescan (re-index everything)
catalog index --full

# Stay on same filesystem
catalog index --one-filesystem
```

## Configuration

### Managing Roots

Control which directories are indexed:

```sh
# Show configured roots and settings
catalog roots

# Add directories to index
catalog add ~/path/to/dir ~/another/dir

# Remove directories (also purges their index entries)
catalog rm ~/path/to/dir
```

### Debug Logging

Enable detailed logging:

```sh
catalog --debug index
# Or with RUST_LOG
RUST_LOG=catalog=debug catalog index
```

## Advanced

### Export & Maintenance

```sh
# Export store as JSON for debugging/analysis
catalog export --output /tmp/catalog.json

# Hard reset (remove all index data, keep config)
catalog prune
```

### Paths and Environment Variables

Default locations:
- Config: `~/Library/Application Support/catalog/config.toml`
- Store: `~/Library/Application Support/catalog/catalog.bin`

Override with environment variables:
```sh
CATALOG_CONFIG=/path/to/config.toml catalog index
CATALOG_STORE=/path/to/store.bin catalog index
```

## How It Works

- **Metadata-only indexing** — File contents are never read, only metadata (size, mtime, extension)
- **Incremental updates** — Only scans what changed since last run
- **Soft deletes** — Missing files are marked as deleted, not removed
- **Smart excludes** — Skips noise like `.git`, `node_modules`, `~/Library/Caches` by default
- **Local-first** — All data stays on your machine in a compact binary format

## Presets

Choose an indexing scope when you initialize:

- `macos-user-additions` — Only user-added files (Documents, Downloads, Desktop, etc.)
- `macos-deep` — User files + common dev directories
- `macos-full` — Everything except system noise

```sh
catalog init --preset macos-user-additions
```
