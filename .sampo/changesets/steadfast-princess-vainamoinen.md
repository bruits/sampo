---
cargo/sampo: minor
cargo/sampo-core: minor
cargo/sampo-github-action: minor
---

**⚠️ breaking change:** Add infrastructure for validating dependency version constraints during release planning. This enables early detection of constraint violations before file modifications, with errors for fixed/linked packages and warnings for others.

**⚠️ Warning:** Ecosystem-specific constraint parsing and validation logic is not yet implemented; current methods return "Skipped" stubs.
