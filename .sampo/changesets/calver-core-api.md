---
cargo/sampo-core: minor
---

**⚠️ breaking change:** Changed `detect_all_dependency_explanations` to accept borrowed `ReleasedPackage` records keyed by canonical identifier. Pass only canonical version targets to `PackageAdapter::finalize_workspace_roots`.

Set `versioning` and `calver_year_month` in `Config` literals, or use `..Config::default()`.
