---
cargo/sampo: minor
cargo/sampo-core: minor
---

Added `--cargo-args`, `--npm-args`, `--hex-args`, `--pypi-args`, and `--packagist-args` flags to `sampo publish` to forward extra arguments to specific package managers. The existing `-- <args>` syntax continues to forward arguments to all ecosystems.
