# hello

## Goal
Create a Rust binary crate `hello` from the template at `templates/bin/`, verify that it compiles, passes its tests, and is clean under clippy, then open a pull request for the result.

## Acceptance
- [ ] `cargo check --locked` exits 0 in the sandbox.
- [ ] `cargo test --locked` exits 0 in the sandbox.
- [ ] `cargo clippy --locked -- -D warnings` exits 0 in the sandbox.
- [ ] A branch containing the crate is pushed and a pull request is recorded.

## Constraints
- Rust only, no external crates.
- No network access inside the sandbox.
- The crate is generated from `templates/bin/`; the template is the only source of code.

## Non-goals
- No LLM involvement.
- No review agent.
- No knowledge artifacts.

## Context
- `docs/spec.md` §11 and §14.

## Open questions
None.
