---
cargo/sampo: patch
cargo/sampo-core: patch
cargo/sampo-github-action: patch
---

Elixir packages without a `package()` function in `mix.exs` are now correctly identified as private and excluded from publishing.
