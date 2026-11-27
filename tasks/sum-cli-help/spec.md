# Sum CLI

## Goal
Build a Rust command-line tool that sums one or more decimal integers given as arguments, prints the total, and supports `--help` and `--version`; it prints a short usage message with a non-zero exit when an argument is an unknown flag or not an integer.

## Acceptance
- [ ] One or more integer arguments print their sum.
- [ ] Negative integers are accepted as ordinary addends.
- [ ] `--help` prints the usage message and exits 0.
- [ ] `--version` prints the package name and version from `CARGO_PKG_VERSION` and exits 0.
- [ ] An unknown flag or an argument that is not an integer prints usage and exits non-zero.
- [ ] A sum that would overflow `i64` prints a short error and exits non-zero instead of panicking.
- [ ] Unit tests cover sums, negative values, help, version, unknown flags, invalid input, and overflow.

## Constraints
- Rust only, no external crates.
- Start from the reference template; keep its package metadata and lock file valid.
- Tests must not touch the filesystem.
- `cargo clippy --locked -- -D warnings` must pass.

## Non-goals
- No floats, flags other than `--help` and `--version`, stdin input, or multiple operations.
- No arbitrary-precision integers.

## Context
- The reference template at `templates/bin/` is the starting point.
- This is the third task in the `sum-cli` project. `tasks/sum-cli` and `tasks/sum-cli-nary` are the prior runs; their review lessons are in `lessons/sum-cli.md` and `lessons/sum-cli-nary.md`.
- The prior tasks excluded overflow handling and flags; this task keeps the overflow error and adds the two flags.

## Open questions
None.
