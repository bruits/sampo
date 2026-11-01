---
cargo/sampo: patch
cargo/sampo-core: patch
cargo/sampo-github-action: patch
---

Fixed `sampo publish` tagging a package that got `--dry-run` through the forwarded arguments (e.g. `--npm-args --dry-run`), which published nothing.
