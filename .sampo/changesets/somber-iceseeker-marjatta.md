---
cargo/sampo-core: minor
cargo/sampo: minor
cargo/sampo-github-action: minor
---

Made git tag format configurable via new `tag_format` and `short_tags_format` options under `[git]`. Templates accept `{ecosystem}`, `{package_name}`, and `{version}`.

`sampo publish` now also detects cross-ecosystem tag conflicts: it errors when two packages would produce the same git tag for the release in flight, and warns when packages share a name across ecosystems with a template that doesn't include `{ecosystem}` (a future version bump would silently collide). Both diagnostics suggest setting `tag_format = "{ecosystem}-{package_name}-v{version}"` as the fix.
