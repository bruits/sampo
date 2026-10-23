---
cargo/sampo-github-action: patch
---

Fixed GitHub release notes taking the top section of the changelog whatever its version. They now take the section whose heading matches the tag's version, or fall back to a generic body.
