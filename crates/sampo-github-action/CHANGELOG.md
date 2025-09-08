# sampo-github-action

## 0.2.0

### Minor changes

- Release PR's changelogs now include clear explanations for automatic version bumps. For example: "Updated dependencies: name@x.x.x" for cascade bumps, and "Bumped due to fixed dependency group policy" for fixed group updates.

## 0.1.0

### Minor changes

- [`d0d7244`](https://github.com/bruits/sampo/commit/d0d7244a43d76a0d7b377cf5f328a1fe244282b4) Changelog messages are now enriched with commit hash links and author thank-you notes, especially for first-time contributors. Added `[changelog]` configuration section with `show_commit_hash` and `show_acknowledgments` options (both default to true). — Thanks @goulvenclech!
- [`c7f252e`](https://github.com/bruits/sampo/commit/c7f252ef8c2671c3d35a3a69ab878591f024bf4a) Initial release of Sampo's GitHub Action, to help you automate release workflows in CI/CD pipelines. Supports multiple operation modes: `prepare-pr` (creates/updates Release PRs), `post-merge-publish` (publishes and tags after merge), and traditional `release`/`publish` commands. — Thanks @goulvenclech!

