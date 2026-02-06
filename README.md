# catalog

Local-first CLI to index and search file metadata on macOS. No file contents are read. Metadata is stored in a compact binary snapshot on disk (with JSON export for debugging).

## Install

Build and install the CLI so you can run `catalog` directly:

```sh
cargo install --path .
```

Make sure `~/.cargo/bin` is in your `PATH`.

## Quick Start

```sh
# Initialize config + store
catalog init --preset macos-user-additions

# Index configured roots
catalog index

# Search
catalog search <query>
```

## Debug Logging

Enable debug logs for the `catalog` crate:

```sh
catalog --debug index
```

You can also use standard `RUST_LOG` filters:

```sh
RUST_LOG=catalog=debug catalog index
```

## Commands

### Roots and config

```sh
# Show configured roots and settings
catalog roots

# Add roots
catalog add ~/path/to/dir ~/another/dir

# Remove roots
catalog rm ~/path/to/dir
```

### Indexing

```sh
# Incremental index
catalog index

# Full rescan
catalog index --full

# Override one-filesystem for this run
catalog index --one-filesystem
```

### Search

```sh
# Basic search
catalog search font

# Extensions (comma-separated)
catalog search font --ext ttf,otf

# Date range (YYYY-MM-DD)
catalog search launch --after 2024-01-01 --before 2025-01-01

# Size filters (bytes)
catalog search log --min-size 1000 --max-size 100000

# Restrict to a root
catalog search build --root ~/Projects

# Long output (more metadata)
catalog search report --long
```

### Recent files

```sh
catalog recent
catalog recent --days 3 --limit 20
catalog recent --long
```

## Output

- Default output is plain text.
- Add `--json` to `search` or `recent` for machine-readable output.
- Add `--long` for additional metadata.

```sh
catalog search <query> --json
catalog recent --json
```

## Paths and Environment Variables

Default paths:

- Config: `~/Library/Application Support/catalog/config.toml`
- Store: `~/Library/Application Support/catalog/catalog.bin`

Overrides:

```sh
CATALOG_CONFIG=/path/to/config.toml catalog index
CATALOG_STORE=/path/to/store.bin catalog index
```

Legacy override (still accepted):

```sh
CATALOG_DB=/path/to/store.bin catalog index
```

## Export Store as JSON

```sh
catalog export
catalog export --output /tmp/catalog.json
```

## Prune (Hard Reset)

Remove all stored index data while keeping config:

```sh
catalog prune
```

## Notes

- Indexing is metadata-only (no content reads).
- Hidden files are excluded by default unless `include_hidden=true` in config.
- Soft deletes: missing files are marked deleted, not removed.
