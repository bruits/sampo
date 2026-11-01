---
cargo/sampo: patch
cargo/sampo-core: patch
cargo/sampo-github-action: patch
---

Fixed the GitHub Action losing the tags and GitHub Releases of packages published before a failed publish. It now pushes them and creates their releases, then fails.
