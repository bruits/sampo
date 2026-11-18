---
cargo/sampo: patch
cargo/sampo-core: patch
cargo/sampo-github-action: patch
---

In Rust (Cargo) projects, fixed the publish order for crates whose workspace dependencies appear only under `[target.*]`. `sampo release` now checks these dependencies when it validates version ranges and bumps dependents.
