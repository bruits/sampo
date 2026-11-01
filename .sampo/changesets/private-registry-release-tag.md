---
cargo/sampo: patch
cargo/sampo-core: patch
cargo/sampo-github-action: patch
---

Fixed `sampo publish` publishing a version again, and failing, when it goes to a private registry Sampo cannot query (a private PyPI index, Maven repository, Composer repository, or gem server). A tagged version there now counts as published, and a package that gets `--dry-run` through the forwarded arguments is no longer tagged.
