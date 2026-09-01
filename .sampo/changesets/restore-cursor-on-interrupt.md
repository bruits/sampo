---
cargo/sampo: patch
---

Fixed interactive prompts (`sampo add`, `sampo pre`, `sampo update`) leaving the terminal cursor hidden after Ctrl-C. Previously the cursor stayed hidden until running `reset`.
