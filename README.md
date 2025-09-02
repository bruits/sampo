<picture>
  <source media="(prefers-color-scheme: dark)" srcset="./.github/assets/Sampo_logo_dark.svg" />
  <img alt="Sampo logo" src="./.github/assets/Sampo_logo_light.svg" />
</picture>

Automate changelogs, versioning, and publishing—even for monorepos across multiple package registries. Deeply inspired by [Changesets](https://github.com/changesets/changesets) and [Lerna](https://github.com/lerna/lerna). Enforce [Semantic Versioning](https://semver.org/) (SemVer) across all packages.

> [!WARNING]
> This project is in early development, most features are not yet implemented, and the API may change dramatically.

## Crates

Sampo is a monorepo that contains the following crates (Rust packages):

| Name                  | Description                                          | Link                                                        | README                                        |
| --------------------- | ---------------------------------------------------- | ----------------------------------------------------------- | --------------------------------------------- |
| `sampo`               | CLI to manage changesets, versioning, and publishing | ![Crates.io Version](https://img.shields.io/crates/v/sampo) | [README](./crates/sampo/README.md)            |
| `sampo-github-bot`    | GitHub App to inspect PRs and request changesets     | [GitHub App](https://github.com/apps/bruits-sampo)          | [README](./crates/sampo-github-bot/README.md) |
| `sampo-github-action` | GitHub Action to automate the release process.       | *WIP*                                                       | *Soon*                                        |

## Packages

Additionally, Sampo contains the following packages for diverse ecosystems:

| Name | Description | Link | README |
| ---- | ----------- | ---- | ------ |
