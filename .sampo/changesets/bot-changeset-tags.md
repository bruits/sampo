---
cargo/sampo-github-bot: patch
---

Fixed the bot reporting changesets with a tag, such as `minor (Added)`, as invalid when `[changesets]` `tags` allows it.
