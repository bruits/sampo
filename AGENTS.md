# Agents Guide

Sampo is a Rust monorepo, with a CLI, a GitHub Action, and a GitHub App to automate changelogs, versioning, and publishing across package registries 🧭

## Useful Commands

```sh
cargo fmt --all
cargo clippy --all-targets --all-features -- -D warnings        # lint, as CI does
cargo clippy --all-targets --no-default-features -- -D warnings # lint without default features, as CI does
cargo test                                                       # test, as CI does
cargo test -p sampo-core                                         # only one crate's tests
cargo run -p sampo -- <command>                                  # run the CLI from the repository
pnpm run check                                                   # everything CI checks, in CI order
```

## Useful Resources

- In [CONTRIBUTING.md](./CONTRIBUTING.md): [Quality Guidelines](./CONTRIBUTING.md#quality-guidelines) applies to agents and humans equally, [Getting Started](./CONTRIBUTING.md#getting-started) helps you understand the project structure, and [Philosophy](./CONTRIBUTING.md#philosophy) is the project's north star.
- The [CLI README](./crates/sampo/README.md) is Sampo's user documentation. Per-package READMEs (e.g. [sampo-github-action](./crates/sampo-github-action/README.md)) document the public API, and stay concise and user-facing.
- [GitHub](https://github.com/bruits/sampo) issues and PRs hold implementation details, design discussions, and technical decisions.

## Agent Guardrails

- Common errors live in `sampo-core`'s `errors.rs`, and crate-specific errors in each crate's `error.rs`.
- Match the project's structure, naming, and style, with no parallel pattern or duplication. Add a dependency only with a reason, after the standard library and existing utilities.
- Add no documentation file to explain implementation, and change CI/CD only when asked.
- Code, comments, docs, commit messages, and user-facing output are in English.
- A change users meet ships with a Sampo changeset, see [CONTRIBUTING.md](./CONTRIBUTING.md#writing-changesets).
