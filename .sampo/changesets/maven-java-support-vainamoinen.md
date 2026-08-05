---
cargo/sampo-core: minor
cargo/sampo: minor
cargo/sampo-github-action: minor
---

**Java packages are now supported!** Sampo now automatically detects Maven projects (via their `pom.xml`, including multi-module reactors with parent-inherited versions) and handles versioning, changelogs, and publishing to Maven Central through `mvn deploy`. Packages whose version is a snapshot or a build-time property (`${revision}`) are skipped with a warning, and Sampo refuses to write a snapshot version rather than lose track of the package.
