---
cargo/sampo-core: minor
cargo/sampo: minor
cargo/sampo-github-action: minor
---

In Gleam projects, added support for discovering, versioning, and publishing `gleam.toml` packages to Hex. Gleam shares the hex.pm registry with Elixir, so Sampo now recognises `gleam.toml` alongside `mix.exs` in a Hex workspace, bumps versions and dependency requirements in it, publishes with `gleam publish`, and refreshes the `manifest.toml` lockfile with `gleam deps download`.
