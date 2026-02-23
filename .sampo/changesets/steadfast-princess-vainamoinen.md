---
cargo/sampo: minor
cargo/sampo-core: minor
cargo/sampo-github-action: minor
---

**⚠️ breaking change:** Sampo no longer overwrites range constraints for internal dependencies (or skips them silently in some cases). During release planning, if a planned version bump doesn't satisfy a range constraint (e.g. bumping `foo` to `2.0.0` when another package requires `foo = "^1.0"`), you'll get either an error (for packages in `fixed` or `linked` groups) or a warning, instead of silently skipping. Pinned versions (e.g. `foo = "1.2.3"`) are still bumped automatically.

**Note:** Constraint validation is currently implemented for Cargo and npm packages. Other ecosystems (Hex, PyPI, Packagist) will skip validation with an informative message until support is added.
