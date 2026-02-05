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

### `catalog init [--preset macos-user-additions|macos-deep]`

- Creates config and DB if missing.
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
- Does not delete DB rows.

### `catalog index [--full] [--one-filesystem]`

- Incrementally indexes roots.
- `--full` forces rescan and marks missing items as deleted.
- `--one-filesystem` overrides config for this run.

### `catalog search <query> [--ext ...] [--tag ...] [--after ...] [--before ...] [--min-size ...] [--max-size ...] [--root ...] [--json]`

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

### `catalog tag add <path|id> <tag>`

- Adds a tag to a file referenced by absolute path or file id.

### `catalog tag rm <path|id> <tag>`

- Removes a tag from a file referenced by absolute path or file id.

### `catalog tags`

- Lists tags and counts.

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

