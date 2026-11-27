# Status

Last updated: 2026-11-27

**Phase:** 1 — First tasks (week 5–6). The deterministic gates and the LLM-backed review gate now form one pipeline; human approval remains the final gate.

**Done since last handoff:**
- `ReviewAgent` is the fourth mandatory gate (`gate::MANDATORY_V0`), after `Compiles`, `TestsPass`, and `LintsClean`.
- It reviews the real project tree (full spec + all text files, `.git`/`target` excluded, sorted, 256 KiB per-file cap) using the `Reviewer` role, records `AgentInvoked`/`AgentResponded` with token usage, and maps `{approve, issues[]}` to `Pass`/`Fail`. A malformed verdict becomes `NeedsHuman`, never a pass.
- A withheld approval stops the run immediately with the issues as the fail reason; the same artifact is never re-sent to the reviewer. The reducer accepts reviewer turns during `Verifying` without anomalies while executor turns stay phase-checked.
- Offline template mode cannot pass the review gate: it stops there with an actionable error, and the task resumes at that gate once a model is configured.
- Spec bumped to v0.5 (§7.6); ADR-0007 recorded; README explains the mandatory review.
- Checks pass: `cargo check --locked`, `cargo test --locked` (47 tests), `cargo clippy --locked -- -D warnings`.
- End-to-end against a local OpenAI-compatible stub: approve → 4 gates pass → PR opened; reject → review `Fail` with reason, no PR; offline run → review error → resume with the model → PR. Both logs replay byte-identically (22 and 12 events). Token usage recorded for both roles.

**In flight:** nothing.

**Next:**
1. Gate-failure recovery: a rejected review (or any gate failing after retries) stops at `RequestHumanReview` with no command path to revise or abandon. Needs a revision phase that re-invokes the executor with the failure reasons.
2. `knowledge.rs`: write `lessons/<task>.md` after a task and feed the most recent lessons into the next prompt (the rest of week 5–6).
3. Run one task against a real OpenAI-compatible endpoint and land the first model-authored project.
4. Sandbox hardening; an execution replay path to rebuild a gitignored `project/` from the log.

**Blockers:** none.

**Open questions:** `docs/spec.md` §13 items 1, 4, 5, 7 remain open; §13.1's provider/budget choice is configuration after ADR-0006. Item 2 is settled by ADR-0003, item 3 by ADR-0004, item 6 by ADR-0005.
