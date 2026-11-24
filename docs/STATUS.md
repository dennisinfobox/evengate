# Status

Last updated: 2026-11-24

**Phase:** 1 — First tasks (week 3–4). Phase 0 exit criterion met: one task ran end to end, the PR was opened by the git CLI, and a human merged it.

**Done since last handoff:**
- Seed kernel complete: `event.rs`, `state.rs`, `machine.rs`, `executor.rs`, plus `spec.rs`, `gate.rs`, `sandbox.rs`, `log.rs`, and the `ev` / `evengate` CLI.
- `tasks/hello` finished: stub agent materialized `templates/bin/`, `compiles` / `tests_pass` / `lints_clean` passed in the sandbox, branch `ev/hello` was pushed, `PrOpened` recorded.
- Human merge: `main` fast-forwarded through merge commit `cec5904`; approval recorded by `dennis`; the `Merged` event carries the approved PR head `506bd83`.
- `ev replay tasks/hello` reproduces `state.json` byte-identically (22 events, no anomalies).
- `cargo check --locked`, `cargo test --locked` (25 tests), `cargo clippy --locked -- -D warnings` pass on the pinned 1.100.0 toolchain.
- Sandbox image `evengate-sandbox:1.100` defined in `sandbox/Dockerfile` (adds clippy and rustfmt, runs with `--network=none`).

**In flight:** nothing.

**Next:**
1. `llm::LiveClient` over `ureq` and `llm::RecordedClient` for replay; record responses as events.
2. `ReviewAgent` gate (LLM-backed, recorded) after the deterministic gates.
3. Sandbox hardening: confirm the copy-in/copy-out contract and timeout behaviour under failure.

**Blockers:** none.

**Open questions:** `docs/spec.md` §13 items 1, 5, 6, 7 remain open; item 2 is settled by ADR-0003, item 3 for v0 by ADR-0004.
