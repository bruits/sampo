---
cargo/sampo-core: minor
cargo/sampo: minor
cargo/sampo-github-action: minor
---

Added Ruby gem support. `sampo release` versions gems found by their gemspec (from a literal `spec.version` or the version constant it references) and updates sibling requirements and `Gemfile.lock`, and `sampo publish` pushes them to RubyGems (`--rubygems-args` in the CLI, `rubygems-args` in the GitHub Action).
