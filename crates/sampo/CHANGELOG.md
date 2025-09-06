# sampo

## 0.2.0

### Minor changes

- [`d0d7244`](https://github.com/bruits/sampo/commit/d0d7244a43d76a0d7b377cf5f328a1fe244282b4) Changelog messages are now enriched with commit hash links and author thank-you notes, especially for first-time contributors. Added `[changelog]` configuration section with `show_commit_hash` and `show_acknowledgments` options (both default to true). — Thanks @goulvenclech!

### Patch changes

- [`c7f252e`](https://github.com/bruits/sampo/commit/c7f252ef8c2671c3d35a3a69ab878591f024bf4a) Changed release and publish semantics: `sampo release` no longer creates git tags and can be run multiple times to iteratively prepare a release. Git tags are now created by `sampo publish` after successful crate publication. This allows for better separation between release preparation and finalization workflows. This is technically a **breaking change** ⚠️ but this is expected in 0.x.x alpha versions. — Thanks @goulvenclech!
- [`978bbe4`](https://github.com/bruits/sampo/commit/978bbe4e78205a685a6f92ae0412f8d5e65c3259) Only publish crates with a release tag (`name-v<version>`) and skip versions already on crates.io. Prevents “crate already exists” errors and avoids publishing unplanned crates. — Thanks @goulvenclech!


## 0.1.0

### Minor changes

- Initial release of Sampo CLI, a tool to automate changelogs, versioning, and publishing. This first version includes proof of concepts for the main commands (`help`, `init`, `add`, `release`, `publish`), and has been published using Sampo itself!

