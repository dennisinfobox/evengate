# Status

Last updated: 2026-11-28

**Phase:** Week 5–6 complete: the lessons → review → promotion loop ran end to end, including one promotion and three dismissals, and both new tasks are merged. Month 2 (self-edits) is next.

**Done since last handoff:**
- Ran `tasks/sum-cli-nary` and `tasks/sum-cli-help` against local Ollama `qwen2.5-coder:14b`. Both reached all four gates and opened their PRs; the third did so after its promotion decisions.
- `LiveClient` now sends `response_format: {"type":"json_object"}`. Without it the local model emitted unescaped quotes inside Rust source, so no manifest event was recorded (ADR-0018).
- The review prompt now carries the five most recent lesson files and asks for verbatim reuse of a still-applicable lesson. The reviewer copied all five task-1 lessons into task 2 and four of them into task 3 (ADR-0017).
- The third task paused with four lessons at count 3. All four decisions are recorded: edge cases was promoted into `rules/sum-cli.md`; dependencies, clippy, and unit tests were dismissed. The task then opened its PR. The usage lesson was reworded and did not group. The misleading "revision limit reached" line is suppressed when every gate passes.
- Approved both tasks with `ev approve`. Each bare remote's `main` was fast-forwarded to the feature tip, matching the recorded `merged_commit` for `sum-cli-nary` (`0687067`) and `sum-cli-help` (`0a25c9a`).
- Verified `ev replay` byte-identical for the new logs (39 and 47 events after the merges) and `ev materialize` rebuilds the lesson file. Checks pass: 84 tests per binary plus clippy.

**In flight:** nothing.

**Next:**
1. Start Month 2 with the first self-edit task: add a gate that checks `cargo fmt`, plus the loop-protection and gate-immutability rules in `machine.rs`.

**Blockers:** none.

**Open questions:** `docs/spec.md` §13 items 1, 4, 5, 7 remain open; item 1's provider choice is configuration and this run used local Ollama. Items 2, 3, and 6 are settled by ADR-0003, ADR-0004, and ADR-0005.
