# Sampo

Automate changelogs, versioning, and publishing—even for monorepos across multiple package registries. Currently supported ecosystems: Rust ([Crates](https://crates.io)), JavaScript/TypeScript ([npm](https://www.npmjs.com)), Elixir/Gleam/Erlang ([Hex](https://hex.pm)), Python ([PyPI](https://pypi.org)), PHP ([Packagist](https://packagist.org)), Java ([Maven Central](https://central.sonatype.com))... And more [coming soon](https://github.com/bruits/sampo/issues/104)!

Not sure what Sampo is? Don't know where to start? Check out Sampo's [repository](https://github.com/bruits/sampo).

## Development

This package is a shim that runs the binary of the `@bruits/sampo-<os>-<arch>` package matching the user's platform, and refuses to run when their versions differ (see `bin/sampo.js`). Working on it needs [Node.js](https://nodejs.org/) and [pnpm](https://pnpm.io/). How the platform packages are released is in [Writing Changesets](https://github.com/bruits/sampo/blob/main/CONTRIBUTING.md#writing-changesets).

Refer to [CONTRIBUTING.md](https://github.com/bruits/sampo/blob/main/CONTRIBUTING.md#getting-started) for development setup and workflow details.
