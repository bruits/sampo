# Sampo

Automate changelogs, versioning, and publishing—even for monorepos across multiple package registries. Currently supported ecosystems: Rust ([Crates](https://crates.io)), JavaScript/TypeScript ([npm](https://www.npmjs.com)), Elixir ([Hex](https://hex.pm))... And more [coming soon](https://github.com/bruits/sampo/issues/104)!

**In a nutshell,** Sampo is a CLI, a GitHub App, and a GitHub Action, that automatically detects packages in your repository, and use changesets (markdown files describing changes explicitly) to bump versions (in SemVer format), generate changelogs (human-readable files listing changes), and publish packages (to their respective registries). It's designed to be easy to opt-in and opt-out, with minimal configuration required, sensible defaults, and no assumptions/constraints on your workflow (except using SemVers).

If you’ve ever struggled with keeping user-facing changelogs updated, coordinating version bumps across dependent packages, or automating your publishing process... Sampo might be the tool you were looking for!

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
├─ prerelease/ <- Changesets preserved while shipping pre-release builds (optional)
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

Pre-release versions are supported using [SemVer §9](https://semver.org/#spec-item-9) conventions (e.g., `1.0.0-alpha`, `2.1.0-beta.2`, `3.0.0-rc.5`, etc). While a pre-release stays within its implied level (patch for `x.y.z-prerelease`, minor for `x.y.0-prerelease`, major for `x.0.0-prerelease`), we only bump the numeric suffix (`alpha` → `alpha.1` -> `alpha.2` -> etc). If a higher bump is required, the base version advances and numeric suffix is reset (`1.8.0-alpha.2` + major → `2.0.0-alpha`).

#### Changesets

A markdown file describing what changed, which packages are affected, and the type of version bump required :

```
---
cargo/example: minor
npm/web-app: patch
---

A helpful description of the change, to be read by your users.
```

Packages are referenced by their canonical identifier (`<ecosystem>/<name>`). Pending changesets are stored in the `.sampo/changesets` directory.

#### Changelog

At the root of each published package, a human-readable file listing all changes for each released version. Example:

```
# Example

## 0.2.0 — 2024-06-20

### Minor changes

- [abcdefg](link/to/commit) A helpful description of the changes. — Thanks @user!

## 0.1.1 — 2024-05-12

### Patch changes

- [hijklmn](link/to/commit) A brief description of the fix. — Thanks @first-time-contributor for their first contribution!

... previous entries ...
```

Sampo generates changelog entries from consumed changesets and enriches them with commit hash links and author acknowledgments (can be disabled in config). 

Any intro content or custom main header before the first `##` section is preserved. You can also manually edit the previously released entries, and Sampo will keep them intact.

### Usage

#### 1. Add changesets

Use `sampo add` to create a new changeset file. The command guides you through selecting packages and describing changes. Use [Sampo GitHub bot](https://github.com/bruits/sampo/tree/main/crates/sampo-github-bot) to get reminders on each PR without a changeset.

#### 2. Prepare a release

Run `sampo release` to process all pending changesets, bump package versions, and update changelogs. This can be automated in CI/CD pipelines using [Sampo GitHub Action](../sampo-github-action).

As long as the release is not finalized, you can continue to add changesets and re-run the `sampo release` command. Sampo will update package versions and pending changelogs accordingly.

#### 3. Publish packages

Finally, run `sampo publish` to publish updated packages to their respective registries and tag the current versions. This step can also be automated in CI/CD pipelines using [Sampo GitHub Action](../sampo-github-action).

> [!IMPORTANT]
> Always run `sampo release` before `sampo publish` to ensure versions are properly updated.

> [!WARNING]
> Publishing adapters call the native tooling (`cargo`, `npm`, `mix`, …) directly. In local or CI environments, make sure those tools are installed and accessible via your `PATH`.

#### Pre-release versions

Run `sampo pre` to manage pre-release versions for one or more packages.

While in pre-release mode, you can continue to add changesets and run `sampo release` and `sampo publish` as usual, Sampo preserves the consumed changesets in `.sampo/prerelease/`. When exiting pre-release mode or switching to a different label (for example, from `alpha` to `beta`), any preserved changesets are restored back to `.sampo/changesets/`, so the next release keeps the full history.

## Configuration

> [!NOTE]
> Since Sampo automatically detects packages in your workspace (based on ecosystem conventions) and infers sensible defaults for most settings, you can often skip this section and use Sampo out-of-the-box. 

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
  "cargo/package-a",
  "internal-*",
  "examples/*"
]
fixed = [["cargo/pkg-a", "cargo/pkg-b"], ["cargo/pkg-c", "cargo/pkg-d"]]
linked = [["cargo/pkg-e", "cargo/pkg-f"], ["cargo/pkg-g", "cargo/pkg-h"]]
```

### `[git]` section

`default_branch`: Name of the primary release branch (default: `"main"`).

`release_branches`: Additional branch names that should behave like long-lived release lines. The default branch is always included automatically, so this list only needs the extra branches (e.g. `"3.x"`, `"4.0"`).

> [!TIP]
> At runtime you can override the detected branch with the `SAMPO_RELEASE_BRANCH` environment variable, which is useful for local testing or custom CI setups.

### `[github]` section

`repository`: The GitHub repository slug in the format "owner/repo". If not set, Sampo uses the `GITHUB_REPOSITORY` environment variable or attempts to detect it from the `origin` git remote. This setting is used to enrich changelog messages with commit hash links and author acknowledgments, especially for first-time contributors.

### `[changelog]` section

`show_commit_hash`: Whether to include commit hash links in changelog entries (default: `true`). When enabled, changelog entries include clickable commit hash links that point to the commit on GitHub.

`show_acknowledgments`: Whether to include author acknowledgments in changelog entries (default: `true`). When enabled, changelog entries include author acknowledgments with special messages for first-time contributors.

`show_release_date`: Whether to append a release date to each changelog heading (default: `true`).

`release_date_format`: [`chrono` strftime](https://docs.rs/chrono/latest/chrono/format/strftime/index.html) pattern used for the heading date (default: `%Y-%m-%d`).

`release_date_timezone`: Optional timezone for the stamp. Accepts `local`, `UTC`, numeric offsets such as `+02:00`, or any IANA name (for example `Europe/Paris`).

### `[packages]` section

You can ignore certain packages, so they do not appear in the CLI commands, changesets, releases, or publishing steps. This is useful for packages that are not meant to be published or versioned, such as internal tools, examples, or documentation packages. Changesets targeting only ignored packages are left unconsumed.

`ignore_unpublished`: If `true` (default: `false`), ignore every package configured as not publishable. For example, `publish = false` in `Cargo.toml` for Rust crates or `"private": true` in a workspace `package.json` for npm packages.

`ignore`: A list of glob-like patterns to ignore packages by canonical identifier, plain name, or relative path. `*` matches any sequence. Examples:
- `cargo/internal-*`: ignores Cargo packages with names like `internal-tool`.
- `npm/web-*`: ignores npm packages whose names start with `web-`.
- `examples/*`: ignores any package whose relative path starts with `examples/`.
- `package-a`: ignores a package named exactly `package-a` (only if the name is unique across ecosystems).

> [!NOTE]
> Canonical identifiers follow the `<ecosystem>/<name>` format (e.g., `cargo/my-crate` for a Rust package, `npm/web-app` for a JavaScript package). Sampo continues to accept plain names, but you'll be prompted to disambiguate if a name appears in multiple ecosystems.

Sampo detects packages within the same repository that depend on each other and automatically manages their versions. By default, dependent packages are automatically patched when a workspace dependency is updated. For example: if `a@0.1.0` depends on `b@0.1.0` and `b` is updated to `0.2.0`, then `a` will be automatically bumped to `0.1.1` (patch). If `a` needs a major or minor change due to `b`'s update, it should be explicitly specified in a changeset. Some options allow customizing this behavior:

`fixed`: An array of dependency groups (default: `[]`) where packages in each group are bumped together with the same version level. Each group is an array of packages and can mix ecosystems. When any package in a group is updated, all other packages in the same group receive the same version bump, regardless of actual dependencies. For example: if `fixed = [["cargo/a", "cargo/b"], ["cargo/c", "cargo/d"]]` and `cargo/a` is updated to `2.0.0` (major), then `cargo/b` will also be bumped to `2.0.0`, but `cargo/c` and `cargo/d` remain unchanged.

`linked`: An array of dependency groups (default: `[]`) where affected packages and their dependents are bumped together using the highest bump level in the group. Each group is an array of packages and may include multiple ecosystems. When any package in a group is updated, all packages in the same group that are affected or have workspace dependencies within the group receive the highest version bump level from the group. For example: if `linked = [["cargo/a", "cargo/b"]]` where `cargo/a` depends on `cargo/b`, when `cargo/b` is updated to `2.0.0` (major), then `cargo/a` will also be bumped to `2.0.0`. If `cargo/a` is later updated to `2.1.0` (minor), `cargo/b` remains at `2.0.0` since it's not affected. Finally, if `cargo/b` has a patch update, both `cargo/a` and `cargo/b` will be bumped with patch level since it's the highest bump in the group.

> [!WARNING]
> Packages cannot appear in both `fixed` and `linked` configurations.

## Commands

All commands should be run from the root of the repository:

| Command         | Description                                                               |
| --------------- | ------------------------------------------------------------------------- |
| `sampo help`    | Show commands or the help of the given subcommand(s)                      |
| `sampo init`    | Initialize Sampo in the current repository                                |
| `sampo add`     | Create a new changeset                                                    |
| `sampo pre`     | Manage pre-release versions (enter or exit pre-release mode)              |
| `sampo release` | Consume changesets, and prepare release(s) (bump versions and changelogs) |
| `sampo publish` | Publish packages to registries and tag current versions                   |

For detailed command options, use `sampo help <command>` or `sampo <command> --help`.

## Alternatives

Sampo is deeply inspired by [Changesets](https://github.com/changesets/changesets) and [Lerna](https://github.com/lerna/lerna), from which we borrow the changeset format and monorepo release workflows. But our project goes beyond the JavaScript/TypeScript ecosystem, as it is made with Rust, and designed to support multiple mixed ecosystems. Other <abbr title="Node Package Manager">npm</abbr>-limited tools include [Rush](https://github.com/microsoft/rushstack), [Ship.js](https://github.com/algolia/shipjs), [Release It!](https://github.com/release-it/release-it), and [beachball](https://github.com/microsoft/beachball).

Google's [Release Please](https://github.com/googleapis/release-please) is ecosystem-agnostic, but lacks publishing capabilities, and is not monorepo-focused. Also, it uses [Conventional Commits](https://www.conventionalcommits.org/en/v1.0.0/) messages to infer changes instead of explicit changesets, which confuses the technical history (used and written by contributors) with the <abbr title="Application Programming Interface">API</abbr> changelog (used by users, can be writen/reviewed by product/docs owner). Other commit-based tools include [semantic-release](https://github.com/semantic-release/semantic-release) and [auto](https://github.com/intuit/auto).

[Knope](https://github.com/knope-dev/knope) is an ecosystem-agnostic tool inspired by Changesets, but lacks publishing capabilities, and is more config-heavy. But we are thankful for their open-source [changeset parser](https://github.com/knope-dev/changesets) that we reused in Sampo!

To our knowledge, no other tool automates versioning, changelogs, and publishing, with explicit changesets, and multi-ecosystem support. That's the gap Sampo aims to fill!

## Development

Refer to [CONTRIBUTING.md](../../CONTRIBUTING.md#sampo) for development setup and workflow details.
