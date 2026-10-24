---
cargo/sampo: patch
cargo/sampo-core: patch
cargo/sampo-github-action: patch
---

In JavaScript/TypeScript (npm) projects, fixed `sampo publish` failing on pre-release versions with npm 11: a pre-release now goes under a dist-tag named after its first pre-release identifier (`alpha` for `1.2.0-alpha.1`), or `next` when npm cannot take that identifier as a tag. `publishConfig.tag` and a `--tag` passed through `--npm-args` still take precedence, while a `tag` set in `.npmrc` or `NPM_CONFIG_TAG` no longer applies to pre-releases.
