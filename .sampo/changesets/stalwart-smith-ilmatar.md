---
cargo/sampo-core: patch
cargo/sampo: patch
---

Fixed `sampo release` planning a do-nothing release for a package whose version it cannot bump (for example a PEP 440 `1.0.0.post1`) consuming the changeset and writing a changelog entry under the unchanged version. It now fails with the package and version named, leaving manifests and changesets untouched.
