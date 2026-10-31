# Sampo's GitHub Action

GitHub Action to run Sampo (release and/or publish) in GitHub Actions.

Not sure what Sampo is? Don't know where to start? Check out Sampo's [documentation](https://github.com/bruits/sampo/blob/main/crates/sampo/README.md).

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
  id-token: write      # Publish to npm through trusted publishing (optional)

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
          NPM_TOKEN: ${{ secrets.NPM_TOKEN }}         # For npm packages (optional, not needed with trusted publishing)
          HEX_API_KEY: ${{ secrets.HEX_API_KEY }}     # For Hex packages (optional)
          UV_PUBLISH_TOKEN: ${{ secrets.PYPI_TOKEN }} # For PyPI packages via uv (optional)
          # Maven packages read credentials from ~/.m2/settings.xml (see actions/setup-java)
```

> [!WARNING]
> If your repository commits a lockfile, install the matching package manager in the workflow before this step. Runners ship `npm` and `yarn`, but not `pnpm` or `bun`.

> [!NOTE]
> npm packages can publish without `NPM_TOKEN` through [npm trusted publishing](https://docs.npmjs.com/trusted-publishers). Grant `id-token: write`, add a trusted publisher on npmjs.com for each package, with this workflow's filename and `npm publish` allowed, and use npm 11.5.1 or later (Node 24 ships one). In Bun workspaces, Sampo packs with Bun and publishes with npm. With [`packages.npm_stage`](https://github.com/bruits/sampo/blob/main/crates/sampo/README.md#packages-section), allow `npm stage publish` instead, and approve each version on npmjs.com.

### Creating GitHub Releases and Discussions

Set the `create-github-release` input to `true` to create a GitHub Release for each new tag created when publishing packages. The release notes are generated from the changesets included in the release.

In a monorepo that ships one product built from many packages, set `create-github-release` to a comma-separated list of package names instead. Only those packages get a GitHub Release, and the release of the package listed first also carries the notes of every other package published with it: an "Other packages" section with each package's version and changelog section, and a collapsed list of the packages bumped only to follow others. If none of the listed packages has a new tag, no release is created, though all tags are still pushed. When the notes exceed GitHub's limit of 125,000 characters, the last packages are left out with a note pointing to their changelogs.

```yaml
- uses: bruits/sampo/crates/sampo-github-action@main
  with:
    create-github-release: my-app # one release per publish, for my-app
```

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

> [!IMPORTANT]
> GitHub's **Immutable releases** repository setting must be disabled for release asset uploads to work.

### Using outputs to conditionally run steps

The action exposes two outputs:

- `released`: `"true"` when release automation ran (release PR prepared, stabilize PR prepared, or `sampo release` executed).
- `published`: `"true"` when `sampo publish` completed successfully and created version tags. With `git.tag_unpublished` set to `false` or a list, a release of only untagged packages creates no tag and reports `"false"`.

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
- `args`: extra flags forwarded to **all** ecosystem publish commands via `sampo publish -- …`.
- `cargo-args`: extra arguments forwarded only to `cargo publish`.
- `npm-args`: extra arguments forwarded only to npm/pnpm/yarn/bun publish.
- `hex-args`: extra arguments forwarded only to `mix hex.publish`.
- `pypi-args`: extra arguments forwarded only to PyPI/twine upload.
- `packagist-args`: extra arguments forwarded only to Packagist/Composer.
- `maven-args`: extra arguments forwarded only to `mvn deploy`.
- `base-branch`: base branch used by the release PR that `auto` prepares (defaults to the detected git branch).
- `pr-branch`: working branch used for the release PR that `auto` prepares (defaults to `release/<current-branch>` with `/` replaced by `-`).
- `pr-title`: title of the release PR that `auto` prepares (defaults to `Release (<current-branch>)`).
- `stabilize-pr-branch`: working branch used for the stabilize PR that `auto` prepares (defaults to `stabilize/<current-branch>` with `/` replaced by `-`).
- `stabilize-pr-title`: title of the stabilize PR that `auto` prepares (defaults to `Release stable (<current-branch>)`).
- `create-github-release`: create GitHub Releases for new tags. Accepts `true` (one per new tag), `false` (none, default), or a comma-separated list of package names that get a release, the first listed one gathering the other packages' notes (see [Creating GitHub Releases and Discussions](#creating-github-releases-and-discussions)).
- `open-discussion`: create a GitHub Discussion for released packages. Accepts `true` (all packages), `false` (none, default), or a comma-separated list of package names to filter (e.g., `sampo,sampo-github-action`). Only applies to the releases `create-github-release` creates.
- `discussion-category`: preferred Discussions category slug when creating releases.
- `release-assets`: comma or newline separated list of paths or glob patterns for pre-built artifacts to upload when creating GitHub releases. Use `=>` to rename matches (e.g. `dist/*.zip => my-tool.zip`). Placeholders `{{tag}}`, `{{crate}}`, and `{{version}}` are available.
- `github-token`: GitHub token to create/update PRs (defaults to `GITHUB_TOKEN` env).
- `use-local-build`: if `true`, compile the local `sampo-github-action` binary instead of installing it with `cargo-binstall`.

## Development

The composite action in `action.yml` runs this crate's binary, which shells out to git and calls the GitHub API, so it needs credentials and a clean git workspace. Integration tests simulate a repository in temporary directories, but a full run expects GitHub Actions (its `GITHUB_TOKEN`, a checked-out repository, and sometimes `cargo-binstall`). Testing a change often means pushing a branch to a test repository and watching a real workflow run, and help to make this easier is welcome.

Refer to [CONTRIBUTING.md](https://github.com/bruits/sampo/blob/main/CONTRIBUTING.md#getting-started) for development setup and workflow details.
