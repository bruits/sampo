# Sampo

Automate changelogs, versioning, and publishing—even for monorepos across multiple registries.

## Getting Started

Install Sampo using Cargo:

```bash
cargo install sampo
```

Initialize Sampo in your repository:

```bash
sampo init
```

### Main concepts

**Version bump**: Sampo enforces [Semantic Versioning](https://semver.org/) (SemVer) to indicate the nature of changes in each release. Versions follow the `MAJOR.MINOR.PATCH` format where:
- **patch**: Bug fixes and backwards-compatible changes
- **minor**: New features that are backwards-compatible  
- **major**: Breaking changes that are not backwards-compatible

For example, a user can safely update from version `1.2.3` to `1.2.4` (patch) or `1.3.0` (minor), but should review changes before updating to `2.0.0` (major).

**Changeset**: A markdown file describing what changed and how to version affected packages. Each changeset specifies which packages to bump and if it should be a patch, minor, or major update.

```
---
packages:
  - example
release: minor
---
An helpful description of the changes.
```

**Changelog**: Automatically generated file listing all changes for each package version. Sampo consumes changesets to build comprehensive changelogs with semantic versioning.

**Release**: The process of consuming changesets to bump package versions, update changelogs, and create git tags. Sampo works seamlessly with **monorepos** containing multiple packages and supports publishing to **multiple registries** across different ecosystems.

### Usage

**Creating a changeset**: Use `sampo add` to create a new changeset file. The command guides you through selecting packages and describing changes. Use [Sampo GitHub bot](https://github.com/bruits/sampo/tree/main/crates/sampo-github-bot) to get reminders on each PR without a changeset.

**Consuming changesets**: Run `sampo release` to process all pending changesets, bump package versions, and update changelogs. This can be automated in CI/CD pipelines using [Sampo GitHub Action](../sampo-github-action).

As long as the release is not finalized, you can continue to add changesets and re-run the `sampo release` command. Sampo will update package versions and pending changelogs accordingly.

**Publishing**: After running `sampo release`, use `sampo publish` to publish updated packages to their respective registries and tag the current versions. This step can also be automated in CI/CD pipelines using [Sampo GitHub Action](../sampo-github-action).

### Sampo folder structure

`sampo init` creates a `.sampo` directory at your repository root:

```
.sampo/
├─ changesets/ <- Individual changeset files describing pending changes
├─ config.toml <- Sampo configuration (package settings, registry options)
└─ README.md <- A copy of this documentation
```

### Sampo configuration

*Work in progress*

## Commands

All commands should be run from the root of the repository:

| Command         | Description                                                               |
| --------------- | ------------------------------------------------------------------------- |
| `sampo help`    | Show commands or the help of the given subcommand(s)                      |
| `sampo init`    | Initialize Sampo in the current repository                                |
| `sampo add`     | Create a new changeset                                                    |
| `sampo release` | Consume changesets, and prepare release(s) (bump versions and changelogs) |
| `sampo publish` | Publish packages to registries and tag current versions                   |

For detailed command options, use `sampo help <command>` or `sampo <command> --help`.
