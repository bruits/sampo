---
"cargo:sampo": minor
"cargo:sampo-core": minor
"cargo:sampo-github-action": minor
---

To avoid ambiguity between packages in different ecosystems (e.g. a Rust crate and an npm package both named `example`), Sampo now assigns canonical identifiers to all packages, prefixed by their ecosystem: `cargo:example` for Rust crates, `npm:example` for npm packages, etc.

Changesets, the CLI, and the GitHub Action now accept and emit ecosystem-qualified package names. Plain package names (without ecosystem prefix) are still supported when there is no ambiguity.
