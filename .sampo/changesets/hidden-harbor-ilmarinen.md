---
cargo/sampo-core: patch
cargo/sampo: patch
cargo/sampo-github-action: patch
---

In JavaScript/TypeScript (npm) projects, added support for private registries. `sampo publish` now authenticates its check for already-published versions with `NPM_TOKEN` or `NODE_AUTH_TOKEN`.
