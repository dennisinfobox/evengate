# Evengate

A deterministic, Rust-native software factory that builds and improves itself under human oversight.

**Evengate** = *event* + *gate*: all state is derived from an append-only event log, and nothing moves without passing a gate.

Status: phase 1 — the first model-authored project is merged, review lessons feed the next prompt, and a lesson repeated across three tasks pauses for human promotion. See `docs/STATUS.md`.

## Documentation

| File | Purpose |
|---|---|
| [`docs/spec.md`](docs/spec.md) | Authoritative design, v0.13 |
| [`docs/STATUS.md`](docs/STATUS.md) | Current phase, next step, blockers |
| [`docs/DECISIONS.md`](docs/DECISIONS.md) | Decision log with rationale |
| [`AGENTS.md`](AGENTS.md) | Rules for agents and contributors |

## Quickstart

```sh
docker build -t evengate-sandbox:1.100 sandbox   # once: clippy + rustfmt inside
cargo build --locked

# start or resume a task; every task directory carries its own spec.md
cargo run --locked --bin ev -- run tasks/<task-dir>/spec.md

# verify recorded state and rebuild derived artifacts
cargo run --locked --bin ev -- replay tasks/sum-cli
cargo run --locked --bin ev -- materialize tasks/sum-cli

# after reviewing the branch:
cargo run --locked --bin ev -- approve <task-dir> --reviewer <your-name>

# when a gate blocks the task:
cargo run --locked --bin ev -- revise <task-dir> --reviewer <your-name> --reason "fix the failing test"
cargo run --locked --bin ev -- abandon <task-dir> --reviewer <your-name> --reason "not worth finishing"

# when a repeated lesson is proposed:
cargo run --locked --bin ev -- promote <task-dir> --reviewer <your-name>
cargo run --locked --bin ev -- dismiss <task-dir> --reviewer <your-name>
```

`tasks/hello` is the frozen seed-kernel run. It predates the mandatory review gate, so `ev replay tasks/hello` intentionally reports a pre-gate anomaly instead of matching its snapshot (ADR-0015).

Binaries: `ev` (daily use) and `evengate` (scripts, CI), both built from `src/main.rs`.

Environment: `EVENGATE_SANDBOX_IMAGE`, `EVENGATE_SANDBOX_TIMEOUT_SECS`, `EVENGATE_SANDBOX_MEMORY` (default `2g`), `EVENGATE_SANDBOX_CPUS` (default `2`), `EVENGATE_SANDBOX_PIDS` (default `256`), `EVENGATE_TEMPLATES`, `EVENGATE_LESSONS_DIR` (default `lessons/`), `EVENGATE_RULES_DIR` (default `rules/`).

Gate containers run read-only with no network, a read-only project mount, a 1 GiB executable `/tmp`, dropped capabilities, `no-new-privileges`, and the memory/PID/CPU caps above.

The executing agent defaults to the offline `TemplateClient`. Set `EVENGATE_LLM_URL` (an OpenAI-compatible chat-completions endpoint) to switch to `LiveClient`; `EVENGATE_LLM=template|live|recorded` overrides the choice. Live mode reads `EVENGATE_LLM_API_KEY`, `EVENGATE_MODEL_EXECUTOR`/`_PLANNER`/`_REVIEWER` (fallback `EVENGATE_MODEL`), `EVENGATE_LLM_TIMEOUT_SECS`, and `EVENGATE_LLM_MAX_TOKENS`.

The `ReviewAgent` gate is mandatory and needs a model. With no endpoint configured, `ev run` executes the template agent and the three deterministic gates, then stops at the review gate with an actionable error; re-run with `EVENGATE_LLM_URL` set to finish the review and open the PR. `ev replay` is always offline.

When a gate blocks a task, `ev revise` re-runs the executor with the failing verdicts, any human guidance, and the current project tree, then reruns every gate (at most two revisions). `ev abandon` ends a blocked task without a PR.

The review agent also returns 3–5 lessons. The executor writes them to `lessons/<task-id>.md`, and the next executor prompt includes the five most recent lesson files; the current spec always wins over inherited advice.

Project-wide rules live in `rules/<project>.md`, where `<project>` is the spec title slug. They join both the executor and the reviewer prompt on every invocation; a missing or blank file adds nothing, and an unreadable file fails the run.

After each review the executor counts canonical lessons across `lessons/*.md`. A lesson seen in three tasks pauses the task before its PR: `ev promote` appends it to `rules/<project>.md` and continues; `ev dismiss` declines for that task. Dismissal is task-scoped, so a later task may propose the same unpromoted lesson again.

`project/` and `lessons/` are derived state. `ev materialize <task-dir>` rebuilds both from `events.jsonl` without a model or network: the last executor manifest becomes `project/`, and the last usable reviewer verdict becomes the lesson file. `state.json` is untouched. Only recorded files are recreated, so a local `.git` or `target/` inside `project/` is dropped; the bare remote under `artifacts/` is not.

## Working on it

Read `AGENTS.md`, then `docs/STATUS.md`. All repository documentation is written in English.
