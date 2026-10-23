---
cargo/sampo-core: minor
cargo/sampo: minor
cargo/sampo-github-action: minor
---

Added the `bump_dependents` option to the `[packages]` section. With `"out-of-range"`, a release only bumps the dependents whose requirement no longer covers the new version, instead of every dependent.

```toml
[packages]
bump_dependents = "out-of-range"
```
