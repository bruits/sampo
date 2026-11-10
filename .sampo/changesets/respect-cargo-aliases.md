---
cargo/sampo: patch
cargo/sampo-core: patch
cargo/sampo-github-action: patch
---

In Rust (Cargo) projects, fixed `sampo release` not bumping crates that depend on a workspace crate through a renamed dependency (`package = "…"`), directly or through `[workspace.dependencies]`, and leaving those renamed workspace entries on the old version.
