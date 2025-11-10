<picture>
  <source media="(prefers-color-scheme: dark)" srcset="./.github/assets/Sampo_logo_dark.svg" />
  <img alt="Sampo logo" src="./.github/assets/Sampo_logo_light.svg" />
</picture>

> Steers his mighty boat in safety, Through the perils of the whirlpool, Through the watery deeps and dangers. 

Automate changelogs, versioning, and publishing—even for monorepos across multiple package registries. Currently supported ecosystems: Rust ([Crates](https://crates.io)), JavaScript/TypeScript ([npm](https://www.npmjs.com)), Elixir ([Hex](https://hex.pm))... And more [coming soon](https://github.com/bruits/sampo/issues/104)!

Don't know where to start? Check out Sampo's [documentation](./crates/sampo/README.md).

## Crates

Sampo is a monorepo that contains the following crates (Rust packages):

| Name                  | Description                                                   | Registry                                                                                                                                                               | README                                           |
| --------------------- | ------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------ |
| `sampo`               | CLI to manage changesets, versioning, and publishing          | <a href="https://crates.io/crates/sampo"><img alt="Sampo Crates.io Version" src="https://img.shields.io/crates/v/sampo"></a>                                           | [README](./crates/sampo/README.md)               |
| `sampo-core`          | Core logic, common types, and internal utilities              | <a href="https://crates.io/crates/sampo-core"><img alt="Sampo Core Crates.io Version" src="https://img.shields.io/crates/v/sampo-core"></a>                            | [README](./crates/sampo-core/README.md)          |
| `sampo-github-bot`    | GitHub App to inspect PRs and request changesets              | <a href="https://crates.io/crates/sampo-github-bot"><img alt="Sampo GitHub Bot Crates.io Version" src="https://img.shields.io/crates/v/sampo-github-bot"></a>          | [README](./crates/sampo-github-bot/README.md)    |
| `sampo-github-action` | GitHub Action to automate the release and publishing process. | <a href="https://crates.io/crates/sampo-github-action"><img alt="Sampo GitHub Action Crates.io Version" src="https://img.shields.io/crates/v/sampo-github-action"></a> | [README](./crates/sampo-github-action/README.md) |

## Acknowledgements

Sampo is deeply inspired by [Changesets](https://github.com/changesets/changesets) and [Lerna](https://github.com/lerna/lerna), but made with Rust and designed for multi-ecosystems monorepos. Read more about Sampo's [alternatives](./crates/sampo/README.md#alternatives).

Sampo uses Knope's [changesets](https://github.com/knope-dev/changesets) crate to parse changeset files. [Knope](https://github.com/knope-dev/knope) is another Rust project inspired by Changesets, we highly recommend checking it out!

Sampo uses and enforces [Semantic Versioning](https://semver.org/) (SemVer), like their standard `MAJOR.MINOR.PATCH` version format and the "Version Bumps" concepts.

Sampo is an open-source project born from [Bruits](https://bruits.org/), a Rust-focused collective 💛
