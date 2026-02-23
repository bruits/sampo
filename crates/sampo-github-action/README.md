# Sampo's GitHub Action

GitHub Action to run Sampo (release and/or publish) in GitHub Actions.

Not sure what Sampo is? Don't know where to start? Check out Sampo's [documentation](./crates/sampo/README.md).

## Usages

### Release and Publish

By default, the action runs in `auto` mode:

- When changesets exist on the current release branch (see the `[git]` configuration), it prepares or refreshes that branch's release PR.
- When the release plan targets pre-release versions, it also prepares a stabilize PR that exits pre-release mode and lines up the stable release for the same set of changesets.
- When that PR is merged, it publishes your packages, creates tags, and can open GitHub Releases/Discussions. Can also mark Github Releases as « pre-release » for pre-releases branches.

```yaml
name: Release & Publish
on:
  push:
    branches: [main]
  workflow_dispatch: {}

permissions:
  contents: write      # Create tags and releases
  pull-requests: write # Create and update release PRs
  discussions: write   # Open GitHub Discussions (optional)

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
          # Options here, see below
        env:
          GITHUB_TOKEN: ${{ secrets.GITHUB_TOKEN }}
          CARGO_TOKEN: ${{ secrets.CARGO_TOKEN }}     # For Cargo packages (optional)
          NPM_TOKEN: ${{ secrets.NPM_TOKEN }}         # For npm packages (optional, uses .npmrc if not set)
          HEX_API_KEY: ${{ secrets.HEX_API_KEY }}     # For Hex packages (optional)
          UV_PUBLISH_TOKEN: ${{ secrets.PYPI_TOKEN }} # For PyPI packages via uv (optional)
```

### Creating GitHub Releases and Discussions

Set the `create-github-release` input to `true` to create a GitHub Release for each new tag created when publishing packages. The release notes are generated from the changesets included in the release.

To also open a GitHub Discussion for each release, set `open-discussion` to `true` (all packages) or a comma-separated list of package names (e.g., `sampo,sampo-github-action`). Use `discussion-category` to specify the target category.

### Uploading release assets in GitHub Releases

Build binaries or archives in earlier workflow steps, then pass their paths or glob patterns through the `release-assets` input for upload. Patterns are resolved relative to `working-directory` unless absolute.

- Separate multiple entries with commas or new lines.
- Use `=>` to rename matches (for example `dist/*.zip => my-tool.zip`).
- The placeholders `{{tag}}`, `{{crate}}`, and `{{version}}` expand from the tag being published.

Example:

```yaml
- run: cargo build --release
- uses: bruits/sampo/crates/sampo-github-action@main
  with:
    create-github-release: true
    release-assets: |
      target/release/my-cli => my-cli-{{tag}}
      dist/{{crate}}-v{{version}}-*.tar.gz
```

### Using outputs to conditionally run steps

The action exposes two outputs:

- `released`: `"true"` when release automation ran (release PR prepared, stabilize PR prepared, or `sampo release` executed).
- `published`: `"true"` when `sampo publish` completed successfully and created version tags.

These outputs can be used to gate subsequent steps, example:

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
  run: echo "Packages were published"
```

## Configuration

The action supports the following inputs:

- `command`: `auto`, `release`, or `publish` (default: `auto`).
- `dry-run`: if `true`, simulates changes without writing or publishing (default: `false`).
- `working-directory`: path to workspace root (defaults to `GITHUB_WORKSPACE`).
- `cargo-token`: crates.io API token for Cargo packages; when set, exported as `CARGO_REGISTRY_TOKEN`.
- `npm-token`: npm API token for npm packages, or uses `.npmrc` if not set. When provided, it is exported as `NPM_TOKEN` for use by npm/pnpm/yarn/bun during publish.
- `args`: extra flags forwarded to **all** ecosystem publish commands via `sampo publish -- …`.
- `cargo-args`: extra arguments forwarded only to `cargo publish`.
- `npm-args`: extra arguments forwarded only to npm/pnpm/yarn/bun publish.
- `hex-args`: extra arguments forwarded only to `mix hex.publish`.
- `pypi-args`: extra arguments forwarded only to PyPI/twine upload.
- `packagist-args`: extra arguments forwarded only to Packagist/Composer.
- `base-branch`: base branch used by the release PR that `auto` prepares (defaults to the detected git branch).
- `pr-branch`: working branch used for the release PR that `auto` prepares (defaults to `release/<current-branch>` with `/` replaced by `-`).
- `pr-title`: title of the release PR that `auto` prepares (defaults to `Release (<current-branch>)`).
- `stabilize-pr-branch`: working branch used for the stabilize PR that `auto` prepares (defaults to `stabilize/<current-branch>` with `/` replaced by `-`).
- `stabilize-pr-title`: title of the stabilize PR that `auto` prepares (defaults to `Release stable (<current-branch>)`).
- `create-github-release`: if `true`, create GitHub Releases for new tags.
- `open-discussion`: create a GitHub Discussion for released packages. Accepts `true` (all packages), `false` (none, default), or a comma-separated list of package names to filter (e.g., `sampo,sampo-github-action`). Requires `create-github-release: true`.
- `discussion-category`: preferred Discussions category slug when creating releases.
- `release-assets`: comma or newline separated list of paths or glob patterns for pre-built artifacts to upload when creating GitHub releases. Use `=>` to rename matches (e.g. `dist/*.zip => my-tool.zip`). Placeholders `{{tag}}`, `{{crate}}`, and `{{version}}` are available.
- `github-token`: GitHub token to create/update PRs (defaults to `GITHUB_TOKEN` env).
- `use-local-build`: if `true`, compile the local `sampo-github-action` binary instead of installing it with `cargo-binstall`.

## Development

Refer to [CONTRIBUTING.md](../../CONTRIBUTING.md#sampo-github-action) for development setup and workflow details.
