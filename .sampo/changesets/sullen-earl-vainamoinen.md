---
sampo-github-action: major
---

**⚠️ breaking change:** Drop the legacy `prepare-pr`, `post-merge-publish`, and `release-and-publish` commands in favour of the unified `auto` flow and the explicit `release` / `publish` modes. This simplifies massively the configuration and usage, to only one workflow for both creating release PRs and publishing crates:

```yaml
name: Release & Publish
on:
  push:
    branches: [ main ]
  workflow_dispatch: {}

permissions:
  # TODO: precise permissions needed

jobs:
  release:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
        with:
          fetch-depth: 0
      - name: Run Sampo Release/Publish Action
        id: sampo
        uses: bruits/sampo/crates/sampo-github-action@main
        with:
          # Read the "Inputs" section below for details on these options
        env:
          GITHUB_TOKEN: ${{ secrets.GITHUB_TOKEN }}
          CARGO_TOKEN: ${{ secrets.CARGO_TOKEN }} # Only needed to publish to crates.io
```
