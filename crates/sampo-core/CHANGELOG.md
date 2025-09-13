# sampo-core

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

