# Status

Last updated: 2026-11-27

**Phase:** 1 — First tasks (week 5–6). The factory can now recover from a blocked gate inside the event log: a human asks for a bounded revision, the executor re-runs with the verdicts, and every gate re-judges the new tree.

**Done since last handoff:**
- Gate-failure recovery: `ev revise <task> --reviewer <name> [--reason <text>]` records `RevisionRequested`, snapshots the non-passing verdicts plus guidance into `State`, clears all gate results and attempt counters, and enters `Revising`.
- `next_command` re-invokes the executor once with a `Context` whose new `feedback` field lists the failing gates and human guidance; the prompt renders it under `# Revision feedback`.
- Revisions are capped at `MAX_REVISIONS = 2`; the machine also refuses to invoke beyond the cap, so a hand-edited log cannot loop. Clearing attempts is safe because the revision count bounds total retries.
- `ev abandon` records `HumanAbandoned` and ends any non-terminal task without a PR; `TaskAbandoned` still follows a PR rejection. `ev run` now prints the blocking gate verdicts.
- Spec bumped to v0.6 (§7.2, §7.3, §7.6, §10.3); ADR-0008 recorded; README documents both recovery commands.
- Checks pass: `cargo check --locked`, `cargo test --locked` (56 tests), `cargo clippy --locked -- -D warnings`.
- End-to-end against a local OpenAI-compatible stub: first manifest fails `cargo check` twice → blocked with the reason; `ev revise` with guidance → all four gates pass → PR; `ev approve` → merged; the 32-event log replays byte-identically. A synthetic blocked log replays identically after `ev abandon`; `revise`/`abandon` on a merged task are rejected.

**In flight:** nothing.

**Next:**
1. `knowledge.rs`: write `lessons/<task>.md` after a task and feed the most recent lessons into the next prompt (the rest of week 5–6).
2. Run one task against a real OpenAI-compatible endpoint and land the first model-authored project.
3. Sandbox hardening; an execution replay path to rebuild a gitignored `project/` from the log.

**Blockers:** none.

**Open questions:** `docs/spec.md` §13 items 1, 4, 5, 7 remain open; §13.1's provider/budget choice is configuration after ADR-0006. Item 2 is settled by ADR-0003, item 3 by ADR-0004, item 6 by ADR-0005.
