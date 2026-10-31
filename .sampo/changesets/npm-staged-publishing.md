---
cargo/sampo-core: minor
cargo/sampo: minor
cargo/sampo-github-action: minor
---

In JavaScript/TypeScript (npm) projects, added the `stage` option to a new `[npm]` section. `sampo publish` then stages npm packages through npm staged publishing, and each version becomes installable once a maintainer approves it.

```toml
[npm]
stage = true
```
