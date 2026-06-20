---
cargo/sampo-core: patch
cargo/sampo: patch
cargo/sampo-github-action: patch
---

In Cargo (Rust) projects, added support for alternative and private registries, and fixed crates that publish solely to an alternative registry (`publish = ["my-registry"]`) being wrongly marked as non-publishable. `sampo publish` now checks whether a version already exists on the target registry using your Cargo registry configuration and credentials.
