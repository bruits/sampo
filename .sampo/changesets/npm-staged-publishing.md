---
cargo/sampo-core: minor
cargo/sampo: minor
cargo/sampo-github-action: minor
---

In JavaScript/TypeScript (npm) projects, added the `npm_stage` option to the `[packages]` section. `sampo publish` then stages the selected npm packages through npm staged publishing, and each version becomes installable once a maintainer approves it.

```toml
[packages]
npm_stage = true # or a list, such as ["npm/web"]
```
