---
cargo/sampo: patch
cargo/sampo-core: patch
cargo/sampo-github-action: patch
---

In JavaScript/TypeScript (npm) projects, fixed `sampo publish` failing on pre-release versions with npm 11, which refuses to publish one without a dist-tag. A pre-release is now published under the first identifier of its pre-release version instead of `latest` (`1.2.0-alpha.1` under `alpha`), unless `publishConfig.tag` or a `--tag` passed through `--npm-args` names another; a `tag` set in `.npmrc` or `NPM_CONFIG_TAG` no longer applies to it.
