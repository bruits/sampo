---
cargo/sampo-core: minor
cargo/sampo-github-action: minor
cargo/sampo-github-bot: minor
---

Added the `pull_request_changes` option to the `[github]` section. Set it to `"changesets"` to list each changeset once in the release pull request and the bot's comment, followed by the packages it names and after a table of packages and their bumps, instead of repeating it under every package.

```toml
[github]
pull_request_changes = "changesets"
```
