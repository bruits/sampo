# sampo

## 0.22.0 — 2026-10-09

### Patch changes

- [eda3c06](https://github.com/bruits/sampo/commit/eda3c06a246430106cf0c05eb3bca2856d7e71dd) Changed `sampo` to print `sampo: binary received signal <SIGNAL>` and exit with status 128 plus the signal number when its native binary is killed by a signal. — Thanks @pjonsson!
- Updated dependencies: @bruits/sampo-darwin-arm64 (npm)@0.22.0, @bruits/sampo-darwin-x64 (npm)@0.22.0, @bruits/sampo-linux-arm64 (npm)@0.22.0, @bruits/sampo-linux-x64 (npm)@0.22.0, @bruits/sampo-win32-x64 (npm)@0.22.0

## 0.21.0 — 2026-08-20

### Minor changes

- Bumped due to fixed dependency group policy

## 0.20.0 — 2026-08-13

### Minor changes

- Bumped due to fixed dependency group policy

## 0.19.0 — 2026-06-28

### Minor changes

- Bumped due to fixed dependency group policy

## 0.18.1 — 2026-06-19

### Patch changes

- Bumped due to fixed dependency group policy

## 0.18.0 — 2026-06-07

### Minor changes

- [c71edc3](https://github.com/bruits/sampo/commit/c71edc30a9f7cee0ed67b2e2a2125bae76190086) Sampo is now published to npm 🎉 For JavaScript/TypeScript projects, you can install it via `pnpm`/`npm`/`yarn`/`bun` without a Cargo toolchain, on Linux x64/arm64, macOS x64/arm64, or Windows x64.
  
  ```bash
  pnpm add -D sampo   && pnpm sampo --help    # or:
  npm  i   -D sampo   && npx  sampo --help    # or:
  yarn add -D sampo   && yarn sampo --help    # or:
  bun  add -D sampo   && bunx sampo --help
  ```
   — Thanks @goulvenclech!

