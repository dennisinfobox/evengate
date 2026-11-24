# Status

Last updated: 2026-11-24

**Phase:** 0 — Seed kernel, weeks 1–2 (`docs/spec.md` §11, §14).

**Done since last handoff:**
- Repository initialized on `main`; published to GitHub.
- `docs/spec.md` v0.2 committed.
- Documentation scaffold: `AGENTS.md`, `README.md`, `docs/STATUS.md`, `docs/DECISIONS.md`.

**In flight:** nothing.

**Next:** implement the first vertical slice from `docs/spec.md` §14 — `src/event.rs`, `src/state.rs`, `src/machine.rs`, `src/executor.rs` for the `hello` scenario, ending with `ev replay` producing a byte-identical `state.json`. No LLM, no knowledge, no review agent.

**Blockers:** none.

**Open questions:** see `docs/spec.md` §13. None block the seed kernel.
