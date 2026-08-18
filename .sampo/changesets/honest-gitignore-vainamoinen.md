---
cargo/sampo: minor
cargo/sampo-core: minor
cargo/sampo-github-action: minor
---

In Python (PyPI), Gleam (Hex), and Erlang (Hex) projects, package scanning now honours `.gitignore` files: gitignored manifests are skipped, and virtual environments self-exclude through the catch-all `.gitignore` written by uv and `python -m venv` (Python 3.13+). Only `.gitignore` files inside the repository are read — never your personal global gitignore (`core.excludesFile`) — so local and CI scans agree.
