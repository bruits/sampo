## Sampo GitHub Action

Run Sampo (release and/or publish) in GitHub Actions.

This action drives the existing `sampo` CLI within the repository workspace. It can bump versions and changelogs from changesets, and publish crates to crates.io. Git tags are created during the publish step.

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

Prepare a Release PR on push (recommended):

```yaml
name: Prepare Release PR
on:
  push:
    branches: [ main ]

permissions:
  contents: write
  pull-requests: write

jobs:
  release_pr:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
        with:
          fetch-depth: 0
      - name: Open/refresh Release PR
        uses: bruits/sampo/crates/sampo-github-action@main
        with:
          command: prepare-pr
          # optional overrides (defaults shown):
          # base-branch: main
          # pr-branch: release/sampo
          # pr-title: Release
```

Publish after the Release PR is merged (optionally create GitHub releases):

```yaml
name: Publish After Merge
on:
  push:
    branches: [ main ]

permissions:
  contents: write

jobs:
  publish:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
        with:
          fetch-depth: 0
      - name: Publish crates (and create tags)
        uses: bruits/sampo/crates/sampo-github-action@main
        with:
          command: post-merge-publish
          cargo-token: ${{ secrets.CRATES_IO_TOKEN }}
          # optional: also create GitHub Releases for new tags
          create-github-release: true
          # optional: pass flags to cargo publish
          args: --allow-dirty --no-verify
```

Notes:

- prepare-pr runs `sampo release --skip-tag` on a release branch and opens/updates a PR. No tags are created on the PR branch.
- post-merge-publish creates any missing tags for the current crate versions on the main branch, pushes them, and runs `sampo publish`. It can also create GitHub Releases for the new tags.
- Ensure the workflow has `contents: write` (and `pull-requests: write` for prepare-pr) permissions.

### Inputs

- `command`: `release`, `publish`, `release-and-publish`, `prepare-pr`, or `post-merge-publish` (default: `release-and-publish`)
- `dry-run`: if `true`, simulates changes without writing or publishing (default: `false`)
- `working-directory`: path to workspace root (defaults to `GITHUB_WORKSPACE`)
- `cargo-token`: crates.io API token; when set, exported as `CARGO_REGISTRY_TOKEN`
- `args`: extra flags forwarded to `cargo publish` via `sampo publish -- …`
- `base-branch`: base branch used by the Release PR (prepare-pr)
- `pr-branch`: working branch used for the Release PR (prepare-pr)
- `pr-title`: title of the Release PR (prepare-pr)
- `create-github-release`: if `true`, create GitHub Releases for new tags (post-merge-publish)

### Outputs

- `released`: "true" if the release step executed successfully
- `published`: "true" if the publish step executed successfully
