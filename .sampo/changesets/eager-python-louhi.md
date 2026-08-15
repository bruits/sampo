---
cargo/sampo: minor
cargo/sampo-core: minor
---

In Python (PyPI) projects, added package discovery without a root `pyproject.toml`, and an explicit root `[project]` or `[tool.uv.workspace]` still wins. Also, releasing a package without a static `[project].version` now fails before anything is written, instead of recording a bump the manifest never received.
