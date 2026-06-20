---
cargo/sampo-core: minor
cargo/sampo: minor
cargo/sampo-github-action: minor
---

In Elixir (Hex) projects, added support for private organisations. For packages that declare an `organization` in their `mix.exs` package configuration, `sampo publish` now checks the organisation's repository for already-published versions and authenticates that check with `HEX_API_KEY`.
