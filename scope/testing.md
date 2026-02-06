## Testing Plan

This document lists MVP tests and fixtures.

---

## Unit Tests

- Config load/save round trip.
- Preset expansion picks correct roots.
- Ignore matcher filters `node_modules` and `.git`.
- CLI argument parsing for each command.

---

## Integration Tests

- Indexer inserts files and updates metadata.
- Incremental index with no changes is fast and minimal.
- Soft delete on missing file after index run.
- Search finds case-insensitive matches.
- Search filters by ext, size, time, root.

---

## Fixtures

- Small test tree with nested directories.
- Symlink cases to ensure no traversal.
- Hidden files and directories.
- Permission-denied directory simulation.
