---
cargo/sampo-core: minor
cargo/sampo: minor
cargo/sampo-github-action: minor
---

In Python (PyPI) projects, added support for private indexes. For packages that declare a uv index with a `publish-url` in `pyproject.toml`, `sampo publish` now routes the upload to that index via `uv publish --index`, which resolves the upload URL, credentials, and already-published checks from your uv configuration.
