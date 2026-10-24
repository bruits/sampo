---
cargo/sampo: patch
cargo/sampo-core: patch
cargo/sampo-github-action: patch
cargo/sampo-github-bot: patch
---

Fixed package names containing a `/` (e.g. `@scope/name` in npm, `vendor/package` in Packagist) failing with `unsupported package kind` when written without their ecosystem prefix, in changesets, `packages.fixed` and `packages.linked` groups, and `sampo add` or `sampo pre` with `-p`.
