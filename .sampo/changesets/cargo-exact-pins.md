---
cargo/sampo: patch
cargo/sampo-core: patch
cargo/sampo-github-action: patch
---

In Rust (Cargo) projects, fixed `sampo release` and `sampo pre` failing at `Cargo.lock` regeneration, with the new versions already written, when an internal dependency is pinned with an exact requirement: `=1.0.1` is now rewritten to the released version, in crates and in `[workspace.dependencies]`. Such a pin is also no longer reported as an unsatisfied constraint: no warning (dry run included), and no constraint violation in `fixed` or `linked` groups.
