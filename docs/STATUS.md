# Status

Last updated: 2026-11-28

**Phase:** 1 — First tasks (week 5–6). The first model-authored project is merged.

**Done since last handoff:**
- Closed the `tasks/hello` question: it stays frozen as the seed-run record. Its snapshot predates the mandatory `ReviewAgent` gate, so `ev replay tasks/hello` is expected to exit with the `merged without passed gates and human approval` anomaly. No gate event was fabricated and the snapshot was not regenerated. Spec v0.13 scopes the replay guarantee to the reducer that wrote the snapshot; ADR-0015 records the decision, `tasks/hello/README.md` states the provenance.
- README quickstart now replays and materializes `tasks/sum-cli` (byte-identical) and uses `<task-dir>` placeholders for decisions; spec version reference corrected.
- Re-verified the mechanism on copies: `replay tasks/hello` → mismatch plus anomaly, `replay tasks/sum-cli` → 37 events, byte-identical.

**In flight:** nothing.

**Next:**
1. Run a second task to exercise the lessons/rules feedback loop end to end (week 5–6 exit criterion). The promotion threshold needs three lesson files sharing a canonical lesson before it fires naturally. Requires a live model endpoint in the shell (`EVENGATE_LLM_URL`, `EVENGATE_LLM_API_KEY`, `EVENGATE_MODEL` or per-role models) and an owner-chosen spec.
2. `codebase-map.md` joins prompt assembly once a planner role has a consumer.

**Blockers:** No live-model configuration (`EVENGATE_*`) is present in this environment, so the second-task run cannot start here.

**Open questions:** `docs/spec.md` §13 items 1, 4, 5, 7 remain open; §13.1's provider/budget choice is configuration after ADR-0006. Item 2 is settled by ADR-0003, item 3 by ADR-0004, item 6 by ADR-0005.
