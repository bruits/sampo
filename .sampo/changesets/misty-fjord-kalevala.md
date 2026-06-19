---
cargo/sampo: patch
---

Changed `sampo update` to be an optional Cargo feature (`self-update`, enabled by default). Disable it with `--no-default-features` to exclude self-update dependency and reduce binary size.
