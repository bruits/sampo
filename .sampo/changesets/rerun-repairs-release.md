---
cargo/sampo: patch
cargo/sampo-core: patch
cargo/sampo-github-action: patch
---

Fixed a GitHub Action release that failed after publishing leaving tags and GitHub Releases missing, which rerunning could not repair. The action now pushes and releases everything it can before failing, also when an asset upload fails, and a rerun adds whatever is still missing.
