---
cargo/sampo-core: patch
cargo/sampo: patch
---

Fixed `packages.fixed` group members being silently released at different bump levels when another changeset or a `packages.linked` group raised one of them.
