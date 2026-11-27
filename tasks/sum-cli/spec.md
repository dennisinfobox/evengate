# Sum CLI

## Goal
Build a Rust command-line tool that reads two decimal integers from its first two arguments, prints their sum, and prints a short usage message with a non-zero exit when the arguments are missing or not integers.

## Acceptance
- [ ] Two integer arguments print their sum.
- [ ] Missing or invalid arguments print usage and exit non-zero.
- [ ] Unit tests cover parsing valid and invalid integers.

## Constraints
- Rust only, no external crates.
- Start from the reference template; keep its package metadata and lock file valid.
- Tests must not touch the filesystem.
- `cargo clippy --locked -- -D warnings` must pass.

## Non-goals
- No floats, overflow handling, or flags.

## Context
- The reference template at `templates/bin/` is the starting point.

## Open questions
None.
