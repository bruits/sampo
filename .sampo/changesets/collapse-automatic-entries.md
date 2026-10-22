---
cargo/sampo-core: minor
cargo/sampo: minor
cargo/sampo-github-action: minor
---

Added the `automatic_entries` option to the `[changelog]` section. Set it to `"collapse"` to drop the entries Sampo writes for packages bumped by other packages ("Updated dependencies: ...", "Bumped due to fixed dependency group policy"), and give consecutive releases with nothing else one changelog section, whose heading spans their versions (`## 1.0.1 ... 1.1.0`).

```toml
[changelog]
automatic_entries = "collapse"
```
