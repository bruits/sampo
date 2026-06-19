---
cargo/sampo: patch
---

Changed `sampo update` to an optional `self-update` Cargo feature (enabled by default) so Sampo can be packaged for more package managers that handle updates themselves. Disable it with `--no-default-features`.
