## Performance and Scale Targets

This document defines performance budgets and constraints for MVP.

---

## Targets

- Search median latency under 100ms for 100k to 500k entries.
- Incremental index with no changes completes in seconds on typical laptops.
- Full index remains usable for 100k to 500k entries.

---

## Constraints

- Avoid loading all paths into memory.
- Use streaming directory walk.
- Use prepared statements and transactions.
- Use WAL mode for SQLite.

---

## Benchmark Guidance

- Benchmark search with 100k, 250k, and 500k records.
- Benchmark incremental index when no files change.
- Record cold start and warm start times.

