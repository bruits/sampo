---
cargo/sampo: patch
cargo/sampo-core: patch
cargo/sampo-github-action: patch
---

Fixed `sampo release` folding existing changelog sections into the new release when their heading is not the bare current version, such as `## [1.0.1] - 2024-01-01`, `## v1.0.1`, `## About`, or the stable release a pre-release was entered from: they are now kept as written, and the new release goes above the first version heading. Also fixed the first release after `sampo pre exit` bumping the version a second time when a `##` section sits above the pre-release section.
