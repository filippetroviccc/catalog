## Operational Guidelines

This document defines logging, error handling, and operational behavior.

---

## Logging

- Use structured logging with levels.
- Default level is `info`.
- Errors and warnings are short and actionable.

---

## Permission Errors

- Continue indexing when permission errors occur.
- Print a per-root summary count of skipped permission entries.

---

## Crash and Recovery

- Store writes are atomic (write temp, fsync, rename).
- Partial index runs should not corrupt the store.

---

## Output Stability

- JSON output fields and types must remain stable within a major version.
- Plain output columns remain stable within a major version.
