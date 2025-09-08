# sampo

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

