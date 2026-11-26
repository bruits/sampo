---
cargo/sampo: patch
cargo/sampo-core: patch
cargo/sampo-github-action: patch
---

In Rust (Cargo) projects, fixed `sampo publish` publishing a crate before a workspace crate it depends on only under a `[target.'cfg(…)']` table, and `sampo release` not bumping crates for those dependencies or for dev-dependencies inherited from `[workspace.dependencies]`. `sampo release` also no longer reports a range violation for a path-only dependency on a prerelease, or for a `[workspace.dependencies]` requirement it rewrites.
