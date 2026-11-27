---
cargo/sampo-core: minor
cargo/sampo: minor
cargo/sampo-github-action: minor
---

Added `packages.versioning = "calver"` to use the UTC year and month for `sampo release` and release pull requests. To pin a release line, also set `packages.calver_year_month`.
