# Contributing Guidelines

## Philosophy

Sampo is a helpful, reliable, and flexible tool that users can trust to manage changelogs, versioning, and publishing.

It is easy to start with, through minimal configuration, sensible defaults, and automated workflows, while rich configuration options and flexible workflows cover more advanced use cases. It stays easy to opt in and out of, with few assumptions, conventions to follow, or lock-ins.

Issues, PRs, and discussions that break the [code of conduct](./CODE_OF_CONDUCT.md) are deleted and their authors banned.

## Before Opening Issues/PRs

- Report a security vulnerability privately, as the [security policy](./SECURITY.md) explains.
- Ask questions about using Sampo on [Discord](https://discord.com/invite/84pd4QtmzA). For an idea, open a [feature request](https://github.com/bruits/sampo/issues/new?template=02-feature-request.yml) or talk it through on Discord first.
- Look through existing issues and PRs, in case your topic is already covered.
- Give as much detail as you can. It doesn't have to be perfect, we'll figure it out together.
- Keep a PR small and focused on one issue or feature. Run the checks in [AGENTS.md](./AGENTS.md), add tests for what it adds or fixes, and a [changeset](#writing-changesets) for what users meet.
- Submit your best effort, and we'll gladly help polish it.

## Quality Guidelines

- Write self-documenting, idiomatic code, with clear names, plain logic, small functions, and no hidden state or side effects. Avoid duplication. Comments say why (intent, invariants, trade-offs), and never decorate or separate. The formatter and linter in [AGENTS.md](./AGENTS.md) settle style.
- In Rust, import standard library types explicitly (e.g. `use std::collections::HashMap;`), propagate errors with `?`, and keep `.expect()` and `.unwrap()` for programmer bugs (e.g. a hardcoded regex literal, a test helper). Library crates have a `thiserror` error enum and a `pub type Result<T>` alias, with concise messages in the core and context added by the CLI and the Action.
- Tests assert observable behaviour, stay deterministic, and share no global state. Unit tests sit in their module, and integration tests in their crate's `tests/`. New behaviour ships with tests, and touched behaviour gains them.
- Public APIs, configuration options, and user-facing behaviour are documented in the relevant README.
- Docs avoid ecosystem-specific notes, above all in general, widely-read sections. One is added only for a Sampo specificity that users of that ecosystem must know to avoid problems, kept brief, and opened like an ecosystem-specific changeset (e.g. `In PHP (Packagist) projects, …`).

## Writing Changesets

Sampo versions itself and writes its own changelogs from changesets, setting the example for its users. A change users meet (behaviour, public API, configuration, or output) ships with one. Install Sampo with `cargo install sampo`, then run `sampo add -p cargo/sampo cargo/sampo-core -b minor -m "Added …"`, or `cargo run -p sampo -- add …` from the repository.

While Sampo is 0.x, a feature or a breaking change is a `minor` bump, and a fix a `patch`.

A description starts with `Added`, `Removed`, `Fixed`, `Changed`, `Deprecated`, or `Improved`, and says in one or two sentences what changed for users, naming the command, option, or API, without the implementation. A breaking change opens with `**⚠️ breaking change:**`, and a change for one ecosystem with its name, after any breaking prefix (e.g. `In Python (PyPI) projects, added …`). A minimal usage example can follow when it clarifies the change.

Changesets wait in `.sampo/changesets/`. On `main`, the Release & Publish workflow runs Sampo's own GitHub Action, which gathers them into a release PR, and merging that PR publishes the crates and npm packages. The same workflow deploys the GitHub App when a release changes it.

## Getting Started

Sampo is a Rust monorepo using [Cargo workspaces](https://doc.rust-lang.org/book/ch14-03-cargo-workspaces.html), with a [pnpm workspace](https://pnpm.io/workspaces) for the npm packages of the CLI. It needs the stable [Rust](https://www.rust-lang.org/) toolchain, which `rust-toolchain.toml` selects. [Node.js](https://nodejs.org/) LTS, and [pnpm](https://pnpm.io/) at the version `package.json` pins, are needed only for `packages/` and `pnpm run check`. The commands are in [AGENTS.md](./AGENTS.md), and each package's README ends with what to know before working on it.

| Path | What it holds |
| --- | --- |
| `crates/sampo-core` | Release planning, changelog generation, configuration parsing, and one adapter per ecosystem, shared by the other crates |
| `crates/sampo` | The CLI, on top of `sampo-core` |
| `crates/sampo-github-action` | The GitHub Action, whose binary prepares release PRs and publishes |
| `crates/sampo-github-bot` | The GitHub App that asks pull requests for missing changesets, deployed to [Fly.io](https://fly.io) |
| `packages/sampo` | The npm shim that runs the CLI binary for the user's platform |
| `packages/sampo-<os>-<arch>` | One platform package per OS and architecture, published with the CLI binary as `@bruits/sampo-<os>-<arch>` |
| `scripts/` | The release script that publishes the npm platform packages |
| `.sampo/` | Sampo's own configuration and pending changesets |

`sampo`, `sampo-github-action`, and `sampo-github-bot` depend on `sampo-core`, and on nothing else in the workspace.
