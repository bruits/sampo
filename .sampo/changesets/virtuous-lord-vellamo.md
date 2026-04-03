---
cargo/sampo-core: patch
cargo/sampo-github-action: patch
---

Fixed incorrect stable version computation when stabilizing a prerelease package (e.g. `0.2.7-alpha.6` + patch now correctly produces `0.2.7` instead of `0.2.8`), and fixed the stabilize PR not being created after merging a prerelease PR when all preserved changesets target packages still in prerelease.
