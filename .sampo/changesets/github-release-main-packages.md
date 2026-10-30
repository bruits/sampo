---
cargo/sampo-core: minor
cargo/sampo-github-action: minor
---

Added a package list to the GitHub Action's `create-github-release` input, besides `true` and `false`. Only the listed packages get a GitHub Release, and the release of the first one also carries the notes of every other package published with it.

```yaml
create-github-release: my-app
```
