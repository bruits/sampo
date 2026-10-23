# Sampo Core

Provides core logic, common types, and internal utilities shared across all Sampo crates.

Not sure what Sampo is? Don't know where to start? Check out Sampo's [documentation](https://github.com/bruits/sampo/blob/main/crates/sampo/README.md).

## Development

Release planning, changelog generation, and configuration parsing live here. The `PackageAdapter` enum holds every ecosystem-specific operation (workspace discovery, manifest parsing, publishability checks, registry APIs, lockfile regeneration). A new ecosystem gets its own adapter in `src/adapters/<ecosystem>.rs` and a variant in the enum, and every `match` on the enum then delegates to that adapter. Most tests run on throwaway workspaces, with helpers in `src/release_tests.rs` and `src/workspace.rs`.

Refer to [CONTRIBUTING.md](https://github.com/bruits/sampo/blob/main/CONTRIBUTING.md#getting-started) for development setup and workflow details.
