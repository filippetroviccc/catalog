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
- Disk usage analysis returns correct totals and top-N ordering.

---

## Fixtures

- Small test tree with nested directories.
- Symlink cases to ensure no traversal.
- Hidden files and directories.
- Permission-denied directory simulation.

---

## Performance Smoke Test

Run a lightweight performance + correctness check that exercises indexing and analyze logic:

```sh
cargo run --bin perf_smoke
```

Environment overrides:

```sh
CATALOG_PERF_DIRS=20 CATALOG_PERF_FILES_PER_DIR=150 CATALOG_PERF_FILE_SIZE=4096 \\
CATALOG_PERF_MAX_INDEX_SECS=10 CATALOG_PERF_MAX_ANALYZE_SECS=3 CATALOG_PERF_MAX_BROWSE_SECS=3 \\
cargo run --bin perf_smoke
```
