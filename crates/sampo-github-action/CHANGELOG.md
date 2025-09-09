# sampo-github-action

## 0.5.0

### Minor changes

- [cc04060](https://github.com/bruits/sampo/commit/cc040606a59c5372116b203a17ce5e5e9692f133) Test — Thanks @goulvenclech!

### Patch changes

- Updated dependencies: sampo-core@0.3.0


## 0.4.0

### Minor changes

- [e511d28](https://github.com/bruits/sampo/commit/e511d28b15ef5aac0e07ef31ddc7112bdae9b64c) GitHub releases now use the matching changelog section as body. Optional Discussions creation is supported, with new `open-discussion` and `discussion-category` inputs. — Thanks @goulvenclech!

### Patch changes

- [81344d5](https://github.com/bruits/sampo/commit/81344d512b41d94b28d0dc62d8737e13b0384a8d) Restore detailed changelog for release PRs. — Thanks @goulvenclech!
- [81344d5](https://github.com/bruits/sampo/commit/81344d512b41d94b28d0dc62d8737e13b0384a8d) Fix unsolicited Cargo.toml formatting in release PRs. — Thanks @goulvenclech!


## 0.3.0

### Minor changes

- [0b3d77b](https://github.com/bruits/sampo/commit/0b3d77bd3c5096e44f459721b5d0b5ba6332705f) Add support for uploading binaries when creating to GitHub releases automatically. — Thanks @Princesseuh!

### Patch changes

- Updated dependencies: sampo-core@0.2.0


## 0.2.2

### Patch changes

- [6062083](https://github.com/bruits/sampo/commit/6062083ae20e3bcea6c1f4f00d6b58cf790cd9f1) Fix deploys and publishing. — Thanks @goulvenclech!
- Updated dependencies: sampo-core@0.1.1


## 0.2.1

### Patch changes

- [7397d24](https://github.com/bruits/sampo/commit/7397d24eb0276de3e8aaef6246a4c7b628cfa2a8) Fixed changelog enrichment not working. — Thanks Goulven Clec'h!
- Updated dependencies: sampo-core@0.1.0


## 0.2.0

### Minor changes

- [b68b1e1](https://github.com/bruits/sampo/commit/b68b1e1222355053b815a506365d25cacc6c1f2e) Release PR's changelogs now include clear explanations for automatic version bumps. For example: "Updated dependencies: name@x.x.x" for cascade bumps, and "Bumped due to fixed dependency group policy" for fixed group updates. — Thanks @goulvenclech!

## 0.1.0

### Minor changes

- [`d0d7244`](https://github.com/bruits/sampo/commit/d0d7244a43d76a0d7b377cf5f328a1fe244282b4) Changelog messages are now enriched with commit hash links and author thank-you notes, especially for first-time contributors. Added `[changelog]` configuration section with `show_commit_hash` and `show_acknowledgments` options (both default to true). — Thanks @goulvenclech!
- [`c7f252e`](https://github.com/bruits/sampo/commit/c7f252ef8c2671c3d35a3a69ab878591f024bf4a) Initial release of Sampo's GitHub Action, to help you automate release workflows in CI/CD pipelines. Supports multiple operation modes: `prepare-pr` (creates/updates Release PRs), `post-merge-publish` (publishes and tags after merge), and traditional `release`/`publish` commands. — Thanks @goulvenclech!

