---
cargo/sampo: patch
cargo/sampo-core: patch
cargo/sampo-github-action: patch
---

Private packages now receive git version tags during `sampo publish`, ensuring the GitHub Action's published output correctly triggers subsequent workflow steps, even for projects that don't publish to package registries.

Fixed a critical bug where partial publish failures would prevent git tags from being created for successfully published packages. Tags are now created immediately after each successful publish, ensuring accurate version tracking and proper GitHub Action output reporting even in case of mid-process failures.
