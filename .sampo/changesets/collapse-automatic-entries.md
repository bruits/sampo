---
cargo/sampo-core: minor
cargo/sampo: minor
cargo/sampo-github-action: minor
---

Added the `automatic_entries` option to the `[changelog]` section. With `"collapse"`, changelogs drop the entries Sampo writes for packages bumped by other packages (such as "Updated dependencies"), consecutive releases with nothing else share one section, and the release pull request folds those packages into a collapsed block.

```toml
[changelog]
automatic_entries = "collapse"
```
