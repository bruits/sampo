# sampo-core

## 0.9.0 — 2025-10-23

### Minor changes

- [3703dfa](https://github.com/bruits/sampo/commit/3703dfa5c93190ca46d60e1a9e4a96e180f0c3d2) **Elixir packages are now supported!** Sampo now automatically detects Hex packages managed by `mix` (Elixir), and handles versioning, changelogs, and publishing—even in mixed workspaces. — Thanks @goulvenclech!

### Patch changes

- [a83904b](https://github.com/bruits/sampo/commit/a83904bf59f25291b78b466335baec28d8044c94) Npm packages marked `private: true` no longer block Sampo publish when their manifest omits `version`. — Thanks @goulvenclech!

## 0.8.0 — 2025-10-16

### Minor changes

- [8d4e0e0](https://github.com/bruits/sampo/commit/8d4e0e0b4f076d0679525480539f3cfc170f0ede) **npm packages are now supported!** Sampo now automatically detects npm packages, and handles versioning, changelogs, and publishing—even in mixed Rust/JS workspaces. — Thanks @goulvenclech!

## 0.7.0 — 2025-10-12

### Minor changes

- [a4bcf23](https://github.com/bruits/sampo/commit/a4bcf230586f6643dd5a75f8c4fe38b0c70b2905) To avoid ambiguity between packages in different ecosystems (e.g. a Rust crate and an npm package both named `example`), Sampo now assigns canonical identifiers to all packages using `<ecosystem>/<name>`, such as `cargo/example` for Rust crates or `npm/example` for JavaScript packages.
  
  Changesets, the CLI, and the GitHub Action now accept and emit ecosystem-qualified package names. Plain package names (without ecosystem prefix) are still supported when there is no ambiguity. — Thanks @goulvenclech!

## 0.6.0 — 2025-10-03

### Minor changes

- [7fe1d43](https://github.com/bruits/sampo/commit/7fe1d43da5aa3e809b5a4ab44900efdfbf474936) Add prerelease management helpers that toggle Cargo manifests between stable and labeled versions, validate custom identifiers, update internal dependency requirements, and regenerate `Cargo.lock` with dedicated error reporting. — Thanks @goulvenclech!
- [ee1cdaa](https://github.com/bruits/sampo/commit/ee1cdaad4672de0cbe62231e3c840f921414b312) Add a release timestamp in changelog headers (e.g., `## 1.0.0 - 2024-06-20`), with configuration options to toggle visibility, pick the format, or force a timezone. — Thanks @goulvenclech!
- [3e0f9ad](https://github.com/bruits/sampo/commit/3e0f9ad64f461aa03f00ebf29f2362583252bf49) While in pre-release mode, you can continue to add changesets and run `sampo release` and `sampo publish` as usual, Sampo preserves the consumed changesets in `.sampo/prerelease/`. When exiting pre-release mode or switching to a different label (for example, from `alpha` to `beta`), any preserved changesets are restored back to `.sampo/changesets/`, so the next release keeps the full history. — Thanks @goulvenclech!
- [fff8a3d](https://github.com/bruits/sampo/commit/fff8a3d2e23861878b05124449888414aac65e55) Add a `[git]` configuration section that defines the default release branch (default to `"main"`) and the full set of branch names allowed to run `sampo release` or `sampo publish`. The CLI and GitHub Action now detect the current branch (or respect `SAMPO_RELEASE_BRANCH`) and abort early when the branch is not whitelisted, enabling parallel maintenance lines such as `main` and `3.x` without cross-contamination. — Thanks @goulvenclech!
- [74b94c6](https://github.com/bruits/sampo/commit/74b94c6623a6096bd501d1d8ae2c1b095bcc20fd) Added support for pre-release identifiers such as `1.8.0-alpha` or `2.0.0-rc.3`. While a pre-release stays within its implied level (patch for `x.y.z-prerelease`, minor for `x.y.0-prerelease`, major for `x.0.0-prerelease`), we only bump the numeric suffix (`alpha` → `alpha.1` -> `alpha.2` -> etc). If a higher bump is required, we advance the base version first and reset the numeric suffix (`1.8.0-alpha.2` + major → `2.0.0-alpha`). Purely numeric tags like `1.0.0-1` are rejected. — Thanks @goulvenclech!

### Patch changes

- [a290985](https://github.com/bruits/sampo/commit/a29098505b3a93392b971995ffc601646e77f706) Fix release workflow so root `Cargo.toml` refreshes semver versions for member dependencies declared in `[workspace.dependencies]`, while leaving wildcard or path-only entries untouched. — Thanks @goulvenclech!
- [4afd1dd](https://github.com/bruits/sampo/commit/4afd1dddf5c5b0b318fc5d3ba94e2dce5d017802) When updating changelogs, Sampo now preserves any intro content or custom main header before the first `##` section. You can also manually edit previously released entries, and Sampo will keep them intact. — Thanks @goulvenclech!

## 0.5.0

### Minor changes

- [46d5af6](https://github.com/bruits/sampo/commit/46d5af6fb22a312cf7175cc25e05675e64038343) Allow `render_changeset_markdown` to accept per-package bump entries so callers can record different levels in a single changeset. — Thanks @goulvenclech!

### Patch changes

- [6c431c4](https://github.com/bruits/sampo/commit/6c431c4a93c9195e7a9f0eee4e82b88d945a1a47) Releases now bump the right crate even if the package name is quoted. — Thanks @goulvenclech!
- [6c431c4](https://github.com/bruits/sampo/commit/6c431c4a93c9195e7a9f0eee4e82b88d945a1a47) Sampo's CLI should not add quotes around package names in changesets. — Thanks @goulvenclech!


## 0.4.0

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


## 0.3.1

### Patch changes

- [061a5f3](https://github.com/bruits/sampo/commit/061a5f368f6409a868d94dc60f39f0fc1c138727) `packages.ignore` and `packages.ignore_unpublished` configuration options now work as intended for release and publishing steps. — Thanks @goulvenclech!


## 0.3.0

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
- [5255617](https://github.com/bruits/sampo/commit/5255617685f9ab71fd2af336536758fd16e547df) Fix `workspace = true` dependencies handling, whether for internal monorepo dependencies or monorepo-wide external dependencies. — Thanks @goulvenclech!


## 0.2.1

### Patch changes

- [1c47715](https://github.com/bruits/sampo/commit/1c47715b40df61d4768f371826858c6d5f7fda71) Bump `sampo-core` version to propagate an unpublished fix to `sampo-github-action` and `sampo` CLI. Should definitely fix the malformed `Cargo.toml` issue in release PRs. — Thanks @goulvenclech!


## 0.2.0

### Minor changes

- [20ea306](https://github.com/bruits/sampo/commit/20ea306ce5e913a90c64b19544820f2503625df7) New `release` and `publish` API endpoints in the core library, to be used by the GitHub Action and CLI. — Thanks @Princesseuh!


## 0.1.1

### Patch changes

- [6062083](https://github.com/bruits/sampo/commit/6062083ae20e3bcea6c1f4f00d6b58cf790cd9f1) Fix deploys and publishing. — Thanks @goulvenclech!


## 0.1.0

### Minor changes

- [78515cc](https://github.com/bruits/sampo/commit/78515ccfbf53dcd952dc7f7e7716c0f0a5fc82b6) Initial release of `sampo-core`, a foundational crate providing core logic, common types, and internal utilities shared across all Sampo crates. — Thanks Goulven Clec'h!

