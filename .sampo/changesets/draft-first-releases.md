---
cargo/sampo-github-action: minor
---

Added support for GitHub's immutable releases to the GitHub Action's `release-assets` input. A release with assets is now published from a draft, so workflows on `release: created` no longer run for it and should listen for `release: published`.
