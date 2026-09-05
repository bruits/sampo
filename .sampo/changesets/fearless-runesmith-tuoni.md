---
npm/sampo: patch
---

Distinguish between wrapper and binary getting a signal.

Make the wrapper print a log message when the sampo binary
gets killed by a signal. This makes it clear whether it
was the wrapper or the binary that received the signal.
