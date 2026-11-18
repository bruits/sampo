---
cargo/sampo-core: patch
cargo/sampo: patch
cargo/sampo-github-action: patch
---

Fixed dependency pins and lockfile updates for packages that share a name across ecosystems. In Cargo workspaces, release previews and releases now refuse conflicting or unplanned changes to inherited versions before they write files.
