## Sampo's GitHub Action

GitHub Action to run Sampo (release and/or publish) in GitHub Actions.

Not sure what Sampo is? Don't know where to start? Check out Sampo's [Getting Started](./crates/sampo/README.md#getting-started) guide.

### Usage

By default, the action runs in `auto` mode:
- When changesets exist on the default branch, it prepares or refreshes the release PR.
- After that PR is merged, the next push publishes your crates, creates tags, and can open GitHub Releases/Discussions.

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

### Inputs

TODO: add permissions needed when known.

- `command`: `auto`, `release`, or `publish` (default: `auto`).
- `dry-run`: if `true`, simulates changes without writing or publishing (default: `false`).
- `working-directory`: path to workspace root (defaults to `GITHUB_WORKSPACE`).
- `cargo-token`: crates.io API token; when set, exported as `CARGO_REGISTRY_TOKEN`.
- `args`: extra flags forwarded to `cargo publish` via `sampo publish -- …`.
- `base-branch`: base branch used by the release PR that `auto` prepares.
- `pr-branch`: working branch used for the release PR that `auto` prepares.
- `pr-title`: title of the release PR that `auto` prepares.
- `create-github-release`: if `true`, create GitHub Releases for new tags.
- `open-discussion`: if `true`, create a GitHub Discussion for each created release (requires GitHub Releases).
- `discussion-category`: preferred Discussions category slug when creating releases.
- `upload-binary`: if `true`, upload a binary asset when creating GitHub releases (only for crates with a binary target).
- `binary-name`: override binary name (defaults to the crate name or the single `[[bin]]` name when unambiguous).
- `targets`: space- or comma-separated Rust target triples to build and upload (must be installed); default builds the host target only.
- `github-token`: GitHub token to create/update PRs (defaults to `GITHUB_TOKEN` env).
- `use-local-build`: if `true`, compile the local `sampo-github-action` binary instead of installing it with `cargo-binstall`.

### Outputs

- `released`: `"true"` when release automation ran (release PR prepared or `sampo release` executed).
- `published`: `"true"` when new tags were pushed (i.e. crates were published in non-dry runs).

Can be used to gate subsequent steps, example:

```yaml
      - name: Create release PR or publish packages
        id: sampo
        uses: bruits/sampo/crates/sampo-github-action@main
        with:
          command: auto
        env:
          GITHUB_TOKEN: ${{ secrets.GITHUB_TOKEN }}
      - name: Notify on release PR creation
        if: steps.sampo.outputs.released == 'true'
        run: echo "Release PR created or updated"
      - name: Notify on publish
        if: steps.sampo.outputs.published == 'true'
        run: echo "Crates were published"
```
