# Status

Last updated: 2026-11-27

**Phase:** 1 — First tasks (week 3–4). Deterministic gates, sandbox, PR flow, and replay are done; the LLM boundary is implemented and awaits a first live run.

**Done since last handoff:**
- New `src/llm.rs`: `LlmClient` trait, `LlmRequest`/`LlmResponse`, prompt hashing, and the JSON file-manifest agent protocol (`{"files": {path: contents}}`).
- `LiveClient` posts OpenAI-compatible chat completions over blocking `ureq`: per-role model, bearer key, timeout, token cap, capped error bodies. Tested against a local HTTP stub for request shape, auth, usage, and non-2xx handling.
- `RecordedClient` rebuilds `AgentInvoked`/`AgentResponded` pairs from the log and refuses role/prompt-hash mismatches.
- `TemplateClient` renders `templates/` into the same manifest and stays the offline default.
- `executor::InvokeAgent` builds a deterministic prompt from the full spec plus the template, calls the selected client, validates and applies the manifest to `project/`, then records the raw response and token usage.
- Selection: `EVENGATE_LLM=template|live|recorded`; `EVENGATE_LLM_URL` implies `live`. Every other command works without LLM configuration.
- Spec bumped to v0.4 (§7.5, §9, §13.1); ADR-0006 recorded; README documents the environment.
- Checks pass: `cargo check --locked`, `cargo test --locked` (37 tests), `cargo clippy --locked -- -D warnings`. An offline scratch task ran through all three gates and opened a PR; `ev replay` reproduced `state.json` byte-identically (19 events).

**In flight:** nothing.

**Next:**
1. Run one task with a real OpenAI-compatible endpoint and land the first model-authored project (needs `EVENGATE_LLM_URL`/key/model in the environment).
2. `ReviewAgent` gate (LLM-backed, recorded) after the deterministic gates.
3. Sandbox hardening: confirm the copy-in/copy-out contract and timeout behaviour under failure.
4. If a checked-out task must rebuild its gitignored `project/` from the log, add an execution replay path; `ev replay` is currently a state-only fold, so `EVENGATE_LLM=recorded` has no CLI flow yet.

**Blockers:** none.

**Open questions:** `docs/spec.md` §13 items 1, 4, 5, 7 remain open; §13.1's provider/budget choice is configuration after ADR-0006, which settled the wire format. Item 2 is settled by ADR-0003, item 3 by ADR-0004, item 6 by ADR-0005.
