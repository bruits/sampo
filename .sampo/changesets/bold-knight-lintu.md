---
cargo/sampo: patch
cargo/sampo-core: patch
cargo/sampo-github-action: patch
---

Fixed publish command failing on Windows when package managers (npm, pnpm, yarn, composer, mix) are installed as .cmd/.bat scripts.
