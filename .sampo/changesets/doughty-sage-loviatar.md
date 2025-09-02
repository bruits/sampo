---
packages:
  - sampo-github-bot
release: patch
---

Fix deploys by adding openssl-libs-static package. Resolves linker errors when building statically linked binaries with SSL dependencies on Alpine Linux.
