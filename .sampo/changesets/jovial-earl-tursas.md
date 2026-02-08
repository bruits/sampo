---
cargo/sampo-core: patch
---

Fixed prerelease guard in release to only skip preserved changesets when all referenced packages are in prerelease, not when any workspace member is.
