---
cargo/sampo: patch
---

Running Sampo commands from a package subdirectory now correctly finds the workspace root by locating the `.sampo/` directory. If `.sampo/` doesn't exist, Sampo displays a clear error message: "Sampo not initialized. Run sampo init first."
