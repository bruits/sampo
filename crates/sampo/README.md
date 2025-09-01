<picture>
  <source media="(prefers-color-scheme: dark)" srcset="./.github/assets/Sampo_logo_dark.svg" />
  <img alt="Sampo logo" src="./.github/assets/Sampo_logo_light.svg" />
</picture>

Automate changelogs, versioning, and publishing—even for monorepos across multiple registries.

## Subcommands (PoC)

- init: initialize Sampo in the repo
- add: create a new changeset (-p/--package multiple, -m/--message)
- status: show pending changesets and planned releases
- version: apply version bumps (--dry-run)
- publish: publish artifacts (--dry-run)

## Usage

From workspace root:
| Command                                                            | Description                                  |
| ------------------------------------------------------------------ | -------------------------------------------- |
| `cargo run -p sampo -- --help`                                     | Show help                                    |
| `cargo run -p sampo -- init`                                       | Initialize Sampo in the repo                 |
| `cargo run -p sampo -- add -p pkg-a -p pkg-b -m "feat: something"` | Create a new changeset                       |
| `cargo run -p sampo -- status`                                     | Show pending changesets and planned releases |
| `cargo run -p sampo -- version --dry-run`                          | Apply version bumps                          |
| `cargo run -p sampo -- publish --dry-run`                          | Publish artifacts                            |
