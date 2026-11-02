---
cargo/sampo: patch
cargo/sampo-core: patch
cargo/sampo-github-action: patch
---

Fixed a rerun of a failed GitHub Action release never repairing what that run left out: the action now pushes every tag and creates every GitHub Release it can, fails on a failed release or asset upload, and sets `published` once anything landed. A rerun completes the missing tags, releases, and assets, as `sampo publish` now tags a published but untagged version when the current commit released it.
