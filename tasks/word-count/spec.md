# Word Count CLI

## Goal
Build a Rust command-line tool that counts the lines, words, and bytes of a text file given as its first argument, prints the three counts, and exits non-zero with a short usage message when the argument is missing.

## Acceptance
- [ ] Running the binary with a file path prints line, word, and byte counts.
- [ ] Running it without an argument prints a usage message and exits non-zero.
- [ ] Unit tests cover the counting function.

## Constraints
- Rust only, no external crates.
- Start from the reference template; keep its package metadata and lock file valid.
- `cargo clippy --locked -- -D warnings` must pass.

## Non-goals
- No flags, recursion, or multiple files.

## Context
- The reference template at `templates/bin/` is the starting point.

## Open questions
None.
