---
cargo/sampo: patch
cargo/sampo-core: patch
cargo/sampo-github-action: patch
---

Publish command now checks registry versions before running dry-run validation, skipping unnecessary compilation when packages are already published.
