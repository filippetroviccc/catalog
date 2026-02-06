## Storage Analysis Feature (Disk Usage)

This document defines a disk-usage analysis feature for `catalog`, inspired by DaisyDisk's user-facing behavior and constraints.

---

## Goals

- Let users analyze disk usage and identify what occupies the most space.
- Support scanning whole disks or individual folders.
- Provide clear feedback about unscanned/hidden space due to permissions or system behavior.

---

## DaisyDisk Behavior (Research Summary)

- Scans can target entire disks or specific folders; scanning a folder reduces time due to fewer files. citeturn0search2turn0search3
- Scan time depends mostly on the number of files and disk type; network/virtual disks can be slower. citeturn0search1turn0search2
- DaisyDisk can scan multiple disks simultaneously (resource intensive). citeturn0search1turn0search8
- It surfaces "hidden space" when total scanned files do not match used disk space, often due to restricted folders, purgeable space, snapshots, APFS volumes, or filesystem overhead. citeturn0search0turn0search6turn0search9
- It recommends Full Disk Access or administrator scanning to reduce hidden space. citeturn0search4turn0search6

---

## Functional Requirements

### Scan Targets
- Allow scanning:
  - Whole disks (mounted volumes).
  - Individual folders (faster, smaller scope).
- Scan can be run per target and stored as a snapshot.

### CLI
- `catalog analyze [path] [--top N] [--files N] [--json]`
- Defaults to configured roots when `path` is omitted.
- Uses the existing index; automatically re-indexes when index is older than 1 day.

### Output
- Rank and display highest-occupancy paths (folders/files).
- Provide summaries:
  - Total size by top-level directory.
  - Largest files.
  - Largest folders.
- Include a per-root summary showing how much space was scanned under each analyzed root.
- Mark any unaccounted space as **hidden space** (if used space > scanned totals).

### Permissions / Hidden Space
- If traversal errors or permission denials occur, surface:
  - Count of errors.
  - Example path(s).
  - Guidance to grant Full Disk Access (macOS) to reduce hidden space. citeturn0search4turn0search6

---

## Non-Functional Requirements

### Performance Targets
- Initial scan time primarily scales with number of files, not total disk size. citeturn0search1turn0search2
- Support parallel scanning of multiple disks with resource limits. citeturn0search1turn0search8

### UX
- Provide scan progress per target plus an overall progress view.
- If possible, allow reusing recent snapshots (e.g., cached scan results).

---

## Implementation Notes (Catalog Context)

- Current indexer already walks folders and collects metadata.
- A disk-usage analyzer can reuse traversal + aggregation to compute sizes per directory.
- Store per-scan aggregates (`dir_sizes`) in the binary store so "top usage" queries and TUI navigation do not rebuild directory totals on every run.
- Prefer leveraging the `index` scan to avoid duplicate filesystem walks; expose scan results and reuse them for analysis.
