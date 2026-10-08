---
cargo/sampo: patch
cargo/sampo-core: patch
cargo/sampo-github-action: patch
---

In JavaScript/TypeScript (npm) projects, fixed `sampo publish` failing when a package's `devDependencies` include a non-publishable workspace package or close a dependency cycle.
