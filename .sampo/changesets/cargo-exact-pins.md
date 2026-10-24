---
cargo/sampo: patch
cargo/sampo-core: patch
cargo/sampo-github-action: patch
---

In Rust (Cargo) projects, fixed `sampo release` and `sampo pre` failing on internal dependencies pinned with `=x.y.z`. Such pins now follow the released version, like bare pins, and no longer raise constraint warnings or violations.
