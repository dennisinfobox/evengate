# Cover Slug Separator Runs

## Goal
Add a unit test proving that `slug` collapses a run of non-alphanumeric characters into a single dash.

## Acceptance
- [ ] A new test in `src/spec.rs` asserts `slug("a---b") == "a-b"`.
- [ ] The same test asserts `slug("a   b") == "a-b"`.
- [ ] `cargo test --locked` passes.

## Constraints
- Change only `src/spec.rs`.
- No new crates.
- Do not change the behavior of `slug`; only add the test.

## Non-goals
- No parser or target validation changes.

## Target
- src/spec.rs

## Context
- `src/spec.rs` already has a `slugs_are_filesystem_safe` test; add the new case next to it.

## Open questions
None.
