---
cargo/sampo: patch
cargo/sampo-core: patch
cargo/sampo-github-action: patch
---

In Rust (Cargo) projects, fixed `sampo release` and `sampo pre` upgrading every dependency in `Cargo.lock`: they now only update the workspace's own crates.
