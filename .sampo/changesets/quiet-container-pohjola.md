---
cargo/sampo-core: patch
cargo/sampo: patch
cargo/sampo-github-action: patch
---

Fixed `sampo publish` creating a spurious git tag and GitHub release for non-publishable, versionless packages (workspace containers and other packages marked as non-publishable without a declared version). Non-publishable but versioned packages are still tagged as before.
