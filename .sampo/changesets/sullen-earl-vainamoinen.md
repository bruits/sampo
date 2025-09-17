---
sampo-github-action: major
---

**⚠️ breaking change:** Drop the legacy `prepare-pr`, `post-merge-publish`, and `release-and-publish` commands in favour of the unified `auto` flow and the explicit `release` / `publish` modes. This simplifies massively the configuration and usage, with only one workflow needed for both creating release PRs and publishing crates. See usage details in [crates/sampo-github-action/README.md](https://github.com/bruits/sampo/blob/main/crates/sampo-github-action/README.md).
