---
cargo/sampo-github-action: patch
---

Fixed version pinning in the GitHub Action: the installed binary now matches the action tag instead of always pulling the latest version from crates.io. Fixed silent failure when uploading release assets to an existing GitHub release. Improved boolean input compatibility for `dry-run` and `create-github-release` options.
