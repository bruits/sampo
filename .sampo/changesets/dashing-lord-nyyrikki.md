---
cargo/sampo: patch
cargo/sampo-core: patch
cargo/sampo-github-action: patch
---

In Cargo (Rust) projects, fixed a bug overwriting `version.workspace = true` in member crates. Sampo now preserves workspace version inheritance, and updates the root manifest's `[workspace.package].version` and `[workspace.dependencies]` correctly.
