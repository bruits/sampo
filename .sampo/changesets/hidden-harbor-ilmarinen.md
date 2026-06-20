---
cargo/sampo-core: minor
cargo/sampo: minor
cargo/sampo-github-action: minor
---

In JavaScript/TypeScript (npm) projects, added support for private registries. `sampo publish` now authenticates its check for already-published versions with `NPM_TOKEN` or `NODE_AUTH_TOKEN`, falling back to your `.npmrc` when neither is set.
