## Sampo GitHub Action

Run Sampo (release and/or publish) in GitHub Actions.

This action drives the existing `sampo` CLI within the repository workspace. It can bump versions and changelogs from changesets, create git tags, and publish crates to crates.io.

### Usage

Minimal example (release only):

```yaml
name: Release
on:
  push:
    branches: [ main ]

jobs:
  release:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - name: Run Sampo release
        uses: bruits/sampo/crates/sampo-github-action@main
        with:
          command: release
```

Release then publish (requires a crates.io token):

```yaml
name: Release & Publish
on:
  workflow_dispatch: {}

jobs:
  release_publish:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - name: Sampo release & publish
        uses: bruits/sampo/crates/sampo-github-action@main
        with:
          command: release-and-publish
          cargo-token: ${{ secrets.CRATES_IO_TOKEN }}
          # Optional: pass flags to cargo publish (after `--`)
          args: --allow-dirty --no-verify
```

### Inputs

- `command`: `release`, `publish`, or `release-and-publish` (default: `release-and-publish`)
- `dry-run`: if `true`, simulates changes without writing or publishing (default: `false`)
- `working-directory`: path to workspace root (defaults to `GITHUB_WORKSPACE`)
- `cargo-token`: crates.io API token; when set, exported as `CARGO_REGISTRY_TOKEN`
- `args`: extra flags forwarded to `cargo publish` via `sampo publish -- …`

### Outputs

- `released`: "true" if the release step executed successfully
- `published`: "true" if the publish step executed successfully
