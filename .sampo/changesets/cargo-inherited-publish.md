---
cargo/sampo: patch
cargo/sampo-core: patch
cargo/sampo-github-action: patch
---

In Rust (Cargo) projects, fixed `sampo publish` and `ignore_unpublished` ignoring `publish.workspace = true`: crates inheriting `publish = false` are now skipped, and crates inheriting an alternative registry are checked against it instead of crates.io.
