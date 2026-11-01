---
cargo/sampo-core: minor
cargo/sampo: minor
cargo/sampo-github-action: minor
---

**Ruby packages are now supported!** Sampo now automatically detects Ruby gems (via their gemspec, including a version kept in `lib/<gem>/version.rb`, for publishing to RubyGems) and handles versioning, changelogs, and publishing—even in mixed workspaces. Gems whose version is computed (e.g. read from a `VERSION` file) are skipped with a warning.
