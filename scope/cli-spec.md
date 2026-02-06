## CLI UX Specification

This document defines CLI commands, flags, output formats, and exit codes.

---

## Global Behavior

- Default output is plain text.
- `--json` switches output to JSON for scripting.
- Exit codes:
  - `0` success
  - `1` user error
  - `2` internal error

---

## Commands

### `catalog init [--preset macos-user-additions|macos-deep|macos-full]`

- Creates config and store if missing.
- Writes preset roots to config when `--preset` is provided.
- No indexing occurs.

Example:

```sh
catalog init --preset macos-user-additions
```

### `catalog roots`

- Prints configured roots, excludes, include_hidden, one_filesystem, and last_indexed_at.

### `catalog add <path>...`

- Adds one or more roots to config.
- Normalizes paths.
- Warns on missing paths.

### `catalog rm <path>...`

- Removes one or more roots from config.
- Purges store entries for removed roots.

### `catalog index [--full] [--one-filesystem]`

- Incrementally indexes roots.
- `--full` forces rescan and marks missing items as deleted.
- `--one-filesystem` overrides config for this run.

### `catalog search <query> [--ext ...] [--after ...] [--before ...] [--min-size ...] [--max-size ...] [--root ...] [--json]`

- Case-insensitive substring match on filename and path.
- Filters are optional.

Examples:

```sh
catalog search font --ext ttf,otf
catalog search launch --after 2024-01-01 --root ~/Library/LaunchAgents
```

### `catalog recent [--days N] [--limit N]`

- Lists recently modified files.
- Defaults: `days=7`, `limit=50`.

### `catalog export [--output <path>]`

- Exports the store as JSON.
- When `--output` is provided, writes to a file; otherwise prints to stdout.

### `catalog prune`

- Removes all stored index data while keeping config.

### `catalog analyze [path] [--top N] [--files N] [--json] [--raw] [--tui]`

- Reports what occupies the most space under a path (or entire disk).
- Reuses the index scan when possible to avoid duplicate filesystem walks.
- Defaults: `top=20`, `files=20`.
- Auto-refreshes if the stored index is older than 1 day.
- Defaults to an interactive browser (arrow keys or mouse to navigate, Enter to drill, Backspace to go back).
- `--raw` prints the plain text report instead of the TUI.

### `catalog watch [--interval N] [--full] [--one-filesystem]`

- Polls for changes and re-indexes on an interval.
- Default interval: 30 seconds.
- `--full` forces full rescan every interval.
- `--one-filesystem` overrides config for this run.

---

## Output Formats

### Plain Output

- Search and recent output format:
  - `id  mtime  size  path`

### JSON Output

- Output is a JSON array of objects.
- Fields:
  - `id` integer
  - `path` string
  - `mtime` string, RFC 3339 or `YYYY-MM-DD HH:MM:SS`
  - `size` integer
  - `is_dir` boolean
  - `is_symlink` boolean
  - `ext` string or null
  - `root` string
  - `status` string

---

## Error Message Style

- Keep messages concise.
- Include actionable hints.
- Do not print stack traces by default.
