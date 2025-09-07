## Sampo's GitHub Action

GitHub Action to run Sampo (release and/or publish) in GitHub Actions.

If you don't use Sampo yet, please [check it out](https://github.com/bruits/sampo/tree/main/crates/sampo).

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
          cargo-token: ${{ secrets.CARGO_REGISTRY_TOKEN }}
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
        env:
          # Default token is available automatically; ensure permissions above are set
          GITHUB_TOKEN: ${{ secrets.GITHUB_TOKEN }}
```

Publish after the Release PR is merged (optionally create GitHub releases):

```yaml
name: Publish After Merge
on:
  pull_request:
    types: [closed]
    branches: [ main ]

permissions:
  contents: write

jobs:
  publish:
    # Only run when the Release PR (created by prepare-pr) is merged
    if: >
      github.event.pull_request.merged == true &&
      github.event.pull_request.base.ref == 'main' &&
      (
        startsWith(github.event.pull_request.head.ref, 'release/') ||
        contains(github.event.pull_request.title, 'Release')
      )
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
        with:
          fetch-depth: 0
          ref: ${{ github.event.pull_request.base.ref }}
      - name: Publish crates (and create tags)
        uses: bruits/sampo/crates/sampo-github-action@main
        with:
          command: post-merge-publish
          cargo-token: ${{ secrets.CARGO_REGISTRY_TOKEN }}
          # optional: also create GitHub Releases for new tags
          create-github-release: true
          # optional: pass flags to cargo publish
          args: --allow-dirty --no-verify
```

Notes:

- prepare-pr runs `sampo release` on a release branch and opens/updates a PR.
- post-merge-publish runs only when the Release PR is merged into `main`. It creates any missing tags for the current crate versions on the main branch, pushes them, and runs `sampo publish`. It can also create GitHub Releases for the new tags.
- Adjust the branch/title condition in the workflow if you customize the release PR branch name.
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
- `github-token`: GitHub token to create/update PRs (defaults to `GITHUB_TOKEN` env)

### Outputs

- `released`: "true" if the release step executed successfully
- `published`: "true" if the publish step executed successfully
