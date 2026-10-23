<picture>
  <source media="(prefers-color-scheme: dark)" srcset="./.github/assets/Sampo_logo_dark.svg" />
  <img alt="Sampo" src="./.github/assets/Sampo_logo_light.svg" />
</picture>

> Steers his mighty boat in safety, Through the perils of the whirlpool, Through the watery deeps and dangers.

Automate changelogs, versioning, and publishing—even for monorepos across multiple package registries. Currently supported ecosystems: Rust ([Crates](https://crates.io)), JavaScript/TypeScript ([npm](https://www.npmjs.com)), Elixir/Gleam/Erlang ([Hex](https://hex.pm)), Python ([PyPI](https://pypi.org)), PHP ([Packagist](https://packagist.org)), Java ([Maven Central](https://central.sonatype.com))... And more [coming soon](https://github.com/bruits/sampo/issues/104)!

Don't know where to start? Check out Sampo's [documentation](./crates/sampo/README.md) or join us on [Discord](https://discord.com/invite/84pd4QtmzA)!

## Acknowledgements

Sampo builds on the work of several open-source projects:

- [Changesets](https://github.com/changesets/changesets) and [Lerna](https://github.com/lerna/lerna), which inspired its changesets and monorepo release workflows, as its [alternatives](./crates/sampo/README.md#alternatives) explain
- Knope's [changesets](https://github.com/knope-dev/changesets) crate, which parses changeset files. [Knope](https://github.com/knope-dev/knope) is another Rust project inspired by Changesets, well worth a look
- [Semantic Versioning](https://semver.org/), whose `MAJOR.MINOR.PATCH` format and version bumps Sampo uses and enforces

Thank you to everyone who contributes, or would like to. [CONTRIBUTING.md](./CONTRIBUTING.md) is the place to start.

Sampo's name and quote honour the importance Finland took in my recent life.

Sampo is an open-source project born from [Bruits](https://bruits.org/), a Rust-focused collective 💛
