# Status

Last updated: 2026-11-24

**Phase:** 0 — Seed kernel. Code complete; waiting for the first human merge.

**Done since last handoff:**
- Kernel: `event.rs`, `state.rs`, `machine.rs`, `executor.rs`, plus `spec.rs`, `gate.rs`, `sandbox.rs`, `log.rs`, and the `ev` / `evengate` CLI.
- `ev run tasks/hello/spec.md` runs the §14 scenario end to end: stub agent materializes `templates/bin/`, three deterministic gates pass in Docker, branch `ev/hello` is pushed to `tasks/hello/artifacts/remote.git`, `PrOpened` recorded.
- `ev replay tasks/hello` reproduces `state.json` byte-identically with no network.
- Human approval path verified on a scratch copy: `ev approve` finalizes `Merged` and replay stays byte-identical.
- `cargo check --locked`, `cargo test --locked` (25 tests), and `cargo clippy --locked -- -D warnings` pass.
- Sandbox image `evengate-sandbox:1.100` defined in `sandbox/Dockerfile` (adds clippy and rustfmt, runs with `--network=none`).

**In flight:** nothing.

**Next:**
1. Human reviews `git -C tasks/hello/project diff main..ev/hello`, merges, then records the decision: `ev approve tasks/hello --reviewer <name>`.
2. Week 3–4: `llm::LiveClient` / `RecordedClient` over `ureq`, the `ReviewAgent` gate, and sandbox hardening.

**Blockers:** none.

**Open questions:** `docs/spec.md` §13 items 1, 5, 6, 7 remain open; item 2 is settled by ADR-0003, item 3 for v0 by ADR-0004.
