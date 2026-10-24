---
cargo/sampo: patch
cargo/sampo-core: patch
cargo/sampo-github-action: patch
---

Fixed `sampo release` folding or deleting existing changelog sections, such as `## [1.0.1] - 2024-01-01`, `## v1.0.1`, `## About`, or the stable release a pre-release was entered from. They are now kept as written, the new release goes above the first `##` heading holding a digit, and `## [Unreleased]` stays on top without being folded into the release.
