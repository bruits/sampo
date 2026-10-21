---
cargo/sampo: patch
cargo/sampo-core: patch
cargo/sampo-github-action: patch
---

In Rust (Cargo) projects, fixed crates using `publish.workspace = true` being treated as publishable: Sampo now reads `publish` from the workspace root's `[workspace.package]`, so `sampo publish` and `ignore_unpublished` skip crates that inherit `publish = false`.
