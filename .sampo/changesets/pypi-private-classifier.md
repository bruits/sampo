---
cargo/sampo: minor
cargo/sampo-core: minor
---

In Python (PyPI) projects, added support for the `Private :: Do Not Upload` classifier (any classifier starting with `Private ::`). Packages declaring one are now treated as not publishable, like `"private": true` in npm or `publish = false` in Cargo: `sampo publish` still tags them but no longer builds and uploads them, and `ignore_unpublished` applies to them.
