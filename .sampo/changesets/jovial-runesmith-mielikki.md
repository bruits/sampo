---
packages:
  - sampo
  - sampo-core
  - sampo-github-action
release: minor
---

⚠️ **breaking change:** Reorganize configuration by moving `fixed_dependencies` and `linked_dependencies` from `[packages]` to a new `[internal_dependencies]` section. 

```diff
// .sampo/config.toml
-[packages]
-  fixed_dependencies = [["pkg-a", "pkg-b"], ["pkg-c", "pkg-d", "pkg-e"]]
-  linked_dependencies = [["pkg-f", "pkg-g"]]
+[internal_dependencies]
+  fixed = [["pkg-a", "pkg-b"], ["pkg-c", "pkg-d", "pkg-e"]]
+  linked = [["pkg-f", "pkg-g"]]
```
