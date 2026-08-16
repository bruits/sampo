---
cargo/sampo: patch
cargo/sampo-core: patch
cargo/sampo-github-action: patch
---

In JavaScript/TypeScript (npm) projects, fixed a dry-run publish that published for real on Yarn Classic, and publishing on Yarn Berry, which never reached the registry. Where yarn cannot simulate a publish, the dry run is now skipped with a warning.
