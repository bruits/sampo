---
cargo/sampo-core: minor
cargo/sampo: minor
cargo/sampo-github-action: minor
---

Added the `show_automatic_entries` option to the `[changelog]` section. Set it to `false` to leave out the entries Sampo writes for packages bumped without a changeset ("Updated dependencies: ...", "Bumped due to fixed dependency group policy"): those packages are still bumped, but get no changelog section when nothing else changed, and the release pull request lists them in a collapsed block.

```toml
[changelog]
show_automatic_entries = false
```
