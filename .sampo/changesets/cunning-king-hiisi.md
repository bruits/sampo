---
packages:
  - sampo
  - sampo-core
  - sampo-github-action
release: minor
---

Add support for ignoring packages during releases and in CLI package lists. You can now exclude unpublishable packages or specific packages by name/path patterns from Sampo operations.

```toml
[packages]
# Skip packages that aren't publishable to crates.io
ignore_unpublished = true
# Skip packages matching these patterns
ignore = [
  "internal-*",     # Ignore by name pattern
  "examples/*",     # Ignore by workspace path
  "benchmarks/*"
]
```
