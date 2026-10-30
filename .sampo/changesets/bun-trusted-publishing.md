---
cargo/sampo-core: minor
cargo/sampo: minor
cargo/sampo-github-action: minor
---

In JavaScript/TypeScript (npm) projects, added npm trusted publishing to `sampo publish` in Bun workspaces: in CI with no token, packages publish through npm 11.5.1 or later instead of `bun publish`, which cannot use it.
