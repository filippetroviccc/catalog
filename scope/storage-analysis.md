## Storage Analysis

This document describes the current `catalog analyze` behavior.

---

## CLI

- `catalog analyze [path] [--top N] [--files N] [--json] [--raw] [--tui]`
- Default mode is interactive TUI when neither `--json` nor `--raw` is set.
- `--raw` prints a plain text report.
- `--json` prints a machine-readable report.

---

## Data Source

- Analyze reads from the existing index snapshot in the store.
- If the relevant index is older than 1 day (or missing), analyze triggers a fresh index run first.
- Directory-size cache (`dir_sizes`) is reused when available for faster repeated reports/browsing.

---

## Report Contents

- `total_scanned`: total bytes represented by active, non-directory entries in scope.
- `roots`: per-root totals.
- `top_dirs`: top N directories by aggregated size.
- `top_files`: top N files by size.

---

## Scope and Filtering

- When `path` is omitted, analysis covers configured roots.
- When `path` is provided, only data under that path is included.
- Deleted entries are excluded from analysis.

---

## Current Limitations

- No disk-level “hidden space” reconciliation against filesystem-reported used space.
- No explicit permission-error breakdown in analyze output (permission handling is reported during indexing).
