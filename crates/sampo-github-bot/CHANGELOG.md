# sampo-github-bot

## 0.3.2 — 2025-11-18

### Patch changes

- Updated dependencies: sampo-core@0.9.1

## 0.3.1 — 2025-10-31

### Patch changes

- [673b44d](https://github.com/bruits/sampo/commit/673b44d6966ebf3cf4ce904b67a11fbbc3dc0bb0) Sampo GitHub Bot no longer spams change-request reviews when no changesets are found, and keeps one silent approval review in sync with the presence of changesets in the PR. — Thanks @goulvenclech!

## 0.3.0 — 2025-10-27

### Minor changes

- [d1fb836](https://github.com/bruits/sampo/commit/d1fb836d02b0368f90531ab976b9fb16be6e5553) Github bot's feedback messages have been improved: when no changeset is found, explicit instructions are provided on how to create one; when changesets are present, a preview of their content and bump level is included. — Thanks @goulvenclech!

## 0.2.2

### Patch changes

- [0936318](https://github.com/bruits/sampo/commit/0936318b145d1265bf4a2e9128ce333336a0f7ff) Refactor error handling to improve context and consistency. — Thanks @goulvenclech!


## 0.2.1

### Patch changes

- [6062083](https://github.com/bruits/sampo/commit/6062083ae20e3bcea6c1f4f00d6b58cf790cd9f1) Fix deploys and publishing. — Thanks @goulvenclech!


## 0.2.0

### Minor changes

- [`a3015e7`](https://github.com/bruits/sampo/commit/a3015e7c06ac24394f018b8ec2aed4e971ae7f4b) Initial release of sampo-github-bot, a GitHub App server to inspect pull requests and automatically request Sampo changesets when needed. Includes GitHub App auth, sticky automatic comments, and deployment on Fly.io. — Thanks @goulvenclech!

### Patch changes

- [`c91b1e9`](https://github.com/bruits/sampo/commit/c91b1e9b494476a806108a3f7878b511ace9995c) Sampo's bot now ignores Sampo Action's release pull requests. — Thanks @goulvenclech!
- [`1c9cd1b`](https://github.com/bruits/sampo/commit/1c9cd1bccd779b377517012f25479d0b0885a66b) Fix deploys by adding openssl-libs-static package. Resolves linker errors when building statically linked binaries with SSL dependencies on Alpine Linux. — Thanks @goulvenclech!


## 0.1.0

### Minor changes

- Initial release of sampo-github-bot, a GitHub App server to inspect pull requests and automatically request Sampo changesets when needed. Includes GitHub App auth, sticky automatic comments, and deployment on Fly.io.

