---
cargo/sampo-core: minor
---

**⚠️ breaking change:** Removed `ManifestMetadata::get_dependency_constraint` and `ManifestMetadata::is_dependency_pinned` from `sampo_core::adapters::cargo`, as Sampo no longer uses them.
