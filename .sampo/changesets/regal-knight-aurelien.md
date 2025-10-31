---
cargo/sampo: patch
cargo/sampo-core: patch
cargo/sampo-github-action: patch
---

When the ecosystem allows it, `sampo publish` now performs a dry-run publish for each package, before proceeding with the actual publish. If any package fails the dry-run, the publish process is aborted, avoiding partial releases.
