# Agents Guide

Principles for how automated agents and contributors generate code and docs here. Favor clarity, small testable units, and the project’s existing conventions.

## Core Engineering Values

- Clarity over cleverness: write idiomatic, expressive, statically typed code.
- Leverage immutability and pattern matching where available; avoid hidden state.
- Prefer small, focused modules and explicit types over “magic”.

## Testing

- Unit-test pure functions and isolated modules.
- Assert observable behavior (inputs/outputs, effects), not internal details.
- Keep tests deterministic and independent of global state.

## Errors
- Common errors live in `sampo-core`'s `errors.rs`, while crate-specific errors live in their respective crates' `error.rs`.
- Use typed error enums with `thiserror` for a stable, explicit API.
- Keep error messages concise and in English; add context at the boundary (CLI/action) rather than deep in core.
- Avoid `unwrap()` when possible in production code, prefer proper error propagation with `?` operator.
- Use `Result<T>` type aliases consistently across crates for ergonomic error handling.

## Documentation

- Self-documenting code first: expressive names and straightforward logic.
- Comments explain why (intent, invariants, trade‑offs), not how.
- All code, comments, documentation, commit messages, and user-facing output (CLI prompts, logs, errors) must be in English.
- Do NOT create a documentation file to explain the implementation.

## Repository Conventions

- Before generating new code or docs, parse repository to inherit existing conventions and avoid duplication.
- Match the current project structure, naming, and style; do not create parallel patterns.
- Explicit `use` imports for standard library types.

## Changes & Dependencies

- Do not alter CI/CD configuration unless explicitly instructed.
- Avoid introducing external dependencies; add only with strong justification and prior discussion. Prefer the standard library and existing utilities.
