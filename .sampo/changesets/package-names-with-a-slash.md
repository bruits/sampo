---
cargo/sampo: patch
cargo/sampo-core: patch
cargo/sampo-github-action: patch
---

Fixed package names containing a `/` (`@scope/name` for npm, `vendor/package` for Packagist, `groupId/artifactId` for Maven) failing with `unsupported package kind` when written without their ecosystem prefix in a changeset, a `packages.fixed` or `packages.linked` group, or the `-p` option of `sampo add` and `sampo pre`.
