---
packages:
  - sampo
release: minor
---

Sampo can now detect packages within the same repository that depend on each other, and automatically manages their versions.

- By default, dependent packages are automatically patched when an internal dependency is updated. For example: if `a@0.1.0` depends on `b@0.1.0` and `b` is updated to `0.2.0`, then `a` will be automatically bumped to `0.1.1` (patch). If `a` needs a major or minor change due to `b`'s update, it should be explicitly specified in a changeset.
- **Fixed dependencies** always bump together with the same version, even if not directly affected. For example: if `a@1.0.0` and `b@1.0.0` are in a fixed group and `b` is updated to `2.0.0`, then `a` will also be bumped to `2.0.0`.
- **Linked dependencies** apply the highest bump level to affected packages and their dependents. For example: if `a@1.0.0` depends on `b@1.0.0` in a linked group and `b` is updated to `2.0.0` (major), then `a` will also be bumped to `2.0.0`. If `a` is later updated to `2.1.0` (minor), `b` remains at `2.0.0` since it's not affected. Finally, if `b` has a patch update, both `a` and `b` will be bumped with patch level (the highest in the group).

Changelogs now include clear explanations for automatic version bumps, ensuring transparency in automated dependency management. For example: "Updated dependencies: name@x.x.x" for cascade bumps, and "Bumped due to fixed dependency group policy" for fixed group updates.
