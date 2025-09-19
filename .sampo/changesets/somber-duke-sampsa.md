---
sampo-github-action: minor
---

Update the release automation to create a dedicated pull request branch per release line (for example `release/main` and `release/3.x`). Each branch now has an independent PR title and force-pushed branch, so concurrent maintenance streams stay isolated while the action refreshes their release plans.
