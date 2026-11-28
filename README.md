# Evengate

A deterministic, Rust-native software factory that builds and improves itself under human oversight.

**Evengate** = *event* + *gate*: all state is derived from an append-only event log, and nothing moves without passing a gate.

Status: month 2 — self-edit tasks record a base tree, return partial edits, run Evengate's own gates on the copy, and open a PR whose diff contains only the change; the mandatory gates include `cargo fmt --check`, each task records its gate set, and merging across a gate-set change needs a reasoned human override. See `docs/STATUS.md`.

## Documentation

| File | Purpose |
|---|---|
| [`docs/spec.md`](docs/spec.md) | Authoritative design, v0.19 |
| [`docs/STATUS.md`](docs/STATUS.md) | Current phase, next step, blockers |
| [`docs/DECISIONS.md`](docs/DECISIONS.md) | Decision log with rationale |
| [`AGENTS.md`](AGENTS.md) | Rules for agents and contributors |

## Quickstart

```sh
docker build -f sandbox/Dockerfile -t evengate-sandbox:1.100 .   # once: clippy, rustfmt, locked deps
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

The executing agent defaults to the offline `TemplateClient`. Set `EVENGATE_LLM_URL` (an OpenAI-compatible chat-completions endpoint) to switch to `LiveClient`; `EVENGATE_LLM=template|live|recorded` overrides the choice. Live requests ask for a JSON object (`response_format`), so a model cannot return an unescaped-quote reply. Live mode reads `EVENGATE_LLM_API_KEY`, `EVENGATE_MODEL_EXECUTOR`/`_PLANNER`/`_REVIEWER` (fallback `EVENGATE_MODEL`), `EVENGATE_LLM_TIMEOUT_SECS`, and `EVENGATE_LLM_MAX_TOKENS`.

The `ReviewAgent` gate is mandatory and needs a model. With no endpoint configured, `ev run` executes the template agent and the four deterministic gates — `cargo check`, `cargo test`, `cargo clippy -- -D warnings`, and `cargo fmt --check` — then stops at the review gate with an actionable error; re-run with `EVENGATE_LLM_URL` set to finish the review and open the PR. `ev replay` is always offline.

A task spec with a `## Target` bullet under `src/` is a self-edit. `ev run` records the repository tree as `SelfEditBase` (without `.git`, `target`, `.tmp`, or generated task artifacts), seeds `tasks/<task-dir>/project/` from it, and sends only the target files plus a repository path map to the executor. The agent returns only changed files; they overlay the base. A single-file target returns that file as plain text (no JSON escaping), while a directory or multi-file target returns the manifest. The gates judge that copy in the sandbox, so the live checkout is untouched. A self-edit needs a model: the offline template agent refuses, because rendering `templates/bin` would replace Evengate with an unrelated crate. The review prompt sees only the files changed against the base, and the PR commits the base tree on `main` first, so the human reviews a diff. The sandbox image pre-fetches the locked dependency set at build time, so `cargo check`, `test`, `clippy`, and `fmt` run offline on the copy.

`TaskCreated` records the mandatory gate set in force for that task, so adding or removing a gate never reinterprets an old log. A task whose recorded set differs from the running binary's can merge only with `ev approve <task-dir> --reviewer <name> --reason <text>`; a bare approval stops at `RequestHumanReview`.

When a gate blocks a task, `ev revise` re-runs the executor with the failing verdicts, any human guidance, and the current project tree, then reruns every gate (at most two revisions). `ev abandon` ends a blocked task without a PR.

The review agent also returns 3–5 lessons. The executor writes them to `lessons/<task-id>.md`; both the next executor and the next reviewer prompt include the five most recent lesson files, and the reviewer is asked to copy a still-applicable lesson verbatim. The current spec always wins over inherited advice.

Project-wide rules live in `rules/<project>.md`, where `<project>` is the spec title slug. They join both the executor and the reviewer prompt on every invocation; a missing or blank file adds nothing, and an unreadable file fails the run.

After each review the executor groups similar lessons across `lessons/*.md` — stemmed content tokens with a Dice coefficient of at least 3/5, and no disagreement on negation. A group seen in three task files pauses the task before its PR: `ev promote` appends its representative wording to `rules/<project>.md` and continues; `ev dismiss` declines for that task. Dismissal is task-scoped, so a later task may propose the same unpromoted lesson again. Promotion stays a human decision; lexical matching can propose a near-miss, never apply one.

`project/` and `lessons/` are derived state. `ev materialize <task-dir>` rebuilds both from `events.jsonl` without a model or network: the last executor manifest becomes `project/`, and the last usable reviewer verdict becomes the lesson file. `state.json` is untouched. Only recorded files are recreated, so a local `.git` or `target/` inside `project/` is dropped; the bare remote under `artifacts/` is not.

## Working on it

Read `AGENTS.md`, then `docs/STATUS.md`. All repository documentation is written in English.
