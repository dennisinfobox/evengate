# Sum CLI

## Goal
Build a Rust command-line tool that sums one or more decimal integers given as arguments, prints the total, and prints a short usage message with a non-zero exit when there are no arguments, an argument is not an integer, or the sum would overflow a 64-bit signed integer.

## Acceptance
- [ ] One or more integer arguments print their sum.
- [ ] Negative integers are accepted and summed correctly.
- [ ] No arguments, or an argument that is not an integer, prints usage and exits non-zero.
- [ ] A sum that would overflow `i64` prints a short error and exits non-zero instead of panicking.
- [ ] Unit tests cover a valid sum, negative values, missing arguments, invalid input, and overflow.

## Constraints
- Rust only, no external crates.
- Start from the reference template; keep its package metadata and lock file valid.
- Tests must not touch the filesystem.
- `cargo clippy --locked -- -D warnings` must pass.

## Non-goals
- No floats, flags, stdin input, or multiple operations.
- No arbitrary-precision integers.

## Context
- The reference template at `templates/bin/` is the starting point.
- This is the second task in the `sum-cli` project. `tasks/sum-cli` is the prior run and `lessons/sum-cli.md` holds its review lessons.
- The prior task deliberately excluded overflow handling; this task adds it.

## Open questions
None.
