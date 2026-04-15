---
cargo/sampo-core: patch
---

Fixes unnecessarily adding versions to path-only dev dependencies in Cargo projects, causing publish failures when the dev dependency was also getting a bump in the same release
