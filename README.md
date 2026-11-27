# Evengate

A deterministic, Rust-native software factory that builds and improves itself under human oversight.

**Evengate** = *event* + *gate*: all state is derived from an append-only event log, and nothing moves without passing a gate.

Status: seed kernel works for one scenario — generate a crate from `templates/bin/`, gate it, open a pull request. See `docs/STATUS.md`.

## Documentation

| File | Purpose |
|---|---|
| [`docs/spec.md`](docs/spec.md) | Authoritative design, v0.7 |
| [`docs/STATUS.md`](docs/STATUS.md) | Current phase, next step, blockers |
| [`docs/DECISIONS.md`](docs/DECISIONS.md) | Decision log with rationale |
| [`AGENTS.md`](AGENTS.md) | Rules for agents and contributors |

## Quickstart

```sh
docker build -t evengate-sandbox:1.100 sandbox   # once: clippy + rustfmt inside
cargo build --locked

cargo run --locked --bin ev -- run tasks/hello/spec.md
cargo run --locked --bin ev -- replay tasks/hello

# after reviewing the branch:
cargo run --locked --bin ev -- approve tasks/hello --reviewer <your-name>
cargo run --locked --bin ev -- replay tasks/hello

# when a gate blocks the task:
cargo run --locked --bin ev -- revise tasks/hello --reviewer <your-name> --reason "fix the failing test"
cargo run --locked --bin ev -- abandon tasks/hello --reviewer <your-name> --reason "not worth finishing"
```

Binaries: `ev` (daily use) and `evengate` (scripts, CI), both built from `src/main.rs`.

Environment: `EVENGATE_SANDBOX_IMAGE`, `EVENGATE_SANDBOX_TIMEOUT_SECS`, `EVENGATE_TEMPLATES`, `EVENGATE_LESSONS_DIR` (default `lessons/`).

The executing agent defaults to the offline `TemplateClient`. Set `EVENGATE_LLM_URL` (an OpenAI-compatible chat-completions endpoint) to switch to `LiveClient`; `EVENGATE_LLM=template|live|recorded` overrides the choice. Live mode reads `EVENGATE_LLM_API_KEY`, `EVENGATE_MODEL_EXECUTOR`/`_PLANNER`/`_REVIEWER` (fallback `EVENGATE_MODEL`), `EVENGATE_LLM_TIMEOUT_SECS`, and `EVENGATE_LLM_MAX_TOKENS`.

The `ReviewAgent` gate is mandatory and needs a model. With no endpoint configured, `ev run` executes the template agent and the three deterministic gates, then stops at the review gate with an actionable error; re-run with `EVENGATE_LLM_URL` set to finish the review and open the PR. `ev replay` is always offline.

When a gate blocks a task, `ev revise` re-runs the executor with the failing verdicts and any human guidance, then reruns every gate on the new tree (at most two revisions). `ev abandon` ends a blocked task without a PR.

The review agent also returns 3–5 lessons. The executor writes them to `lessons/<task-id>.md`, and the next executor prompt includes the five most recent lesson files; the current spec always wins over inherited advice.

## Working on it

Read `AGENTS.md`, then `docs/STATUS.md`. All repository documentation is written in English.
