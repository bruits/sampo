---
cargo/sampo: minor
cargo/sampo-core: minor
cargo/sampo-github-action: minor
---

Added ecosystem-specific `--*-args` inputs to the CLI and `*-args` inputs to the GitHub Action (e.g. `--cargo-args` or `npm-args`), allowing users to forward extra arguments to specific package managers. The existing `-- <args>` syntax continues to forward arguments to all ecosystems.
