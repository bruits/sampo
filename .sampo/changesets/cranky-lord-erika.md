---
cargo/sampo: minor
cargo/sampo-core: minor
cargo/sampo-github-action: minor
---

Added `git.short_tags` configuration option to create short version tags (`vX.Y.Z`) for a single package. In PHP (Packagist) projects, this enables Composer-compatible releases, with the limitation of not supporting monorepos with multiple publishable PHP packages.
