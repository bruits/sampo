# Sampo

Automate changelogs, versioning, and publishing—even for monorepos across multiple package registries. Currently supported ecosystems: Rust ([Crates](https://crates.io)), JavaScript/TypeScript ([npm](https://www.npmjs.com)), Elixir/Gleam/Erlang ([Hex](https://hex.pm)), Python ([PyPI](https://pypi.org)), PHP ([Packagist](https://packagist.org)), Java ([Maven Central](https://central.sonatype.com)), Ruby ([RubyGems](https://rubygems.org))... And more [coming soon](https://github.com/bruits/sampo/issues/104)!

Not sure what Sampo is? Don't know where to start? Check out Sampo's [repository](https://github.com/bruits/sampo).

## Development

This package is a shim that runs the binary of the `@bruits/sampo-<os>-<arch>` package matching the user's platform, and refuses to run when their versions differ (see `bin/sampo.js`). Working on it needs [Node.js](https://nodejs.org/) and [pnpm](https://pnpm.io/).

The platform packages, in `packages/sampo-<os>-<arch>/`, hold no committed binary. At release time, `scripts/publish-npm-platform-packages.sh` publishes each one with its CI-built binary, and stops on a missing binary or a platform package whose version differs from the shim's. The `fixed` group of `.sampo/config.toml` releases them with `cargo/sampo` and `npm/sampo`, so a changeset never names one. Adding a platform takes a new package in `packages/sampo-<os>-<arch>/` at the shim's version, listed in the `fixed` group and in the shim's `optionalDependencies`, plus a row in the script's `mappings`, a target in the build matrix of `.github/workflows/release.yml`, and an entry in `PLATFORM_PACKAGES` in `bin/sampo.js`.

Every npm package is published through [npm trusted publishing](https://docs.npmjs.com/trusted-publishers), and trusts the `release.yml` workflow of `bruits/sampo` in the `sampo` environment, with direct `npm publish` allowed (off by default). Trusted publishing cannot create a package, so a new platform package is published once by hand, and its trusted publisher added at most two days before the next release, since npm expires one that has not published within two days.

Refer to [CONTRIBUTING.md](https://github.com/bruits/sampo/blob/main/CONTRIBUTING.md#getting-started) for development setup and workflow details.
