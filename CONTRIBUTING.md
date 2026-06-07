# Contributing Guidelines

First, a huge **thank you** for dedicating your time to helping us improve Sampo ❤️

> [!Tip]
> **New to open source?** Check out [https://github.com/firstcontributions/first-contributions](https://github.com/firstcontributions/first-contributions) for helpful information on contributing

## Philosophy

Sampo is designed to be a helpful, reliable, and flexible tool that users can trust to manage changelogs, versioning, and publishing.

We want to make it easy to get started, with minimal configuration, sensible defaults, and automated workflows. At the same time, we want to provide rich configuration options, and flexible workflows to cover more advanced use cases. Finally, Sampo should be easy to opt in and opt out, with little to none assumptions, conventions to follow, or lock-ins.

We're also committed to fostering a welcoming and respectful community. Any issue, PR, or discussion that violates our [code of conduct](./CODE_OF_CONDUCT.md) will be deleted, and the authors will be **banned**.

## Before Opening Issues

- **Do not report security vulnerabilities publicly** (e.g., in issues), please refer to our [security policy](./SECURITY.md).
- **Do not create issues for questions about using Sampo.** Instead, ask on our [Discord](https://discord.com/invite/84pd4QtmzA).
- **For ideas or feature suggestions**, open a [feature request issue](https://github.com/bruits/sampo/issues/new?template=02-feature-request.yml) or chat about it first on [Discord](https://discord.com/invite/84pd4QtmzA).
- **Check for duplicates.** Look through existing issues to see if your topic has already been addressed.
- In general, provide as much detail as possible. No worries if it's not perfect, we'll figure it out together.

## Before submitting Pull Requests (PRs)

- **Check for duplicates.** Look through existing PRs to see if your changes have already been submitted.
- **Check Clippy warnings.** Run `cargo clippy --all --all-targets` to ensure your code adheres to Rust's best practices.
- **Run formatting.** Run `cargo fmt --all` to ensure your code is properly formatted.
- **Write and run tests.** If you're adding new functionality or fixing a bug, please include tests to cover it. Run `cargo test --all` to ensure all existing tests pass.
- **Write a changeset.** That's the whole point of Sampo! Run `sampo add` to create a new changeset file describing your changes.
- Prefer small, focused PRs that address a single issue or feature. Larger PRs can be harder to review, and can often be broken down into smaller, more manageable pieces.
- PRs don't need to be perfect. Submit your best effort, and we will gladly assist in polishing the work.

## Quality Guidelines

- Prefer self-documenting code first, with expressive names and straightforward logic. Comments should explain *why* (intent, invariants, trade-offs), not *how*. Variable and function names should be clear and descriptive, not cryptic abbreviations. Avoid hidden state and side effects.
- Tests should assert observable behavior (inputs/outputs, effects), not internal implementation details. Keep tests deterministic and independent of global state.
- For errors, use typed error enums in library crates (derived with `thiserror`). Per-crate `pub type Result<T>` aliases for ergonomic signatures. Add context at the boundary (CLI/action) rather than deep in core, keep library error messages concise.
- Prefer `?` propagation when possible, and reserve `.expect()`/`.unwrap()` for cases where failure is a programmer bug (e.g. hardcoded regex literals, test helpers).
- Document any new public APIs, configuration options, or user-facing changes in the relevant README files. If you're unsure where or how to document something, just ask and we'll help you out.
- We deeply value idiomatic, easy-to-maintain Rust code. Avoid code duplication when possible. And prefer clarity over cleverness, and small focused functions over dark magic.
- Explicit `use` imports for standard library types (e.g. `use std::collections::HashMap;`).

## Writing Changesets

Sampo helps users write better changelogs, let's lead by example with our own.

**Structure:**
1. **Breaking prefix (if applicable):** `**⚠️ breaking change:**`
2. **Ecosystem prefix (if applicable):** `In Python (PyPI) projects, ...` or `In Elixir (Hex) projects, ...`
3. **Verb:** `Added`, `Removed`, `Fixed`, `Changed`, `Deprecated`, or `Improved`.
4. **Description**.
5. **Usage example (optional):** A minimal snippet if it clarifies the change.

**Description guidelines:** concise (1-2 sentences), specific (mention the command/option/API), actionable (what changed, not why), user-facing (written for changelog readers), and in English. Don't detail internal implementation changes.

## Getting Started

Sampo is a fairly standard Rust project with a typical directory structure. It does not rely on any third-party build systems, complex configurations or dependencies in other languages. The only prerequisite is to have the latest stable version of [Rust](https://www.rust-lang.org/) installed.

Sampo is a Rust monorepo using [Cargo workspaces](https://doc.rust-lang.org/book/ch14-03-cargo-workspaces.html). It contains multiple crates (Rust packages) in the `crates/` directory:

### Sampo Core

`sampo-core` is a plain Rust library that owns the release planning, changelog generation, and configuration parsing shared by every other crate. It leans heavily on [`serde`](https://docs.rs/serde/latest/serde/) / [`toml`](https://docs.rs/toml/latest/toml/) for configuration parsing, [`semver`](https://docs.rs/semver/latest/semver/) for version math, and [`reqwest`](https://docs.rs/reqwest/latest/reqwest/) + [`tokio`](https://docs.rs/tokio/latest/tokio/) for the bits that fetch metadata or talk to registries. Most tests spin up throwaway workspaces—check the helpers in `src/release_tests.rs` and `src/workspace.rs` before reaching for manual temp-dir plumbing.

The `PackageAdapter` enum abstracts all ecosystem-specific operations: workspace discovery, manifest parsing, publishability checks, registry APIs, lockfile regen, etc. To add another ecosystem, create a new adapter in `src/adapters/<ecosystem>.rs`, add a variant to the enum, and update all match statements to delegate to your adapter.

### Sampo

`sampo` is the CLI façade on top of `sampo-core`. It wires commands together with [`clap`](https://docs.rs/clap/latest/clap/) and relies on [`dialoguer`](https://docs.rs/dialoguer/latest/dialoguer/) for interactive prompts, so changes to choices or flows should be exercised manually. Run commands locally with `cargo run -p sampo -- <command>` from the repository root; creating a scratch repo and trying `cargo run -p sampo -- init` is the quickest way to validate the `.sampo` layout, release flow, and any user-facing copy you touch.

### Sampo GitHub Bot

This crate is an [`axum`](https://docs.rs/axum/latest/axum/) web service that powers the GitHub App asking for missing changesets. Runtime configuration comes entirely from environment variables (`WEBHOOK_SECRET`, `GITHUB_APP_ID`, `GITHUB_PRIVATE_KEY`, plus optional `PORT`/`ADDR`). Local testing requires a real GitHub App and a tunnel (for example `ngrok http 3000`) so GitHub can reach your machine. The bot talks to GitHub through [`octocrab`](https://docs.rs/octocrab/latest/octocrab/) and signs payloads with [`jsonwebtoken`](https://docs.rs/jsonwebtoken/latest/jsonwebtoken/), so watch for rate limits and key handling whenever you refactor request logic. Deployment currently targets [Fly.io](https://fly.io)—keep an eye on `fly.toml` and secret names if you change configuration shape.

### Sampo GitHub Action

`sampo-github-action` ships the binary invoked by the composite action. It orchestrates releases by shelling out to git and calling GitHub APIs via [`reqwest`](https://docs.rs/reqwest/latest/reqwest/), so behaviour depends on having credentials and a clean git workspace. We provide integration tests that simulate a repository in temporary directories, but reproducing a full workflow locally is tricky: the action expects to run inside GitHub Actions with environment variables like `GITHUB_TOKEN`, a checked-out repo, and sometimes `cargo-binstall` tooling. Testing changes often means pushing branches to a test repo and observing the results in a real workflow run... Help is welcome to improve this experience!

### Packages

Beside the Cargo crates, the repository also ships the `sampo` CLI on npm so it can be installed without a Cargo toolchain. The `packages/` directory is a pnpm workspace whose only committed member is the JS shim (`packages/sampo`). At runtime the shim resolves the matching per-platform binary carrier (`@bruits/sampo-<os>-<arch>`) from its `optionalDependencies` and execs it, refusing to run on a version mismatch — so a carrier that was not published or synced at the shim's version surfaces as a runtime error for users on that platform (see `packages/sampo/bin/sampo.js`). Changesets only target `npm/sampo`: the carriers are not workspace members, and the `fixed` group in `.sampo/config.toml` pins `cargo/sampo` and `npm/sampo` together. Working on this npm distribution layer requires [Node.js](https://nodejs.org/) and [pnpm](https://pnpm.io/).

The per-platform carriers are not committed: at release time `scripts/publish-npm-platform-packages.sh` templates each carrier's `package.json`, drops in the CI-built binary, and publishes it straight to npm. The workflow runs this before Sampo publishes the shim, then rewrites the shim's `optionalDependencies` pins in the "Sync shim optionalDependencies" step, since Sampo cannot bump non-workspace packages itself. Adding a new platform therefore requires four coordinated changes: a new row in the `mappings` array of `scripts/publish-npm-platform-packages.sh`, a matching target in the build matrix of `.github/workflows/release.yml`, the package name in that workflow's "Sync shim optionalDependencies" `platforms` list, and an entry in the `PLATFORM_PACKAGES` map of `packages/sampo/bin/sampo.js`. The script fails loudly on a missing build artefact.

---

Thank you once again for contributing, we deeply appreciate all contributions, no matter how small or big.
