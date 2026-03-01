---
cargo/sampo: minor
cargo/sampo-core: patch
---

Added `--bump` / `-b` and `--tag` / `-t` flags to `sampo add`, enabling fully non-interactive changeset creation when combined with `--package` and `--message`:

```sh
sampo add -p my-crate -b minor -t Added -m "Added foo support"
```
