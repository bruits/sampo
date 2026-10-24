---
cargo/sampo: patch
cargo/sampo-core: patch
cargo/sampo-github-action: patch
---

In JavaScript/TypeScript (npm) projects, fixed `sampo publish` failing, or tagging a release that pnpm never published, when run off pnpm's `publish-branch` (`main` or `master` by default). Sampo now passes `--no-git-checks` to `pnpm publish`, which also skips pnpm's clean-working-tree and up-to-date-with-remote checks.
