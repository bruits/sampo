---
cargo/sampo: patch
cargo/sampo-core: patch
cargo/sampo-github-action: patch
---

Fixed `sampo release` bumping a package without a `version` from `0.0.0` when it depends on a released package or shares its `fixed` group. Such a package is now left out of the release, instead of failing it or getting a changelog entry for a version it never holds (Cargo crates still get their first version).

In JavaScript/TypeScript (npm) projects, fixed `sampo release` failing after writing some manifests when a changeset names a package without a `version`: it now fails before anything is written, and `--dry-run` reports the same error.
