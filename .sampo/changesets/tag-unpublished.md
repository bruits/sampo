---
cargo/sampo-core: minor
cargo/sampo: minor
cargo/sampo-github-action: minor
---

Added the `tag_unpublished` option to the `[git]` section. `sampo publish` tags packages that are not publishable by default; set it to `false` to tag only publishable packages, or to a list of patterns to tag only the unpublishable packages matching one of them.

```toml
[git]
tag_unpublished = ["cargo/my-cli"]
```
