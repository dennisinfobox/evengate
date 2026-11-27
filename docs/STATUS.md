# Status

Last updated: 2026-11-28

**Phase:** 1 — First tasks (week 5–6). The first model-authored project is merged.

**Done since last handoff:**
- Abandoned `tasks/word-count` at the two-revision cap with a `HumanAbandoned` event; its 28 events replay byte-identically.
- Hardened gate containers: read-only root and `project/` mount, 1 GiB executable `/tmp` tmpfs, 2 GiB memory/swap cap, PID cap 256, CPU cap 2, capabilities dropped, `no-new-privileges`, `--init`. Overrides: `EVENGATE_SANDBOX_MEMORY`, `EVENGATE_SANDBOX_CPUS`, `EVENGATE_SANDBOX_PIDS`. Spec v0.10, ADR-0012.
- Wired project rules: both executor and reviewer prompts read `rules/<project>.md` (root `rules/`, override `EVENGATE_RULES_DIR`) at assembly time. Missing or blank means no section; unreadable fails the run. Spec v0.11, ADR-0013.
- Checks pass: `cargo check --locked`, `cargo test --locked` (72 tests), `cargo clippy --locked -- -D warnings`.

**In flight:** nothing.

**Next:**
1. Implement the ≥3-occurrence promotion path from §7.8; it needs a reducer-visible source for lesson repetition, so record the design in an ADR first.
2. `codebase-map.md` joins prompt assembly once a planner role has a consumer.
3. Optional: run a fresh word-count task from a sharper spec.

**Blockers:** none.

**Open questions:** `docs/spec.md` §13 items 1, 4, 5, 7 remain open; §13.1's provider/budget choice is configuration after ADR-0006. Item 2 is settled by ADR-0003, item 3 by ADR-0004, item 6 by ADR-0005.
