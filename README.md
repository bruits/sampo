<picture>
  <source media="(prefers-color-scheme: dark)" srcset="./.github/assets/Sampo_logo_dark.svg" />
  <img alt="Sampo logo" src="./.github/assets/Sampo_logo_light.svg" />
</picture>

> Steers his mighty boat in safety, Through the perils of the whirlpool, Through the watery deeps and dangers. 

Automate changelogs, versioning, and publishing—even for monorepos across multiple package registries. Currently supported ecosystems: Rust ([Crates.io](https://crates.io))... And more coming soon!

Deeply inspired by [Changesets](https://github.com/changesets/changesets) and [Lerna](https://github.com/lerna/lerna). Enforce [Semantic Versioning](https://semver.org/) (SemVer) across all packages.

Don't know where to start? Check out Sampo's [Getting Started](./crates/sampo/README.md#getting-started) guide.

## Crates

Sampo is a monorepo that contains the following crates (Rust packages):

| Name                  | Description                                                   | Registry                                                                                                                                                               | README                                           |
| --------------------- | ------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------ |
| `sampo`               | CLI to manage changesets, versioning, and publishing          | <a href="https://crates.io/crates/sampo"><img alt="Sampo Crates.io Version" src="https://img.shields.io/crates/v/sampo"></a>                                           | [README](./crates/sampo/README.md)               |
| `sampo-core`          | Core logic, common types, and internal utilities              | <img alt="Sampo Core Crates.io Version" src="https://img.shields.io/badge/crates.io-unpublished-lightgrey">                                                            | [README](./crates/sampo-core/README.md)          |
| `sampo-github-bot`    | GitHub App to inspect PRs and request changesets              | <a href="https://crates.io/crates/sampo-github-bot"><img alt="Sampo GitHub Bot Crates.io Version" src="https://img.shields.io/crates/v/sampo-github-bot"></a>          | [README](./crates/sampo-github-bot/README.md)    |
| `sampo-github-action` | GitHub Action to automate the release and publishing process. | <a href="https://crates.io/crates/sampo-github-action"><img alt="Sampo GitHub Action Crates.io Version" src="https://img.shields.io/crates/v/sampo-github-action"></a> | [README](./crates/sampo-github-action/README.md) |

## Packages

Additionally, Sampo contains the following packages for diverse ecosystems:

| Name | Description | Registry | README |
| ---- | ----------- | -------- | ------ |

(*Coming soon*)
