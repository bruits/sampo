---
cargo/sampo: patch
cargo/sampo-core: patch
cargo/sampo-github-action: patch
---

Fixed `sampo publish` failing on every run after publishing to a private registry Sampo cannot query (a private PyPI index, Maven repository, Composer repository, or gem server), as it published the same version again. The release tag now marks such a version as published.
