---
cargo/sampo: minor
cargo/sampo-core: minor
cargo/sampo-github-action: minor
---

Introducing `changesets.tags` configuration option, an optional array of custom changelog section names (default: `[]`). When configured, changesets can use the `bump (Tag)` format to categorize entries under custom headings instead of the default bump-based sections. For example, `tags = ["Added", "Changed", "Deprecated", "Removed", "Fixed", "Security"]` enables [Keep a Changelog](https://keepachangelog.com/) style formatting where `cargo/my-crate: minor (Added)` appears under `### Added` while still applying a minor version bump.
