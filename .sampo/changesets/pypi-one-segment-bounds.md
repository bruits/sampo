---
cargo/sampo: patch
cargo/sampo-core: patch
cargo/sampo-github-action: patch
---

In Python (PyPI) projects, version bounds with a single segment, such as `<4` in `core>=2.8,<4`, are now checked like `<4.0`. A release outside such a bound warns, or blocks in a fixed or linked group, and a covered bound like `>=2` is left as written.
