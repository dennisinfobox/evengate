# Status

Last updated: 2026-11-28

**Phase:** 1 — First tasks (week 5–6). The first model-authored project is merged.

**Done since last handoff:**
- Promotion path: after a review, the executor counts canonical lessons once per `lessons/*.md` file and records `LessonRepeated` for lessons seen at least three times that are not already in `rules/<project>.md`. Once every gate passes, `machine.rs` pauses before `OpenPullRequest` and asks for a human decision. `ev promote` appends the lesson to the project rules file and records `LessonPromoted`; `ev dismiss` records `LessonDismissed` only. Spec v0.12, ADR-0014.
- Verified end to end: both commands clear the pause and open the PR, and `promote` creates `rules/hello.md`.
- `tasks/sum-cli` and `tasks/word-count` replay byte-identically with the new state fields.
- Checks pass: `cargo check --locked`, `cargo test --locked` (81 tests per binary), `cargo clippy --locked -- -D warnings`.

**In flight:** nothing.

**Next:**
1. Run a second task to exercise the lessons/rules feedback loop end to end (week 5–6 exit criterion); the promotion threshold needs three tasks with the same lesson before it fires naturally.
2. Decide what to do with `tasks/hello`: its committed snapshot predates the `ReviewAgent` gate, so `ev replay` reports "merged without passed gates and human approval". Regenerate the snapshot or keep it as historical evidence.
3. `codebase-map.md` joins prompt assembly once a planner role has a consumer.

**Blockers:** none.

**Open questions:** `docs/spec.md` §13 items 1, 4, 5, 7 remain open; §13.1's provider/budget choice is configuration after ADR-0006. Item 2 is settled by ADR-0003, item 3 by ADR-0004, item 6 by ADR-0005.
