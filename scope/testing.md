## Testing Status

This document reflects the current automated test coverage in `catalog`.

---

## Unit Tests Present

- Config load/save round trip.
- Root sync pruning removed root data and orphan tags.
- Indexer behavior for excludes, hidden files, and soft delete.
- Search filter behavior (`--ext` path).
- Analyze totals, top-N ordering, and filtered analyze behavior.
- Store binary round-trip, ID counter repair, JSON export round-trip, and stale-index checks.

---

## Coverage Notes

- Tests are colocated inside module `#[cfg(test)]` blocks under `src/*.rs`.
- There are currently no dedicated integration tests under `tests/`.
- There are currently no CLI parsing tests for individual subcommands.

---

## Performance Smoke Test

Run a lightweight performance + correctness check that exercises indexing and analyze logic:

```sh
cargo run --bin perf_smoke
```

Environment overrides:

```sh
CATALOG_PERF_DIRS=20 CATALOG_PERF_FILES_PER_DIR=150 CATALOG_PERF_FILE_SIZE=4096 \
CATALOG_PERF_MAX_INDEX_SECS=10 CATALOG_PERF_MAX_ANALYZE_SECS=3 CATALOG_PERF_MAX_BROWSE_SECS=3 \
cargo run --bin perf_smoke
```
