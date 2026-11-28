# Status

Last updated: 2026-11-28

**Phase:** Month 2 started. The first self-edit landed directly in Evengate's source: `FormatClean` is now a mandatory gate, gate sets are recorded per task, and loop protection plus the reasoned gate-set override are enforced in `machine.rs`.

**Done since last handoff:**
- Added `GateId::FormatClean` (`cargo fmt --check`) between `LintsClean` and `ReviewAgent`; the sandbox image already ships rustfmt. Verified the failing and passing paths inside `evengate-sandbox:1.100`, and formatted the tree once (it was not rustfmt-clean).
- `TaskCreated` records the mandatory gate set. `State.gates` is the task's fixed set; logs without the field fold to `LEGACY_V0` (the old four) and serialize without it. `ev replay` remains byte-identical for `sum-cli` (37 events), `sum-cli-nary` (39), and `sum-cli-help` (47). `tasks/hello` is unchanged and still replays with its documented pre-gate anomaly.
- `machine.rs` iterates the recorded set and now enforces loop protection: `MAX_AGENT_INVOCATIONS_PER_PHASE = 5` via a reducer-reset `phase_invocations` counter, alongside the existing gate-retry and revision caps. The machine never reissues a command without an intervening state change.
- Gate immutability: a task whose recorded set differs from the running binary's `MANDATORY_V0` cannot merge on a bare approval. `ev approve --reason <text>` records `HumanApproved { reviewer, reason }`; the CLI refuses a bare approval when the set differs. A test asserts `LEGACY_V0` stays a subset of `MANDATORY_V0`, so a self-edit that drops a historical gate fails the artifact's own `TestsPass` gate.
- Checks: 96 tests per binary, `cargo check --locked`, `cargo clippy --locked -- -D warnings`, and `cargo fmt --check` pass.

**In flight:** nothing.

**Next:**
1. Give the executor a self-edit path so the next Month 2 change runs through the factory: base `project/` on a copy of this repository and run Evengate's own gates on a separate checkout before the PR.

**Blockers:** none.

**Open questions:** `docs/spec.md` §13 items 1, 4, 5, 7 remain open; item 1's provider choice is configuration and the last factory runs used local Ollama. Items 2, 3, and 6 are settled by ADR-0003, ADR-0004, and ADR-0005.
