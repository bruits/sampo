---
cargo/sampo-core: minor
cargo/sampo: minor
cargo/sampo-github-action: minor
---

Added the `exclude_acknowledgments` option to the `[changelog]` section, listing GitHub usernames or Git author names that changelog entries should not thank.

```toml
[changelog]
exclude_acknowledgments = ["octocat", "The Octocat"]
```
