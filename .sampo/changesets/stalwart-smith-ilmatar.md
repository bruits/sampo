---
cargo/sampo: minor
cargo/sampo-core: minor
cargo/sampo-github-action: minor
---

**⚠️ breaking change:** Sampo now refuses to release a package whose version it cannot parse or bump (for example a PEP 440 `1.0.0.post1`), instead of planning a do-nothing release that consumed the changeset and wrote a changelog entry under the unchanged version. A run that used to succeed while silently skipping such a package now fails with the package and version named, leaving manifests untouched and no changeset consumed; fix its version, or exclude it with `packages.ignore`. The same validation now runs before `sampo pre enter`'s label switch: a refused entry (invalid label, unbumpable version) used to exit pre-release mode and move preserved changesets back before failing; it now leaves the workspace untouched.
