---
cargo/sampo-core: patch
---

Workspace discovery now locates the `.sampo/` directory instead of walking up to find manifests. New `find_sampo_root` and `discover_packages_at` functions are exported. Returns `NotInitialized` when `.sampo/` is missing, and `NoPackagesFound` when `.sampo/` exists but no packages are detected.
