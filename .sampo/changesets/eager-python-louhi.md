---
cargo/sampo: minor
cargo/sampo-core: minor
---

In Python (PyPI) projects, added package discovery without a root `pyproject.toml`: when the repository root declares nothing (no `pyproject.toml`, or one holding only tool configuration), Sampo scans subdirectories for package manifests, as it already does for Gleam and Erlang, skipping virtual environments, caches, build output, and test fixtures. A root `[project]` or `[tool.uv.workspace]` still wins, exactly as uv resolves it. This also applies to repositories of other ecosystems: a nested `pyproject.toml` in a repo with no root Python manifest now becomes a workspace member (use `packages.ignore` to exclude it). Scanned packages that commit their own `uv.lock` get it refreshed on release, like the shared root lockfile of a declared workspace.

Fixed a single unusable manifest (unparseable, or missing its package name) aborting discovery for the entire repository, including every other ecosystem's packages. The file is now skipped with a warning naming it and the reason.

In Python (PyPI) projects, fixed `sampo release` and `sampo pre` silently dropping the version bump of a package whose `pyproject.toml` has no `[project].version` field: the changelog and tag advanced while the manifest kept its old version. Such a plan now fails before anything is written.

Improved `sampo init` errors: they now list the manifest files Sampo looks for, and distinguish a missing manifest from a manifest that yields no packages.
