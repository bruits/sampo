---
cargo/sampo-core: minor
cargo/sampo: minor
cargo/sampo-github-action: minor
---

In PHP (Packagist) projects, added support for private registries. Packages resolved through a private or alternative Composer registry (a `type: "composer"` repository in `composer.json`, or `packagist.org` disabled) no longer have their already-published check run against public Packagist. As private Composer registries (Private Packagist, Satis) stay VCS-based, `sampo publish` defers to the git-tag push that drives the registry update.
