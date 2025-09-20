# Sampo

Automate changelogs, versioning, and publishing—even for monorepos across multiple package registries. Currently supported ecosystems: Rust ([Crates.io](https://crates.io))... And more coming soon!

## Getting Started

Install Sampo using Cargo:

```bash
cargo install sampo
```

Initialize Sampo in your repository:

```bash
sampo init
```

This command creates a `.sampo` directory at your repository root:

```
.sampo/
├─ changesets/ <- Individual changeset files describing pending changes
├─ config.toml <- Sampo configuration (package settings, registry options)
└─ README.md <- Quick links to the online documentation
```

### Main concepts

#### Versioning

Sampo enforces [Semantic Versioning](https://semver.org/) (SemVer) to indicate the nature of changes in each release. Versions follow the `MAJOR.MINOR.PATCH` format with three bump levels:
- **patch**: Bug fixes and backwards-compatible changes
- **minor**: New features that are backwards-compatible
- **major**: Breaking changes that are not backwards-compatible

For example, a user can safely update from version `1.2.3` to `1.2.4` (patch) or `1.3.0` (minor), but should review changes before updating to `2.0.0` (major).

Finally, Sampo follows [SemVer §9](https://semver.org/#spec-item-9) for pre-release identifiers such as `1.8.0-alpha` or `2.0.0-rc.3`. While a pre-release stays within its implied level (patch for `x.y.z-prerelease`, minor for `x.y.0-prerelease`, major for `x.0.0-prerelease`), we only bump the numeric suffix (`alpha` → `alpha.1` -> `alpha.2` -> etc). If a higher bump is required, the base version advances first and keeps the same tag (`1.8.0-alpha.2` + major → `2.0.0-alpha`). Purely numeric tags like `1.0.0-1` are rejected.

#### Changesets

A markdown file describing what changed, which packages are affected, and the type of version bump required :

```
---
"example": minor
---

A helpful description of the change, to be read by your users.
```

Pending changesets are stored in the `.sampo/changesets` directory.

#### Changelog

A generated file listing all changes for each package version released:

```
# Example

## 0.2.0

### Minor changes

- [abcdefg](link/to/commit) A helpful description of the changes. — Thanks @user!

## 0.1.1

### Patch changes

- [hijklmn](link/to/commit) A brief description of the fix. — Thanks @first-time-contributor for their first contribution!

... previous entries ...
```

Each published package has its own `CHANGELOG.md` file at the package root.

### Usage

#### Add a changeset

Use `sampo add` to create a new changeset file. The command guides you through selecting packages and describing changes. Use [Sampo GitHub bot](https://github.com/bruits/sampo/tree/main/crates/sampo-github-bot) to get reminders on each PR without a changeset.

#### Prepare a release

Run `sampo release` to process all pending changesets, bump package versions, and update changelogs. This can be automated in CI/CD pipelines using [Sampo GitHub Action](../sampo-github-action).

As long as the release is not finalized, you can continue to add changesets and re-run the `sampo release` command. Sampo will update package versions and pending changelogs accordingly.

#### Publish packages

Finally, run `sampo publish` to publish updated packages to their respective registries and tag the current versions. This step can also be automated in CI/CD pipelines using [Sampo GitHub Action](../sampo-github-action).

## Configuration

The `.sampo/config.toml` file allows you to customize Sampo's behavior. Example configuration:

```toml
[git]
default_branch = "main"
release_branches = ["3.x"]

[github]
repository = "owner/repo"

[changelog]
show_commit_hash = true
show_acknowledgments = true

[packages]
ignore_unpublished = false
ignore = [
  "package-a",
  "internal-*",
  "examples/*"
]
fixed = [["pkg-a", "pkg-b"], ["pkg-c", "pkg-d"]]
linked = [["pkg-e", "pkg-f"], ["pkg-g", "pkg-h"]]
```

### `[git]` section

`default_branch`: Name of the primary release branch (default: `"main"`).

`release_branches`: Additional branch names that should behave like long-lived release lines. The default branch is always included automatically, so this list only needs the extra branches (e.g. `"3.x"`, `"4.0"`).

At runtime you can override the detected branch with the `SAMPO_RELEASE_BRANCH` environment variable, which is useful for local testing or custom CI setups.

### `[github]` section

`repository`: The GitHub repository slug in the format "owner/repo". If not set, Sampo uses the `GITHUB_REPOSITORY` environment variable or attempts to detect it from the `origin` git remote. This setting is used to enrich changelog messages with commit hash links and author acknowledgments, especially for first-time contributors.

### `[changelog]` section

`show_commit_hash`: Whether to include commit hash links in changelog entries (default: `true`). When enabled, changelog entries include clickable commit hash links that point to the commit on GitHub.

`show_acknowledgments`: Whether to include author acknowledgments in changelog entries (default: `true`). When enabled, changelog entries include author acknowledgments with special messages for first-time contributors.

### `[packages]` section

You can ignore certain packages, so they do not appear in the CLI commands, changesets, releases, or publishing steps. This is useful for packages that are not meant to be published or versioned, such as internal tools, examples, or documentation packages. Changesets targeting only ignored packages are left unconsumed.

`ignore_unpublished`: If `true` (default: `false`), ignore every package configured as not publishable. For example, `publish = false` in `Cargo.toml` for Rust packages.

`ignore`: A list of glob-like patterns to ignore packages by name or relative path. `*` matches any sequence. Examples:
- `internal-*`: ignores packages with names like `internal-tool`.
- `examples/*`: ignores any package whose relative path starts with `examples/`.
- `package-a`: ignores a package named exactly `package-a`.

Sampo detects packages within the same repository that depend on each other and automatically manages their versions. By default, dependent packages are automatically patched when a workspace dependency is updated. For example: if `a@0.1.0` depends on `b@0.1.0` and `b` is updated to `0.2.0`, then `a` will be automatically bumped to `0.1.1` (patch). If `a` needs a major or minor change due to `b`'s update, it should be explicitly specified in a changeset. Some options allow customizing this behavior:

`fixed`: An array of dependency groups (default: `[]`) where packages in each group are bumped together with the same version level. Each group is an array of package names. When any package in a group is updated, all other packages in the same group receive the same version bump, regardless of actual dependencies. For example: if `fixed = [["a", "b"], ["c", "d"]]` and `a` is updated to `2.0.0` (major), then `b` will also be bumped to `2.0.0`, but `c` and `d` remain unchanged.

`linked`: An array of dependency groups (default: `[]`) where affected packages and their dependents are bumped together using the highest bump level in the group. Each group is an array of package names. When any package in a group is updated, all packages in the same group that are affected or have workspace dependencies within the group receive the highest version bump level from the group. For example: if `linked = [["a", "b"]]` where `a` depends on `b`, when `b` is updated to `2.0.0` (major), then `a` will also be bumped to `2.0.0`. If `a` is later updated to `2.1.0` (minor), `b` remains at `2.0.0` since it's not affected. Finally, if `b` has a patch update, both `a` and `b` will be bumped with patch level since it's the highest bump in the group.

Note: Packages cannot appear in both `fixed` and `linked` configurations.

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
