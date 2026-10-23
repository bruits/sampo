# Contributing Guidelines

## Philosophy

Sampo is a helpful, reliable, and flexible tool that users can trust to manage changelogs, versioning, and publishing.

It is easy to start with, through minimal configuration, sensible defaults, and automated workflows, while rich configuration options and flexible workflows cover more advanced use cases. It stays easy to opt in and out of, with few assumptions, conventions to follow, or lock-ins.

Issues, PRs, and discussions that break the [code of conduct](./CODE_OF_CONDUCT.md) are deleted and their authors banned.

## Before Opening Issues

- Report a security vulnerability privately, as the [security policy](./SECURITY.md) explains.
- Ask questions about using Sampo on [Discord](https://discord.com/invite/84pd4QtmzA), and open a [feature request](https://github.com/bruits/sampo/issues/new?template=02-feature-request.yml) for an idea.

## Quality Guidelines

- Write self-documenting, idiomatic code, with clear names, plain logic, small functions, and no hidden state or side effects. Avoid duplication. Comments say why (intent, invariants, trade-offs), and never decorate or separate. The formatter and linter in [AGENTS.md](./AGENTS.md) settle style.
- In Rust, import standard library types explicitly (e.g. `use std::collections::HashMap;`), propagate errors with `?`, and keep `.expect()` and `.unwrap()` for programmer bugs (e.g. a hardcoded regex literal, a test helper). Library crates have a `thiserror` error enum and a `pub type Result<T>` alias, with concise messages in the core and context added by the CLI and the Action.
- Tests assert observable behaviour, stay deterministic, and share no global state. Unit tests sit in their module, and integration tests in their crate's `tests/`. New behaviour ships with tests, and touched behaviour gains them.
- Public APIs, configuration options, and user-facing behaviour are documented in the relevant README.
- Docs avoid ecosystem-specific notes, above all in general, widely-read sections. One is added only for a Sampo specificity that users of that ecosystem must know to avoid problems, kept brief, and opened like an ecosystem-specific changeset (e.g. `In PHP (Packagist) projects, …`).

## Writing Changesets

Sampo versions itself and writes its own changelogs from changesets, setting the example for its users. A change users meet (behaviour, public API, configuration, or output) ships with one. Install Sampo with `cargo install sampo`, then run `sampo add -p cargo/sampo cargo/sampo-core -b minor -m "Added …"`, or `cargo run -p sampo -- add …` from the repository. Packages are named with their registry, since `sampo` exists on both crates.io and npm. Internal refactors, tests, and docs need none.

While Sampo is 0.x, a feature or a breaking change is a `minor` bump, and a fix a `patch`.

A description starts with `Added`, `Removed`, `Fixed`, `Changed`, `Deprecated`, or `Improved`, and says in one or two sentences what changed for users, naming the command, option, or API, without the implementation. A breaking change opens with `**⚠️ breaking change:**`, and a change for one ecosystem with its name, after any breaking prefix (e.g. `In Python (PyPI) projects, added …`). A minimal usage example can follow when it clarifies the change.

Changesets wait in `.sampo/changesets/`. On `main`, the Release & Publish workflow runs Sampo's own GitHub Action, which gathers them into a release PR, and merging that PR publishes the crates and npm packages. The same workflow deploys the GitHub App when a release changes it.

On npm, the `sampo` shim runs the binary of the `@bruits/sampo-<os>-<arch>` carrier matching the user's platform. The `fixed` group in `.sampo/config.toml` releases the carriers with `cargo/sampo` and `npm/sampo`, and Sampo keeps the shim's `optionalDependencies` pins in step, so a changeset never names a carrier. Carriers commit no binary. At release time, `scripts/publish-npm-platform-packages.sh` publishes each with its CI-built binary, then Sampo skips them as already published and ships the shim. The script stops on a missing build artefact or a carrier whose version differs from the shim's. Adding a platform means a carrier `package.json` at the shim's version, its entry in the `fixed` group and in the shim's `optionalDependencies`, a row in the script's `mappings`, a target in the build matrix of `.github/workflows/release.yml`, and an entry in `PLATFORM_PACKAGES` in `packages/sampo/bin/sampo.js`.

Both the script and Sampo publish through [npm trusted publishing](https://docs.npmjs.com/trusted-publishers). Each package on npmjs.com trusts the `release.yml` workflow of `bruits/sampo` in the `sampo` environment, with direct `npm publish` allowed (off by default). Trusted publishing cannot create a package, so a new carrier is published once by hand. Its trusted publisher is added at most two days before the next release, since npm expires one that has not published within two days.

## Getting Started

Sampo is a Rust monorepo using [Cargo workspaces](https://doc.rust-lang.org/book/ch14-03-cargo-workspaces.html), with a [pnpm workspace](https://pnpm.io/workspaces) for the npm packages of the CLI. It needs the stable [Rust](https://www.rust-lang.org/) toolchain, which `rust-toolchain.toml` selects. [Node.js](https://nodejs.org/) LTS, and [pnpm](https://pnpm.io/) at the version `package.json` pins, are needed only for `packages/` and `pnpm run check`. The commands are in [AGENTS.md](./AGENTS.md), and each package's README ends with what to know before working on it.

| Path | What it holds |
| --- | --- |
| `crates/sampo-core` | Release planning, changelog generation, configuration parsing, and one adapter per ecosystem, shared by the other crates |
| `crates/sampo` | The CLI, on top of `sampo-core` |
| `crates/sampo-github-action` | The GitHub Action, whose binary prepares release PRs and publishes |
| `crates/sampo-github-bot` | The GitHub App that asks pull requests for missing changesets, deployed to [Fly.io](https://fly.io) |
| `packages/sampo` | The npm shim that runs the CLI binary for the user's platform |
| `packages/sampo-<os>-<arch>` | One binary carrier per platform, published as `@bruits/sampo-<os>-<arch>` |
| `scripts/` | The release script that publishes the npm carriers |
| `.sampo/` | Sampo's own configuration and pending changesets |

`sampo`, `sampo-github-action`, and `sampo-github-bot` depend on `sampo-core`, and on nothing else in the workspace.
