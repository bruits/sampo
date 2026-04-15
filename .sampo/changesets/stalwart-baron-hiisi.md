---
cargo/sampo: patch
cargo/sampo-core: patch
cargo/sampo-github-action: patch
---

In Cargo projects, fixed unnecessarily adding versions to path-only dev dependencies, which caused publish failures when the dev dependency was also bumped in the same release.
