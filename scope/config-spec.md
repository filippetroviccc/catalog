## Config and Preset Specification

This document defines the config file format, defaults, validation, and presets.

---

## Locations

- Default config path: `~/Library/Application Support/catalog/config.toml`
- Default store path: `~/Library/Application Support/catalog/catalog.bin`
- Env overrides:
  - `CATALOG_CONFIG` overrides config path
  - `CATALOG_STORE` overrides store path (legacy `CATALOG_DB` also accepted)

---

## Config Schema (TOML)

```toml
version = 1
output = "plain"
include_hidden = false
one_filesystem = true

roots = [
  "~/Downloads",
  "~/Desktop"
]

excludes = [
  "**/.git/**",
  "**/node_modules/**",
  "**/target/**",
  "**/dist/**",
  "**/build/**"
]
```

---

## Validation Rules

- `version` must be an integer.
- `output` must be `plain` or `json`.
- `roots` must be a non-empty list of strings for indexing.
- `excludes` must be a list of strings.
- Invalid config values should be rejected with a clear error.

---

## Presets

### `macos-user-additions`

Included roots if present:

- `~/Downloads`
- `~/Desktop`
- `~/Documents`
- `~/Developer` or `~/Projects`
- `~/Library/Mobile Documents`
- `/Applications`
- `~/Applications`
- `/opt/homebrew`
- `/usr/local`
- `~/bin`
- `~/.local/bin`
- `~/.config`
- `~/Library/Preferences`
- `~/Library/LaunchAgents`

### `macos-deep`

Includes all `macos-user-additions` roots plus:

- `/Library/LaunchAgents`
- `/Library/LaunchDaemons`
- `/Library/Fonts`
- `~/Library/Fonts`
- `/Library/PreferencePanes`
- `~/Library/PreferencePanes`
- `/etc`

---

## Default Excludes

- `~/Library/Caches`
- `~/Library/Containers`
- `~/Library/Logs`
- `~/Library/Developer/Xcode/DerivedData`
- `**/.git/**`
- `**/node_modules/**`
- `**/target/**`
- `**/dist/**`
- `**/build/**`
