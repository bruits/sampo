---
cargo/sampo: patch
cargo/sampo-core: patch
cargo/sampo-github-action: patch
---

Fixed the GitHub Action never tagging nor releasing the packages published before a failed publish, even on a rerun. It now pushes their tags and creates their GitHub Releases, then fails.
