## Indexing Rules

This document defines directory walk behavior, ignore rules, and metadata capture.

---

## Walk Rules

- Walk roots recursively.
- Do not follow symlinks.
- Index the symlink node itself if encountered.
- Apply excludes before descending into directories.

---

## Hidden Files

- Hidden files and directories are excluded by default.
- Hidden inclusion is enabled via `include_hidden=true` in config.

---

## One Filesystem

- Default `one_filesystem=true` per root.
- When enabled, do not cross filesystem boundaries.
- `catalog index --one-filesystem` overrides config for the run.

---

## Exclude Matching

- Excludes are gitignore-style glob patterns.
- Patterns are matched against full relative paths from each root.
- Example excludes:
  - `**/.git/**`
  - `**/node_modules/**`
  - `**/target/**`
  - `**/dist/**`
  - `**/build/**`

---

## Metadata Capture

For each indexed entry store:

- `rel_path`
- `abs_path`
- `is_dir`
- `is_symlink`
- `size`
- `mtime`
- `ext`

---

## Change Detection

- Entries are upserted by `(root_id, rel_path)` on every run.
- Current implementation updates metadata fields and `last_seen_run` for each seen entry.
- `size` and `mtime` are captured each run and stored for downstream filtering/sorting.

---

## Deletion Handling

- Missing files are soft deleted.
- Soft delete is applied after walk using `last_seen_run`.

---

## Error Handling

- Permission errors are logged and do not abort.
- A summary of skipped paths is printed at the end of the run.
