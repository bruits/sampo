---
cargo/sampo: patch
cargo/sampo-core: patch
cargo/sampo-github-action: patch
---

In JavaScript/TypeScript (npm) projects, fixed `sampo publish` failing, or tagging a release that pnpm never published, on a release branch other than pnpm's `publish-branch` (`main` or `master` by default). Sampo now passes `--no-git-checks` to `pnpm publish`, so pnpm's branch, clean-tree, and up-to-date checks no longer run, whatever `.npmrc` or `pnpm-workspace.yaml` sets.
