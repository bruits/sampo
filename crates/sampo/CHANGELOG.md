# sampo

## 0.9.0 — 2025-10-03

### Minor changes

- [7fe1d43](https://github.com/bruits/sampo/commit/7fe1d43da5aa3e809b5a4ab44900efdfbf474936) Introduce `sampo pre` command for pre-release management: run `pre enter` or `pre exit` with optional flags, or launch the fully interactive flow that captures the action, label, and affected packages before applying version updates. — Thanks @goulvenclech!
- [ee1cdaa](https://github.com/bruits/sampo/commit/ee1cdaad4672de0cbe62231e3c840f921414b312) Add a release timestamp in changelog headers (e.g., `## 1.0.0 - 2024-06-20`), with configuration options to toggle visibility, pick the format, or force a timezone. — Thanks @goulvenclech!
- [3e0f9ad](https://github.com/bruits/sampo/commit/3e0f9ad64f461aa03f00ebf29f2362583252bf49) While in pre-release mode, you can continue to add changesets and run `sampo release` and `sampo publish` as usual, Sampo preserves the consumed changesets in `.sampo/prerelease/`. When exiting pre-release mode or switching to a different label (for example, from `alpha` to `beta`), any preserved changesets are restored back to `.sampo/changesets/`, so the next release keeps the full history. — Thanks @goulvenclech!
- [fff8a3d](https://github.com/bruits/sampo/commit/fff8a3d2e23861878b05124449888414aac65e55) Add a `[git]` configuration section that defines the default release branch (default to `"main"`) and the full set of branch names allowed to run `sampo release` or `sampo publish`. The CLI and GitHub Action now detect the current branch (or respect `SAMPO_RELEASE_BRANCH`) and abort early when the branch is not whitelisted, enabling parallel maintenance lines such as `main` and `3.x` without cross-contamination. — Thanks @goulvenclech!
- [74b94c6](https://github.com/bruits/sampo/commit/74b94c6623a6096bd501d1d8ae2c1b095bcc20fd) Added support for pre-release identifiers such as `1.8.0-alpha` or `2.0.0-rc.3`. While a pre-release stays within its implied level (patch for `x.y.z-prerelease`, minor for `x.y.0-prerelease`, major for `x.0.0-prerelease`), we only bump the numeric suffix (`alpha` → `alpha.1` -> `alpha.2` -> etc). If a higher bump is required, we advance the base version first and reset the numeric suffix (`1.8.0-alpha.2` + major → `2.0.0-alpha`). Purely numeric tags like `1.0.0-1` are rejected. — Thanks @goulvenclech!

### Patch changes

- [a290985](https://github.com/bruits/sampo/commit/a29098505b3a93392b971995ffc601646e77f706) Fix release workflow so root `Cargo.toml` refreshes semver versions for member dependencies declared in `[workspace.dependencies]`, while leaving wildcard or path-only entries untouched. — Thanks @goulvenclech!
- [4afd1dd](https://github.com/bruits/sampo/commit/4afd1dddf5c5b0b318fc5d3ba94e2dce5d017802) When updating changelogs, Sampo now preserves any intro content or custom main header before the first `##` section. You can also manually edit previously released entries, and Sampo will keep them intact. — Thanks @goulvenclech!
- Updated dependencies: sampo-core@0.6.0

## 0.8.0

### Minor changes

- [46d5af6](https://github.com/bruits/sampo/commit/46d5af6fb22a312cf7175cc25e05675e64038343) Improve `sampo add` UX with interactive prompts for picking packages and assigning bump levels, plus an arrow-key-friendly message editor. — Thanks @goulvenclech!

### Patch changes

- [6c431c4](https://github.com/bruits/sampo/commit/6c431c4a93c9195e7a9f0eee4e82b88d945a1a47) Releases now bump the right crate even if the package name is quoted. — Thanks @goulvenclech!
- [6c431c4](https://github.com/bruits/sampo/commit/6c431c4a93c9195e7a9f0eee4e82b88d945a1a47) Sampo's CLI should not add quotes around package names in changesets. — Thanks @goulvenclech!
- Updated dependencies: sampo-core@0.5.0


## 0.7.0

### Minor changes

- [0936318](https://github.com/bruits/sampo/commit/0936318b145d1265bf4a2e9128ce333336a0f7ff) **⚠️ breaking change:** Sampo now uses standardized changeset format, thanks to [knope-dev/changesets](https://github.com/knope-dev/changesets) crate.
  
  ```md
  ---
  "package-a": minor
  ---
  
  Some description of the change.
  ```
   — Thanks @goulvenclech!

### Patch changes

- [0936318](https://github.com/bruits/sampo/commit/0936318b145d1265bf4a2e9128ce333336a0f7ff) Regenerate lockfiles at release, so it does not leave the repo dirty. — Thanks @goulvenclech!
- [0936318](https://github.com/bruits/sampo/commit/0936318b145d1265bf4a2e9128ce333336a0f7ff) Refactor error handling to improve context and consistency. — Thanks @goulvenclech!
- Updated dependencies: sampo-core@0.4.0


## 0.6.1

### Patch changes

- [061a5f3](https://github.com/bruits/sampo/commit/061a5f368f6409a868d94dc60f39f0fc1c138727) `packages.ignore` and `packages.ignore_unpublished` configuration options now work as intended for release and publishing steps. — Thanks @goulvenclech!
- Updated dependencies: sampo-core@0.3.1


## 0.6.0

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

- [37b006b](https://github.com/bruits/sampo/commit/37b006b96d6bc78d5a9cda661d8b28fa5d0fcd0c) `sampo init` now generates a more up-to-date configuration file and README snippet. — Thanks @goulvenclech!
- [b4a7ea6](https://github.com/bruits/sampo/commit/b4a7ea6c0bfb693ccbe77d0ffc6b72d540a164ff) Fixed a formatting issue in release notes when a block of code was followed immediately by the contributor acknowledgment text. — Thanks @goulvenclech!
- [b4a7ea6](https://github.com/bruits/sampo/commit/b4a7ea6c0bfb693ccbe77d0ffc6b72d540a164ff) Nesting should be preserved in release notes, even for nested lists. — Thanks @goulvenclech!
- Updated dependencies: sampo-core@0.3.0


## 0.5.1

### Patch changes

- Updated dependencies: sampo-core@0.2.1


## 0.5.0

### Patch changes

- Updated dependencies: sampo-core@0.2.0


## 0.4.1

### Patch changes

- [6062083](https://github.com/bruits/sampo/commit/6062083ae20e3bcea6c1f4f00d6b58cf790cd9f1) Fix deploys and publishing. — Thanks @goulvenclech!
- Updated dependencies: sampo-core@0.1.1


## 0.4.0

### Patch changes

- [7397d24](https://github.com/bruits/sampo/commit/7397d24eb0276de3e8aaef6246a4c7b628cfa2a8) Fixed changelog enrichment not working. — Thanks Goulven Clec'h!
- Updated dependencies: sampo-core@0.1.0


## 0.3.0

### Minor changes

- [b68b1e1](https://github.com/bruits/sampo/commit/b68b1e1222355053b815a506365d25cacc6c1f2e) Sampo can now detect packages within the same repository that depend on each other, and automatically manages their versions.

  - By default, dependent packages are automatically patched when an internal dependency is updated. For example: if `a@0.1.0` depends on `b@0.1.0` and `b` is updated to `0.2.0`, then `a` will be automatically bumped to `0.1.1` (patch). If `a` needs a major or minor change due to `b`'s update, it should be explicitly specified in a changeset.
  - **Fixed dependencies** always bump together with the same version, even if not directly affected. For example: if `a@1.0.0` and `b@1.0.0` are in a fixed group and `b` is updated to `2.0.0`, then `a` will also be bumped to `2.0.0`.
  - **Linked dependencies** apply the highest bump level to affected packages and their dependents. For example: if `a@1.0.0` depends on `b@1.0.0` in a linked group and `b` is updated to `2.0.0` (major), then `a` will also be bumped to `2.0.0`. If `a` is later updated to `2.1.0` (minor), `b` remains at `2.0.0` since it's not affected. Finally, if `b` has a patch update, both `a` and `b` will be bumped with patch level (the highest in the group).

  Changelogs now include clear explanations for automatic version bumps. For example: "Updated dependencies: name@x.x.x" for cascade bumps, and "Bumped due to fixed dependency group policy" for fixed group updates. — Thanks @goulvenclech!


## 0.2.0

### Minor changes

- [`d0d7244`](https://github.com/bruits/sampo/commit/d0d7244a43d76a0d7b377cf5f328a1fe244282b4) Changelog messages are now enriched with commit hash links and author thank-you notes, especially for first-time contributors. Added `[changelog]` configuration section with `show_commit_hash` and `show_acknowledgments` options (both default to true). — Thanks @goulvenclech!

### Patch changes

- [`c7f252e`](https://github.com/bruits/sampo/commit/c7f252ef8c2671c3d35a3a69ab878591f024bf4a) Changed release and publish semantics: `sampo release` no longer creates git tags and can be run multiple times to iteratively prepare a release. Git tags are now created by `sampo publish` after successful crate publication. This allows for better separation between release preparation and finalization workflows. This is technically a **breaking change** ⚠️ but this is expected in 0.x.x alpha versions. — Thanks @goulvenclech!
- [`978bbe4`](https://github.com/bruits/sampo/commit/978bbe4e78205a685a6f92ae0412f8d5e65c3259) Only publish crates with a release tag (`name-v<version>`) and skip versions already on crates.io. Prevents “crate already exists” errors and avoids publishing unplanned crates. — Thanks @goulvenclech!


## 0.1.0

### Minor changes

- Initial release of Sampo CLI, a tool to automate changelogs, versioning, and publishing. This first version includes proof of concepts for the main commands (`help`, `init`, `add`, `release`, `publish`), and has been published using Sampo itself!

