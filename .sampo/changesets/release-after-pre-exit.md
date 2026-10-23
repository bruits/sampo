---
cargo/sampo: patch
cargo/sampo-core: patch
cargo/sampo-github-action: patch
---

Fixed the first release after `sampo pre exit` bumping versions a second time: exited packages now ship at the version their pre-release reached (`1.1.0` after `1.1.0-alpha.1`), even in a release that bumps other packages still in pre-release, and including those that left pre-release mode before this fix.
