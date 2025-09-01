<picture>
  <source media="(prefers-color-scheme: dark)" srcset="./.github/assets/Sampo_logo_dark.svg" />
  <img alt="Sampo logo" src="./.github/assets/Sampo_logo_light.svg" />
</picture>

Automate changelogs, versioning, and publishing—even for monorepos across multiple package registries. Deeply inspired by [Changesets](https://github.com/changesets/changesets) and [Lerna](https://github.com/lerna/lerna).

## Ideas

- `sampo`, a CLI to generate changeset files. (Clap?) (How to distribute to other package registries?)
- `sampo-github-bot`, inspect PRs and ask for changesets. (Typescript?)
- `sampo-github-action`, open PRs to consume changesets, generate changelogs, bump versions, and publish packages.
- utils crate for common functionality. Probably multiple so each package can depend on only what it needs.

## Crates

Sampo is a monorepo that contains the following crates (Rust packages):

| Name  | Description                                          | Crates.io | README                             |
| ----- | ---------------------------------------------------- | --------- | ---------------------------------- |
| sampo | CLI to manage changesets, versioning, and publishing | *WIP*     | [README](./crates/sampo/README.md) |


## Packages

Additionally, Sampo contains the following packages for diverse ecosystems:

| Name | Description | Package Registry | README |
| ---- | ----------- | ---------------- | ------ |
