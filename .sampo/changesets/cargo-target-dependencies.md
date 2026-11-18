---
cargo/sampo: patch
cargo/sampo-core: patch
cargo/sampo-github-action: patch
---

In Rust (Cargo) projects, fixed `sampo publish` to order crates after their target-specific normal and build dependencies in the workspace. `sampo release` checks ranges on those dependencies and includes dev dependencies with an explicit `version` when it determines version bumps.
