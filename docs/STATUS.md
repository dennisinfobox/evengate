# Status

Last updated: 2026-11-28

**Phase:** 1 — First tasks (week 5–6). The first model-authored project is merged.

**Done since last handoff:**
- Abandoned `tasks/word-count` at the two-revision cap with a `HumanAbandoned` event; its 28 events replay byte-identically and the phase is `abandoned`.
- Hardened gate containers: read-only root and read-only `project/` mount, 1 GiB executable `/tmp` tmpfs, memory and swap capped at 2 GiB, PID cap 256, CPU cap 2, all capabilities dropped, `no-new-privileges`, `--init`. Caps are overridable with `EVENGATE_SANDBOX_MEMORY`, `EVENGATE_SANDBOX_CPUS`, and `EVENGATE_SANDBOX_PIDS`. Spec v0.10, ADR-0012.
- Verified end to end: a template task passed `Compiles`, `TestsPass`, and `LintsClean` inside the hardened sandbox; the run then stopped at the review gate for lack of a model, as designed.
- Checks pass: `cargo check --locked`, `cargo test --locked` (70 tests), `cargo clippy --locked -- -D warnings`.

**In flight:** nothing.

**Next:**
1. Wire `rules/<project>.md` into executor prompt assembly; `codebase-map.md` waits for a planner consumer.
2. Implement the ≥3-occurrence promotion path from §7.8; it needs a reducer-visible source for lesson repetition, so record the design in an ADR first.
3. Optional: run a fresh word-count task from a sharper spec; the abandoned run is closed.

**Blockers:** none.

**Open questions:** `docs/spec.md` §13 items 1, 4, 5, 7 remain open; §13.1's provider/budget choice is configuration after ADR-0006. Item 2 is settled by ADR-0003, item 3 by ADR-0004, item 6 by ADR-0005.
