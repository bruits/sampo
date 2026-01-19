---
cargo/sampo: minor
---

Improve exit codes to easily detect when running a `sampo` command would not apply any changes. This is useful in CI jobs when we don't want to run the whole process if there's no changes to be applied.
