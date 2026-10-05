---
cargo/sampo: patch
cargo/sampo-core: patch
cargo/sampo-github-action: patch
---

In npm projects, fixed publishing failing when a package has a non-publishable internal package in `devDependencies`. Previously these were treated as regular dependencies.
