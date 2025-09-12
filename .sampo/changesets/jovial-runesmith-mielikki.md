---
packages:
  - sampo
  - sampo-core
  - sampo-github-action
release: minor
---

⚠️ **breaking change:** Rename dependent package options from `fixed_dependencies` and `linked_dependencies` to `fixed` and `linked`.

```diff
// .sampo/config.toml
[packages]
-  fixed_dependencies = [["pkg-a", "pkg-b"], ["pkg-c", "pkg-d", "pkg-e"]]
-  linked_dependencies = [["pkg-f", "pkg-g"]]
+  fixed = [["pkg-a", "pkg-b"], ["pkg-c", "pkg-d", "pkg-e"]]
+  linked = [["pkg-f", "pkg-g"]]
```
