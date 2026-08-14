---
cargo/sampo: minor
cargo/sampo-core: minor
cargo/sampo-github-action: minor
---

**⚠️ breaking change:** Sampo now fails a release when it cannot regenerate a lockfile, instead of warning and committing a stale one. Tooling is checked before anything is written, so a refused `sampo release` or `sampo pre` leaves the workspace untouched.

In JavaScript/TypeScript (npm) projects, fixed Bun lockfile regeneration, which ran an invalid command and left the released package stale in `bun.lock`. Bun 1.2 or later is now required.
