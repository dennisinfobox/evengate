# Status

Last updated: 2026-11-27

**Phase:** 1 — First tasks (week 5–6). The first model-authored project is merged.

**Done since last handoff:**
- `tasks/sum-cli` merged at `c8ab29c` after one revision; 37 events replay byte-identically; lessons in `lessons/sum-cli.md`.
- `tasks/word-count` stopped at the two-revision cap (the test used `io::ErrorKind` without `use std::io`); its log and state are preserved for a decision.
- Added `ev materialize <task-dir>`: execution replay rebuilds gitignored `project/` and `lessons/<task-id>.md` from `AgentResponded` events in log order, with no model, network, or state snapshot change. Spec v0.9, ADR-0011.
- Checks pass: `cargo check --locked`, `cargo test --locked` (70 tests), `cargo clippy --locked -- -D warnings`.

**In flight:** nothing.

**Next:**
1. Owner decision on `word-count`: abandon it, or rerun from a sharper spec (the failure was one missing `std::io` import).
2. Sandbox hardening: resource limits beyond the current no-network, copy, timeout, and output caps.
3. Wire `rules/<project>.md` (and `codebase-map.md` once a planner exists) into prompt assembly, and implement the ≥3-occurrence promotion path from §7.8; promotion needs a reducer-visible source for lesson repetition.

**Blockers:** none.

**Open questions:** `docs/spec.md` §13 items 1, 4, 5, 7 remain open; §13.1's provider/budget choice is configuration after ADR-0006. Item 2 is settled by ADR-0003, item 3 by ADR-0004, item 6 by ADR-0005.
