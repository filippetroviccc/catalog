## Packaging and Distribution

This document defines MVP packaging and release expectations.

---

## Build and Install

- From source: `cargo install --path .`.
- Homebrew tap is supported for macOS Apple Silicon.

---

## Versioning

- Use semver.
- Increment minor for new commands or flags.
- Increment patch for bug fixes.

---

## Release Checklist

- Update version in Cargo.toml.
- Update changelog.
- Run tests.
- Tag release.
