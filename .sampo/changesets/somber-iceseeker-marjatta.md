---
cargo/sampo-core: minor
cargo/sampo: minor
cargo/sampo-github-action: minor
---

Made git tag format configurable via new `tag_format` and `short_tags_format` options under `[git]`. Templates accept `{ecosystem}`, `{package_name}`, and `{version}`. Sampo refuses to publish when two packages would render to the same tag.

**⚠️ breaking change:** The default `tag_format` is now `{ecosystem}-{package_name}-v{version}` (was `{package_name}-v{version}`) so cross-ecosystem packages with the same name get distinct tags. To keep the previous tag shape, set:
```toml
[git]
tag_format = "{package_name}-v{version}"
```
