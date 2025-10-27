---
cargo/sampo: patch
cargo/sampo-core: patch
cargo/sampo-github-action: patch
---

Automatically add version field to workspace dependencies with only path during release. This fixes publish failures when a workspace dependency is declared with `path = "..."` but no `version` field, which is required by cargo publish.
