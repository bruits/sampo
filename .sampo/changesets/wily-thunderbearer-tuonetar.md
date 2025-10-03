---
sampo: patch
sampo-core: patch
sampo-github-action: patch
---

Fix release workflow so root `Cargo.toml` refreshes semver versions for member dependencies declared in `[workspace.dependencies]`, while leaving wildcard or path-only entries untouched.
