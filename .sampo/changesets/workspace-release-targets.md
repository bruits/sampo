---
cargo/sampo-core: patch
cargo/sampo: patch
cargo/sampo-github-action: patch
---

Fixed dependency pins and lockfile updates for packages that share a name across ecosystems. Cargo previews and releases stop before they write files if inherited versions conflict or a non-ignored crate outside the plan would inherit a new version.
