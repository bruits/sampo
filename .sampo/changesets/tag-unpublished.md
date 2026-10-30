---
cargo/sampo-core: minor
cargo/sampo: minor
cargo/sampo-github-action: minor
---

Added the `tag_unpublished` option to the `[git]` section, which sets the unpublishable packages `sampo publish` tags: all by default, none with `false`, or those matching a list of patterns.

```toml
[git]
tag_unpublished = ["cargo/my-cli"]
```
