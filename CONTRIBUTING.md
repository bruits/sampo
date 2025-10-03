# Contributing Guidelines

First, a huge **thank you** for dedicating your time to helping us improve Sampo ❤️

> [!Tip]
>
> **New to open source?** Check out [https://github.com/firstcontributions/first-contributions](https://github.com/firstcontributions/first-contributions) for helpful information on contributing

## Philosophy

Sampo tries to be an helpful, reliable tool that users can trust to manage changelogs, versioning, and publishing.

We want to make it easy to get started, with minimal configuration, sensible defaults, and automated workflows. At the same time, we want to provide rich configuration options (at least to disable defaults) and flexible workflows to cover more advanced use cases. Reliability, predictability, and transparency are key values.

We’re also committed to fostering a welcoming and respectful community. Any issue, PR, or discussion that violates our [code of conduct](https://github.com/bruits/sampo/blob/main/CODE_OF_CONDUCT.md) will be deleted, and the authors will be **banned**.

## Before Opening Issues

- **Do not report security vulnerabilities publicly** (e.g., in issues or discussions), please refer to our [security policy](https://github.com/bruits/sampo/blob/main/SECURITY.md).
- **Do not create issues for questions about using Sampo.** Instead, ask your question in our [GitHub Discussions](https://github.com/bruits/sampo/discussions/categories/q-a).
- **Do not create issues for ideas or suggestions.** Instead, share your thoughts in our [GitHub Discussions](https://github.com/bruits/sampo/discussions/categories/ideas).
- **Check for duplicates.** Look through existing issues and discussions to see if your topic has already been addressed.
- In general, provide as much detail as possible. No worries if it's not perfect, we'll figure it out together.

## Before submitting Pull Requests (PRs)

- **Check for duplicates.** Look through existing PRs to see if your changes have already been submitted.
- **Check Clippy warnings.** Run `cargo clippy --all --all-targets` to ensure your code adheres to Rust's best practices.
- **Run formatting.** Run `cargo fmt --all` to ensure your code is properly formatted.
- **Write and run tests.** If you're adding new functionality or fixing a bug, please include tests to cover it. Run `cargo test --all` to ensure all existing tests pass.
- **Write a changeset.** That's the whole point of Sampo! Run `sampo add` to create a new changeset file describing your changes.
- Prefer small, focused PRs that address a single issue or feature. Larger PRs can be harder to review, and can often be broken down into smaller, more manageable pieces.
- We deeply value idiomatic, expressive, easy-to-maintain Rust code. Avoid code duplication when possible. And prefer clarity over cleverness, and small focused functions over dark magic 🧙‍♂️
- **PRs don't need to be perfect.** Submit your best effort, and we will gladly assist in polishing the work.

## Getting started

Sampo is a fairly standard Rust project with a typical directory structure. It does not rely on any third-party build systems, complex configurations or dependencies in other languages. The only prerequisite is to have the latest stable version of [Rust](https://www.rust-lang.org/) installed.

Sampo is a Rust monorepo using [Cargo workspaces](https://doc.rust-lang.org/book/ch14-03-cargo-workspaces.html). It contains multiple crates (Rust packages) in the `crates/` directory:

### Sampo Core

`sampo-core` is a plain Rust library that owns the release planning, changelog generation, and configuration parsing shared by every other crate. It leans heavily on [`serde`](https://docs.rs/serde/latest/serde/) / [`toml`](https://docs.rs/toml/latest/toml/) for configuration parsing, [`semver`](https://docs.rs/semver/latest/semver/) for version math, and [`reqwest`](https://docs.rs/reqwest/latest/reqwest/) + [`tokio`](https://docs.rs/tokio/latest/tokio/) for the bits that fetch metadata or talk to registries. Most tests spin up throwaway workspaces—check the helpers in `src/release_tests.rs` and `src/workspace.rs` before reaching for manual temp-dir plumbing.

### Sampo

`sampo` is the CLI façade on top of `sampo-core`. It wires commands together with [`clap`](https://docs.rs/clap/latest/clap/) and relies on [`dialoguer`](https://docs.rs/dialoguer/latest/dialoguer/) for interactive prompts, so changes to choices or flows should be exercised manually. Run commands locally with `cargo run -p sampo -- <command>` from the repository root; creating a scratch repo and trying `cargo run -p sampo -- init` is the quickest way to validate the `.sampo` layout, release flow, and any user-facing copy you touch.

### Sampo GitHub Bot

This crate is an [`axum`](https://docs.rs/axum/latest/axum/) web service that powers the GitHub App asking for missing changesets. Runtime configuration comes entirely from environment variables (`WEBHOOK_SECRET`, `GITHUB_APP_ID`, `GITHUB_PRIVATE_KEY`, plus optional `PORT`/`ADDR`). Local testing requires a real GitHub App and a tunnel (for example `ngrok http 3000`) so GitHub can reach your machine. The bot talks to GitHub through [`octocrab`](https://docs.rs/octocrab/latest/octocrab/) and signs payloads with [`jsonwebtoken`](https://docs.rs/jsonwebtoken/latest/jsonwebtoken/), so watch for rate limits and key handling whenever you refactor request logic. Deployment currently targets [Fly.io](https://fly.io)—keep an eye on `fly.toml` and secret names if you change configuration shape.

### Sampo GitHub Action

`sampo-github-action` ships the binary invoked by the composite action. It orchestrates releases by shelling out to git and calling GitHub APIs via [`reqwest`](https://docs.rs/reqwest/latest/reqwest/), so behaviour depends on having credentials and a clean git workspace. We provide integration tests that simulate a repository in temporary directories, but reproducing a full workflow locally is tricky: the action expects to run inside GitHub Actions with environment variables like `GITHUB_TOKEN`, a checked-out repo, and sometimes `cargo-binstall` tooling. Testing changes often means pushing branches to a test repo and observing the results in a real workflow run... Help is welcome to improve this experience!

---

Thank you once again for contributing, we deeply appreciate all contributions, no matter how small or big.
