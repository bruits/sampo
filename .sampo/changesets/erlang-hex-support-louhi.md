---
cargo/sampo-core: minor
cargo/sampo: minor
cargo/sampo-github-action: minor
---

**Erlang packages are now supported!** Sampo now automatically detects Erlang applications managed by rebar3 (via their `.app.src`, for publishing to Hex) and handles versioning, changelogs, and publishing—even in mixed BEAM workspaces. Applications whose version is derived dynamically (`{vsn, git}`, `{vsn, "%VSN%"}`, or a `.app.src.script`) are skipped with a warning.
