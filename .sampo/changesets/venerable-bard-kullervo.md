---
cargo/sampo: patch
cargo/sampo-core: patch
cargo/sampo-github-action: patch
---

Private packages now receive git version tags during `sampo publish`, ensuring the GitHub Action's published output correctly triggers subsequent workflow steps, even for projects that don't publish to package registries.
