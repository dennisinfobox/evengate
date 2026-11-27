# Status

Last updated: 2026-11-27

**Phase:** 1 — First tasks (week 5–6). The knowledge loop is closed: the review agent's lessons become the next executor's context.

**Done since last handoff:**
- The reviewer verdict now carries `lessons` (3–5 bullets). The executor writes them to `lessons/<task-id>.md`, overwriting on a revision, under `EVENGATE_LESSONS_DIR` (default `lessons/`).
- Executor prompt assembly includes the five most recent lesson files, newest first by modification time with file-name ties, and states that the current spec and constraints win. The reviewer prompt is unchanged.
- No new event: the raw verdict stays in `AgentResponded`, so the file is reconstructible and `ev replay` remains a pure fold.
- Spec bumped to v0.7 (§7.5, §7.6, §7.8); ADR-0010 recorded; README documents the lessons directory.
- Checks pass: `cargo check --locked`, `cargo test --locked` (63 tests), `cargo clippy --locked -- -D warnings`.
- End-to-end against a local OpenAI-compatible stub: task A wrote `lessons/knowledge-smoke-a.md`; task B's recorded executor prompt contained the lessons section and both bullets, while its reviewer prompt did not.

**In flight:** nothing.

**Next:**
1. Run one task against a real OpenAI-compatible endpoint and land the first model-authored project.
2. Sandbox hardening; an execution replay path to rebuild a gitignored `project/` from the log.
3. Wire `rules/<project>.md` and `codebase-map.md` into prompt assembly, and implement the ≥3-occurrence promotion path from §7.8.

**Blockers:** none.

**Open questions:** `docs/spec.md` §13 items 1, 4, 5, 7 remain open; §13.1's provider/budget choice is configuration after ADR-0006. Item 2 is settled by ADR-0003, item 3 by ADR-0004, item 6 by ADR-0005.
