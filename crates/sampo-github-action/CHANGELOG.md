# sampo-github-action

## 0.8.1 — 2025-10-03

### Patch changes

- [8ccbf32](https://github.com/bruits/sampo/commit/8ccbf3225a3ee005a958e592f909f547ec464b84) Fix GitHub Release bodies not being correctly filled when changelogs include release timestamps in their headings. — Thanks @goulvenclech!

## 0.8.0 — 2025-10-03

### Minor changes

- [74b94c6](https://github.com/bruits/sampo/commit/74b94c6623a6096bd501d1d8ae2c1b095bcc20fd) Can now mark GitHub releases as « pre-release » for pre-release branches. — Thanks @goulvenclech!
- [4afd1dd](https://github.com/bruits/sampo/commit/4afd1dddf5c5b0b318fc5d3ba94e2dce5d017802) Add automatic stabilize PR support: when a release plan targets pre-release versions the action now prepares a companion PR that exits pre-release mode, consumes preserved changesets, and readies the stable release branch. — Thanks @goulvenclech!
- [ee1cdaa](https://github.com/bruits/sampo/commit/ee1cdaad4672de0cbe62231e3c840f921414b312) Add a release timestamp in changelog headers (e.g., `## 1.0.0 - 2024-06-20`), with configuration options to toggle visibility, pick the format, or force a timezone. — Thanks @goulvenclech!
- [3e0f9ad](https://github.com/bruits/sampo/commit/3e0f9ad64f461aa03f00ebf29f2362583252bf49) While in pre-release mode, you can continue to add changesets and run `sampo release` and `sampo publish` as usual, Sampo preserves the consumed changesets in `.sampo/prerelease/`. When exiting pre-release mode or switching to a different label (for example, from `alpha` to `beta`), any preserved changesets are restored back to `.sampo/changesets/`, so the next release keeps the full history. — Thanks @goulvenclech!
- [fff8a3d](https://github.com/bruits/sampo/commit/fff8a3d2e23861878b05124449888414aac65e55) Update the release automation to create a dedicated pull request branch per release line (for example `release/main` and `release/3.x`). Each branch now has an independent PR title and force-pushed branch, so concurrent maintenance streams stay isolated while the action refreshes their release plans. — Thanks @goulvenclech!
- [fff8a3d](https://github.com/bruits/sampo/commit/fff8a3d2e23861878b05124449888414aac65e55) Add a `[git]` configuration section that defines the default release branch (default to `"main"`) and the full set of branch names allowed to run `sampo release` or `sampo publish`. The CLI and GitHub Action now detect the current branch (or respect `SAMPO_RELEASE_BRANCH`) and abort early when the branch is not whitelisted, enabling parallel maintenance lines such as `main` and `3.x` without cross-contamination. — Thanks @goulvenclech!
- [74b94c6](https://github.com/bruits/sampo/commit/74b94c6623a6096bd501d1d8ae2c1b095bcc20fd) Added support for pre-release identifiers such as `1.8.0-alpha` or `2.0.0-rc.3`. While a pre-release stays within its implied level (patch for `x.y.z-prerelease`, minor for `x.y.0-prerelease`, major for `x.0.0-prerelease`), we only bump the numeric suffix (`alpha` → `alpha.1` -> `alpha.2` -> etc). If a higher bump is required, we advance the base version first and reset the numeric suffix (`1.8.0-alpha.2` + major → `2.0.0-alpha`). Purely numeric tags like `1.0.0-1` are rejected. — Thanks @goulvenclech!

### Patch changes

- [a290985](https://github.com/bruits/sampo/commit/a29098505b3a93392b971995ffc601646e77f706) Fix release workflow so root `Cargo.toml` refreshes semver versions for member dependencies declared in `[workspace.dependencies]`, while leaving wildcard or path-only entries untouched. — Thanks @goulvenclech!
- [4f28531](https://github.com/bruits/sampo/commit/4f285311cc1cbbbdbeddbd36b2095affabff73e6) Stop including the redundant version header in GitHub release bodies. — Thanks @goulvenclech!
- [4afd1dd](https://github.com/bruits/sampo/commit/4afd1dddf5c5b0b318fc5d3ba94e2dce5d017802) When updating changelogs, Sampo now preserves any intro content or custom main header before the first `##` section. You can also manually edit previously released entries, and Sampo will keep them intact. — Thanks @goulvenclech!
- Updated dependencies: sampo-core@0.6.0

## 0.7.1

### Patch changes

- [6c431c4](https://github.com/bruits/sampo/commit/6c431c4a93c9195e7a9f0eee4e82b88d945a1a47) Releases now bump the right crate even if the package name is quoted. — Thanks @goulvenclech!
- Updated dependencies: sampo-core@0.5.0


## 0.7.0

### Minor changes

- [99ef058](https://github.com/bruits/sampo/commit/99ef0587da95359d82be7c1c3d1d454b192e68d1) **⚠️ breaking change:** Drop the legacy `prepare-pr`, `post-merge-publish`, and `release-and-publish` commands in favour of the unified `auto` flow and the explicit `release` / `publish` modes. This simplifies massively the configuration and usage, with only one workflow needed for both creating release PRs and publishing crates. See usage details in [crates/sampo-github-action/README.md](https://github.com/bruits/sampo/blob/main/crates/sampo-github-action/README.md). — Thanks @goulvenclech!


## 0.6.0

### Minor changes

- [786aa6e](https://github.com/bruits/sampo/commit/786aa6e7c4e84e49e7f1aa3013d8a2c844967466) Sampo Github Action's Github Releases now include prebuilt binaries for CLI crates on Linux, macOS, and Windows. Enable with `create-github-release: true` and `upload-binary: true` (library-only crates are skipped automatically). — Thanks @goulvenclech!


## 0.5.2

### Patch changes

- [0936318](https://github.com/bruits/sampo/commit/0936318b145d1265bf4a2e9128ce333336a0f7ff) Regenerate lockfiles at release, so it does not leave the repo dirty. — Thanks @goulvenclech!
- [0936318](https://github.com/bruits/sampo/commit/0936318b145d1265bf4a2e9128ce333336a0f7ff) Clearer errors when Discussions are disabled or the token lacks scope. — Thanks @goulvenclech!
- [0936318](https://github.com/bruits/sampo/commit/0936318b145d1265bf4a2e9128ce333336a0f7ff) Refactor error handling to improve context and consistency. — Thanks @goulvenclech!
- Updated dependencies: sampo-core@0.4.0


## 0.5.1

### Patch changes

- [061a5f3](https://github.com/bruits/sampo/commit/061a5f368f6409a868d94dc60f39f0fc1c138727) `packages.ignore` and `packages.ignore_unpublished` configuration options now work as intended for release and publishing steps. — Thanks @goulvenclech!
- Updated dependencies: sampo-core@0.3.1


## 0.5.0

### Minor changes

- [66a075b](https://github.com/bruits/sampo/commit/66a075b33aed9d7e00498c541b79fbb7fcf4eb09) ⚠️ **breaking change:** Rename dependent package options from `fixed_dependencies` and `linked_dependencies` to `fixed` and `linked`.
  
  ```diff
  // .sampo/config.toml
  [packages]
  -  fixed_dependencies = [["pkg-a", "pkg-b"], ["pkg-c", "pkg-d", "pkg-e"]]
  -  linked_dependencies = [["pkg-f", "pkg-g"]]
  +  fixed = [["pkg-a", "pkg-b"], ["pkg-c", "pkg-d", "pkg-e"]]
  +  linked = [["pkg-f", "pkg-g"]]
  ```
   — Thanks @goulvenclech!
- [3736d06](https://github.com/bruits/sampo/commit/3736d06afedfa80f09e635d15e0e32c141889a1d) Add support for ignoring packages during releases and in CLI package lists. You can now exclude unpublishable packages or specific packages by name/path patterns from Sampo operations.
  
  ```toml
  [packages]
  # Skip packages that aren't publishable to crates.io
  ignore_unpublished = true
  # Skip packages matching these patterns
  ignore = [
    "internal-*",     # Ignore by name pattern
    "examples/*",     # Ignore by workspace path
    "benchmarks/*"
  ]
  ```
   — Thanks @goulvenclech!

### Patch changes

- [b4a7ea6](https://github.com/bruits/sampo/commit/b4a7ea6c0bfb693ccbe77d0ffc6b72d540a164ff) Fixed a formatting issue in release notes when a block of code was followed immediately by the contributor acknowledgment text. — Thanks @goulvenclech!
- [b4a7ea6](https://github.com/bruits/sampo/commit/b4a7ea6c0bfb693ccbe77d0ffc6b72d540a164ff) Nesting should be preserved in release notes, even for nested lists. — Thanks @goulvenclech!
- Updated dependencies: sampo-core@0.3.0


## 0.4.2

### Patch changes

- Updated dependencies: sampo-core@0.2.1


## 0.4.1

### Patch changes

- [4bcb266](https://github.com/bruits/sampo/commit/4bcb266e847d0035a5ba4da17109237e74f82993) Fix "Problems parsing JSON" errors when creating GitHub pull requests, and overall better error handling when interacting with the GitHub API. — Thanks @goulvenclech!


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

