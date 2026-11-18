---
name: sampo-changeset
description: Create or update changesets to describe public API changes, and trigger changelog generation and release planning.
---

Sampo uses changesets to version packages, write changelogs, and publish releases. It uses SemVer by default and also supports CalVer.

See [CONTRIBUTING.md](/CONTRIBUTING.md#writing-changesets) for changeset redaction guidelines.

## Creating New Changesets

To create a changeset non-interactively:

```sh
sampo add -p <package> -b <bump> -m "<description>"
```

Where `<bump>` is `major`, `minor`, or `patch`. Use `-p` multiple times to target several packages. Prefix with the ecosystem to disambiguate: `-p cargo/my-crate`. When `changesets.tags` is configured, use `-t <tag>` to categorize the changeset.

## Updating Existing Changesets

Pending changesets are stored in the `.sampo/changesets` directory. You can edit these markdown files directly, as long as you follow the guidelines above and Sampo format (read `.sampo/changeset.md.example` for reference).
