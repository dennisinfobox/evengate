# Evengate — Technical Specification v0.6

> A deterministic, Rust-native software factory that builds and improves itself under human oversight, and later produces pet projects in a familiar way.

**Changes from v0.5:** gate-blocked tasks have a bounded revision path — `ev revise` re-invokes the executor with the failing gate verdicts and optional human guidance, and `ev abandon` ends a blocked task without opening a PR (§7.2, §7.3, §7.6, ADR-0008).

**Changes from v0.4:** the `ReviewAgent` gate is mandatory and LLM-backed; it reviews the project tree after the deterministic gates and stops for a human when approval is withheld (§7.6, ADR-0007).

**Changes from v0.3:** the model boundary is concrete — a JSON file-manifest agent protocol, an OpenAI-compatible `LiveClient` over `ureq`, a prompt-hash-checked `RecordedClient`, and the offline `TemplateClient` as the default (§7.5, ADR-0006).

**Changes from v0.2:** concurrency settled as post-v1 task-level parallelism (§13.6, ADR-0005); no event-log rework required.

**Changes from v0.1:** the project is now named **Evengate** (event + gate). CLI is `ev` (short) / `evengate` (explicit). Crate and repository are `evengate`. A dedicated Naming section was added. The "event sourcing over reactive streams" principle was made explicit. All `factory`-prefixed identifiers were renamed.

---

## 1. Overview

**Evengate** is a personal software factory: a deterministic control plane written in Rust that orchestrates LLM agents through a strict, gate-checked pipeline. The human is the **owner and arbiter**; Evengate is the **executor and critic**. Over time Evengate grows its own capabilities, accumulates structured knowledge from every task, and reduces the human's role from coder to reviewer to occasional arbiter.

Evengate is **self-hosting in the goal, not in the starting point**: it cannot bootstrap itself from nothing. A minimal human-written **seed kernel** is required. After that, Evengate extends itself under human review.

---

## 2. Naming

| Aspect | Value |
|--------|-------|
| Project name | **Evengate** |
| Origin | **event** + **gate** — the two load-bearing pillars of the architecture |
| Secondary reading | "even gate" — honest, deterministic gates that either pass or fail, nothing in between |
| Crate name | `evengate` |
| Repository | `evengate` |
| Primary binary | `ev` (short, for daily use) |
| Secondary binary | `evengate` (explicit, for CI, scripts, and documentation) |
| One-line pitch | *"Evengate moves work through events and gates — and nothing moves without passing a gate."* |

The name is not decorative: it encodes exactly what the system is. Every time the name is spoken or typed, it reinforces the two invariants that must never break — the event log is the source of truth, and gates define completion.

`Cargo.toml`:

```toml
[package]
name = "evengate"

[[bin]]
name = "ev"
path = "src/main.rs"

[[bin]]
name = "evengate"
path = "src/main.rs"
```

Both binaries compile from the same `src/main.rs`. `ev` is the everyday command; `evengate` exists so scripts and CI can be explicit and grep-able.

---

## 3. Goals

| # | Goal | Measurable outcome |
|---|------|---------------------|
| G1 | Deterministic control plane | Same event log → byte-identical state, regardless of when replayed |
| G2 | Minimal third-party footprint | ≤ 7 direct crate dependencies in `Cargo.toml` |
| G3 | Self-improvement under review | From month 2, ≥ 50% of tasks are modifications to Evengate itself |
| G4 | Knowledge accumulation | Every task produces structured lessons that feed back into future tasks |
| G5 | Reliable completion | "Done" is defined by gates, never by the executing agent |
| G6 | Human oversight by design | Every task has explicit intervention points; nothing merges without human approval in v0 |

---

## 4. Non-Goals (v0)

- **Not** a general problem-solver for non-software domains.
- **Not** fully autonomous. Human review is required for every merge in v0.
- **Not** multi-stack. v0 produces **Rust projects only**.
- **Not** a web service. CLI + filesystem is the interface in v0.
- **Not** async. Blocking threads only.
- **Not** a database-backed system. Flat files + JSONL, versioned in git.
- **Not** a DSL for specs. Markdown with fixed headings.

---

## 5. Design Principles

1. **Determinism at the control layer.** State machine and event reduction are pure Rust functions. LLM calls, process execution, and filesystem I/O live behind explicit boundaries.
2. **Event sourcing as the foundation.** All state derives from an append-only event log. Replay is a first-class operation.
3. **Isolation of the non-deterministic.** The LLM lives behind an `LlmClient` trait. Recording and replaying LLM responses is how determinism survives across runs.
4. **Gates define completion.** Executors produce artifacts; gates decide whether artifacts are acceptable.
5. **Minimal dependencies.** Every crate must justify itself against a hand-rolled alternative.
6. **Blocking over async.** Threads and `std::sync` channels. No `tokio`, no `reqwest`, no `hyper`.
7. **Markdown specs, not DSLs.** Human-readable, diff-able, parseable with ~30 lines of Rust.
8. **Flat files over databases.** Git-versioned state is debuggable, replayable, and diffable. Databases come only when profiling proves they are needed.
9. **Cost visibility from day one.** Token usage and money are logged per event.
10. **No self-certification.** The executing agent cannot mark a task complete.
11. **Event sourcing over reactive streams.** The event log is the single source of truth; all derived state is a `fold` over it. Reactive libraries (`Rx`, `futures::Stream`) are deliberately excluded from the control plane because they introduce schedulers and async composition, both of which break byte-identical replay. Reactive ideas are borrowed as *patterns* (multiple projections, backpressure as budget gates), not as dependencies.
12. **The name is the contract.** *Event* and *gate* are not branding — they are the two invariants that must never be weakened. Any change that bypasses the event log or circumvents a gate is a regression by definition.

---

## 6. Architecture

### 6.1 High-level layers

```
┌─────────────────────────────────────────────────────┐
│  CLI (main.rs) — `ev` / `evengate`                  │
├─────────────────────────────────────────────────────┤
│  machine.rs   ← pure: fn next_command(state) -> Cmd │
│  state.rs     ← pure: fn reduce(state, event) -> St │
├─────────────────────────────────────────────────────┤
│  executor.rs  ← side effects: executes Command,     │
│                  appends Event                      │
├─────────────────────────────────────────────────────┤
│  llm/         ← trait LlmClient (recorded | live)   │
│  gate.rs      ← deterministic checks + review agent │
│  sandbox.rs   ← process isolation via external bin  │
│  knowledge.rs ← read/write structured artifacts     │
└─────────────────────────────────────────────────────┘
```

### 6.2 The determinism boundary

| Layer | Deterministic? | How |
|-------|----------------|-----|
| `machine.rs` | Yes | Pure function, no I/O, no time, no randomness |
| `state.rs` | Yes | Pure `reduce` over immutable events |
| `event.rs` | Yes | Serialization is stable (JSONL with sorted keys) |
| `executor.rs` | No (by design) | Wraps side effects; emits events |
| `llm/` | No (by design) | Live impl calls the API; `recorded` impl replays from the log |
| `gate.rs` | Partially | Most gates deterministic; review gate is LLM-backed and recorded |
| `sandbox.rs` | No (by design) | Spawns processes |

**Rule:** if code in `machine.rs` or `state.rs` needs I/O, time, or randomness, it must be refactored into a `Command` emitted from `machine.rs` and executed by `executor.rs`.

### 6.3 Event sourcing model

Every task lives in its own directory:

```
tasks/<task-id>/
  spec.md         -- immutable input (markdown spec)
  events.jsonl    -- append-only event log
  state.json      -- denormalized snapshot (derived, regenerable)
  artifacts/      -- produced files, patches, logs
```

**Replay guarantee:** `ev replay <task-id>` reads `events.jsonl`, folds through `reduce`, and produces a `State` identical to the live run. Live LLM responses are recorded as events, so replay never calls the network.

**Formal contract:**

```rust
fn reduce(state: State, event: Event) -> State   // pure, total
fn next_command(state: &State) -> Option<Command> // pure, deterministic
```

`next_command` returns `None` only when the task is in a terminal state (merged, rejected, or abandoned).

### 6.4 Command / Event cycle

```rust
loop {
    match machine::next_command(&state) {
        None => break,
        Some(cmd) => {
            let events = executor::run(cmd, &ctx)?;  // side effects happen here
            for ev in events {
                log::append(&ev)?;
                state = state::reduce(state, ev);
            }
        }
    }
}
```

The loop is boring on purpose. All intelligence lives in `next_command` and in the gates.

---

## 7. Module Breakdown

### 7.1 `event.rs` — the event vocabulary

```rust
pub enum Event {
    TaskCreated { spec: Spec },
    AgentInvoked { role: AgentRole, prompt_hash: String },
    AgentResponded { role: AgentRole, response: String, tokens: TokenUsage },
    CommandExecuted { cmd: String, exit_code: i32, stdout: String, stderr: String },
    GateEvaluated { gate: GateId, result: GateResult },
    HumanApproved { reviewer: String },
    HumanRejected { reviewer: String, reason: String },
    RevisionRequested { reviewer: String, guidance: Option<String> },
    HumanAbandoned { reviewer: String, reason: String },
    PrOpened { url: String },
    Merged { commit: String },
    TaskAbandoned { reason: String },
    Tick { now_unix: u64 },
}
```

All events are serialized as JSON Lines with stable field ordering. The log is the single source of truth.

### 7.2 `state.rs` — the reducer

```rust
pub struct State {
    pub phase: Phase,           // Drafting, Generating, Verifying, Revising, Reviewing, Merged, ...
    pub spec: Spec,
    pub artifacts: Vec<ArtifactRef>,
    pub gate_results: HashMap<GateId, GateResult>,
    pub token_spend: TokenUsage,
    pub human_decision: Option<HumanDecision>,
}
```

`reduce` must be total: every `(State, Event)` pair produces a valid `State`, even if the event is unexpected in the current phase (in which case it is recorded as an anomaly, not a panic).

### 7.3 `machine.rs` — the deterministic policy

```rust
pub fn next_command(state: &State) -> Option<Command> { ... }

pub enum Command {
    InvokeAgent { role: AgentRole, context: Context },
    RunGate { gate: GateId },
    RequestHumanReview { artifact: ArtifactRef },
    OpenPullRequest { branch: String },
    AwaitHuman,
    Finalize { outcome: Outcome },
}
```

**Invariants enforced here:**

- `InvokeAgent` is never issued for a role that already produced a response in the current phase without an intervening gate.
- `Finalize { Merged }` is unreachable unless every mandatory gate passed **and** a `HumanApproved` event exists.
- No command is issued twice in a row with identical parameters (loop protection).

**Revision path.** A gate that still fails after `MAX_GATE_ATTEMPTS`, or a review rejection, leaves the task in `Verifying` with `RequestHumanReview`. The human may then issue `RevisionRequested`, which is accepted only in `Verifying` with at least one non-passing verdict. The reducer snapshots those verdicts and the optional guidance into `State`, increments the revision counter, clears every gate verdict and attempt counter, and enters `Revising`. `next_command` then issues one `InvokeAgent` whose `Context` carries the failures; the response moves the task to `Generating` and back to `Verifying`, so every gate runs again on the new tree. Revisions are capped at `MAX_REVISIONS` (2); at the cap the machine requests a human again. `HumanAbandoned` ends the task from any non-terminal phase, while `TaskAbandoned` remains the consequence of rejecting a PR.

### 7.4 `executor.rs` — the impure boundary

Thin. For each `Command`, the executor:

1. Performs the side effect (LLM call, process spawn, git operation).
2. Emits one or more `Event`s.
3. Returns them for the outer loop to log and reduce.

The executor must never mutate `State` directly. It only returns events.

### 7.5 `llm/` — the model boundary

```rust
pub trait LlmClient {
    fn complete(&self, req: &LlmRequest) -> Result<LlmResponse, String>;
}

pub struct LlmRequest { pub role: AgentRole, pub prompt: String }
pub struct LlmResponse { pub text: String, pub tokens: TokenUsage }

pub struct LiveClient { /* ureq; OpenAI-compatible chat completions */ }
pub struct RecordedClient { /* replays AgentResponded events by prompt hash */ }
pub struct TemplateClient { /* offline: renders templates/ into a manifest */ }
```

**Agent protocol.** A model response is one JSON object mapping relative paths to file contents:

```json
{"files": {"src/main.rs": "fn main() {}\n"}}
```

The executor validates every path (relative, `/` separators, no `.`/`..`, bounded count and size), replaces `project/` with the manifest, and only then runs the gates. Prompt assembly is deterministic: the full spec and the reference template are rendered in declaration/sorted order, so the recorded prompt hash is reproducible.

**LiveClient** speaks the OpenAI-compatible chat-completions wire format. URL, key, per-role model, timeout, and optional token cap come from the environment — `EVENGATE_LLM_URL`, `EVENGATE_LLM_API_KEY`, `EVENGATE_MODEL_PLANNER|EXECUTOR|REVIEWER` (fallback `EVENGATE_MODEL`), `EVENGATE_LLM_TIMEOUT_SECS`, `EVENGATE_LLM_MAX_TOKENS`. The wire format is fixed; the vendor is not.

**Client selection.** `EVENGATE_LLM=template|live|recorded`. Unset means `live` when `EVENGATE_LLM_URL` is set, otherwise `template`. `TemplateClient` is the deterministic offline agent: it renders `templates/bin` the same way the seed kernel did, so the hello scenario stays reproducible without credentials.

**RecordedClient** reconstructs `AgentInvoked` → `AgentResponded` pairs from the log in order and refuses a request whose role or prompt hash differs. A changed input therefore fails loudly instead of consuming an answer to a different question. Live responses are stored verbatim in `AgentResponded`, so the event log is sufficient to reconstruct what the model returned. `ev replay` itself remains a pure fold and never calls the network.

### 7.6 `gate.rs` — the quality barrier

A gate is a pure function from an artifact set to a verdict:

```rust
pub enum GateResult {
    Pass,
    Fail { reason: String },
    NeedsHuman { question: String },
}

pub trait Gate {
    fn id(&self) -> GateId;
    fn check(&self, ctx: &GateContext) -> GateResult;
}
```

**Mandatory gates for v0:**

| Gate | Kind | Check |
|------|------|-------|
| `Compiles` | Deterministic | `cargo check --locked` in sandbox, exit 0 |
| `TestsPass` | Deterministic | `cargo test --locked`, all green |
| `LintsClean` | Deterministic | `cargo clippy --locked -- -D warnings` |
| `ReviewAgent` | LLM (recorded) | Project tree (path → contents) → structured verdict `{approve: bool, issues: []}` |
| `HumanApproval` | Human | Explicit `HumanApproved` event |

**Rule:** the executing agent has no mechanism to mark a gate as passed. Gates own their verdicts.

`Compiles`, `TestsPass`, `LintsClean`, and `ReviewAgent` are the mandatory v0 gates, in that order. The review agent runs on the artifact the deterministic gates accepted, records its request and raw response as `Reviewer` events, and maps a withheld approval to `Fail` with the issues as the reason. A rejection stops the run for a human immediately rather than re-asking the same tree; an unusable verdict becomes `NeedsHuman`. Review requires a model: with the offline `TemplateClient` the run stops at this gate with an actionable error and resumes once a model is configured. The executor cannot pass this gate on the reviewer's behalf. From a blocked state the human can `ev revise` (executor re-runs with the verdicts) or `ev abandon`; see §7.3.

### 7.7 `sandbox.rs` — process isolation

- Linux: `bwrap` or `firejail` invoked via `std::process::Command`.
- Cross-platform: `docker run --rm --network=none -v <workdir>:<workdir>`.

**Mandatory constraints for every spawned process:**

- Network disabled, except an explicit allowlist to the LLM API endpoint (or no network at all when the LLM runs outside the sandbox).
- Working directory is a **copy** of the task's project directory.
- Wall-clock timeout (default: 300s).
- Output size cap (default: 1 MiB per stream); excess is truncated with a marker.

### 7.8 `knowledge.rs` — the accumulator

Knowledge is **structured, versioned, and fed back into prompts**. Not a vector store. Not a RAG index in v0.

**Artifact types:**

| Type | Written by | Consumed by |
|------|------------|-------------|
| `adr/<n>-<slug>.md` | Human or ReviewAgent | All future tasks in the project |
| `lessons/<task-id>.md` | ReviewAgent after each task | Prompt assembly for similar tasks |
| `rules/<project>.md` | Human, or promoted from lessons | Every task in the project |
| `codebase-map.md` | Regenerated periodically | Planning agent |
| `patterns/<name>.md` | Promoted from repeated successes | Template selection |

**Feedback mechanism (v0):** after every task, the `ReviewAgent` appends a `lessons/<task-id>.md` with 3–5 bullet points ("what went wrong, what to do differently"). The next task's prompt includes:

- the current `spec.md`,
- `rules/<project>.md`,
- the 5 most recent `lessons/*.md` (by recency, not similarity — a similarity index comes later),
- `codebase-map.md`.

**Promotion path:** if the same lesson appears ≥ 3 times across tasks, `machine.rs` emits a `Command::RequestHumanReview` proposing to promote it to `rules/`. Promotion is always a human decision.

This is Evengate's real moat. A vector index can be added later without changing the artifact schema.

---

## 8. Specification Format

Task specs are markdown with a fixed heading schema. Parsed by a hand-rolled parser (~30 lines), never by a generic markdown library.

```markdown
# <Task title>

## Goal
One paragraph. What the user wants, in outcome terms.

## Acceptance
- [ ] Concrete, testable criterion
- [ ] ...

## Constraints
- Language, crates allowed, style rules, performance limits.

## Non-goals
- Explicitly out of scope.

## Context
Links to relevant files, ADRs, prior tasks.

## Open questions
Anything the agent should ask about before starting.
```

If `Goal`, `Acceptance`, or `Constraints` is missing, the spec is rejected at `TaskCreated` and never reaches the LLM. This is cheaper than letting a confused agent burn tokens.

---

## 9. Crate Policy

**Direct dependencies (target ≤ 7):**

| Crate | Justification |
|-------|---------------|
| `serde` | Derive macros for `Event`, `Command`, `State`. Hand-rolling is error-prone. |
| `serde_json` | JSONL encoding. Non-negotiable. |
| `ureq` | Blocking HTTP client for LLM API. Replaces `reqwest` + `tokio`. |
| `rustls` *(via `ureq`)* | TLS without OpenSSL; enabled by `ureq`'s default `rustls` feature, not a direct dependency. |
| `webpki-roots` *(via `ureq`)* | Embedded trust store; enabled with `ureq`'s `rustls` feature. No system CA dependency. |
| `anyhow` | Error propagation. Reduces boilerplate in `executor.rs` |
| `toml` *(optional)* | Only if config files become necessary. Env vars otherwise. |

**Explicitly rejected:**

| Rejected | Reason | Alternative |
|----------|--------|-------------|
| `tokio`, `async-std` | Async is not needed and harms determinism | `std::thread` |
| `reqwest`, `hyper` | Pull in `tokio` | `ureq` |
| `clap` | Command count < 10 in v0 | `std::env::args` + 20-line parser |
| `log` + `env_logger` | 40+ lines of custom JSONL writer is simpler | `evengate::log` module |
| `git2` | Large C library bindings | `std::process::Command` calling `git` |
| `sqlx`, `rusqlite` | Flat files are sufficient and more debuggable | JSONL + git |
| `langchain`-equivalents | Magic, hard to reason about, hard to make deterministic | `machine.rs` written by hand |
| `pulldown-cmark` | Spec parsing is 4 headings | 30-line splitter |
| `rxrust`, `futures` | Schedulers and async composition break replay | `Iterator::fold` over the event log |

**Version discipline:** `Cargo.lock` is committed, `cargo build --locked` is enforced in CI and in Evengate's own build gate. Toolchain is pinned via `rust-toolchain.toml`.

---

## 10. Self-Improvement Model

### 10.1 Phases

| Phase | Duration | Human role | Evengate role |
|-------|----------|------------|---------------|
| 0. Seed | Week 1–2 | Writes the kernel by hand | Nothing (does not exist yet) |
| 1. First tasks | Week 3–6 | Reviews every PR | Executes tasks end-to-end |
| 2. Self-edits | Month 2 | Reviews every PR | Also edits Evengate's own source |
| 3. Comfort | Month 3+ | Reviews PRs, arbitrates disputes | Adds UX, templates, queue |

### 10.2 What counts as self-improvement

A task is a **self-improvement task** if its spec targets a file under `src/` of Evengate. Evengate is allowed to modify itself under these rules:

- The `HumanApproval` gate is mandatory and non-waivable for self-edits.
- Evengate's own `Compiles` and `TestsPass` gates run on the modified Evengate **before** the PR is opened, using a separate checkout.
- A self-edit that removes or weakens a gate is rejected by `machine.rs` with no path forward except human override (recorded as an explicit `HumanApproved { reviewer, reason }` event). This is principle 12 in action: the name is the contract.

### 10.3 Loop protection

- No command repeated with identical arguments back-to-back.
- Maximum N agent invocations per phase (default: 5).
- Maximum N retries per gate (default: 2). After that, `NeedsHuman`.
- Maximum N human-requested revisions per task (default: 2). Each revision resets the per-gate attempt counter, so total executor retries stay bounded.

---

## 11. Roadmap

### Week 1–2 — Seed Kernel
- `event.rs`, `state.rs`, `machine.rs` for **one** scenario: *create a new Rust project from template*.
- **No LLM.** The "agent" is a stub that copies a template and fills a placeholder.
- Event log writes correctly; `ev replay` reproduces state byte-for-byte.
- CI runs `cargo check`, `test`, `clippy -- -D warnings`.
- **Exit criterion:** one task runs end-to-end; PR is opened by `git` CLI; human merges.

### Week 3–4 — LLM and Gates
- `llm::LiveClient` over `ureq`. `llm::RecordedClient` for replay.
- `Gate` trait; `Compiles`, `TestsPass`, `LintsClean` implemented.
- Sandbox integration (`bwrap` or `docker`).
- **Exit criterion:** Evengate builds a real CLI utility from a markdown spec; all gates pass; human merges.

### Week 5–6 — Knowledge and Review
- `knowledge.rs`: write `lessons/<task>.md` after each task; feed last 5 into next prompt.
- `ReviewAgent` gate (LLM-backed, recorded).
- **Exit criterion:** two consecutive tasks in the same project; second task's prompt visibly includes lessons from the first.

### Month 2 — Self-Edits
- First self-improvement task: "add a gate that checks `cargo fmt`".
- Loop protection and gate-immutability rules enforced in `machine.rs`.
- **Exit criterion:** Evengate has modified its own source and the change is merged.

### Month 3+ — Comfort
- Queue of tasks (`tasks/queue.jsonl`).
- Template selection by project type.
- CLI ergonomics (still `std::env::args`-based).
- **Exit criterion:** creating a pet project is a one-command operation (`ev new <spec>`); human reviews the PR.

---

## 12. Risks and Mitigations

| Risk | Mitigation |
|------|------------|
| Agent claims completion when gates fail | Only gates define completion; `Finalize{Merged}` is unreachable without all gates + human approval |
| Non-determinism creeps into `machine.rs` | Code review rule: no I/O, no time, no randomness, no `unwrap` on `Result` from I/O in `machine.rs` and `state.rs`. Enforced by clippy lints (`disallowed_methods`) |
| Token cost explodes | Per-event token accounting; hard budget per task; `machine.rs` emits `NeedsHuman` if budget exceeded |
| Loss of context on long tasks | Tasks must be splittable; Evengate refuses specs with more than one "Goal" paragraph |
| Premature architecture | v0 has exactly one scenario. New scenarios require a new spec, not a refactor |
| Over-engineering crate selection | Crate policy is in this document and enforced by review; add a crate → update this spec first |
| Knowledge decay (lessons pile up unread) | Promotion rule: ≥ 3 repetitions → human review → `rules/`. Otherwise lessons age out after 30 days |
| Agent self-certification loophole | The executing role has no gate-passing API. Gates are invoked only by `executor.rs` in response to `Command::RunGate` |
| Name drifts from behavior | Principle 12: any change bypassing the event log or circumventing a gate is a regression by definition. The name is the contract |

---

## 13. Open Questions and Decisions

Open design questions keep stable numbers. A resolved question records its answer in place and points to the ADR that settled it; unresolved ones are to be resolved before Week 3.

### 13.1 LLM provider and model per role
Planning, executing, and reviewing may use different models. Which, and at what budget?

**Wire format settled by ADR-0006:** `LiveClient` speaks OpenAI-compatible chat completions, with the URL, key, and model per role supplied by the environment. The provider and budget are therefore configuration and remain open; no provider is wired into the code.

### 13.2 Sandbox choice on the target platform
`bwrap` (Linux-only, lightweight) vs `docker` (cross-platform, heavier). Pick one for v0.

### 13.3 Git hosting
Local bare repo, GitHub, or self-hosted? Affects `OpenPullRequest` implementation.

### 13.4 Multi-repo strategy
One Evengate repo + one repo per produced project? Or a monorepo with subdirectories?

### 13.5 Human review interface
Terminal-based (`less` on the PR diff) or web (GitHub/GitLab UI)? v0 assumes terminal + `git diff`.

### 13.6 Concurrency
*Resolved by ADR-0005.*

**v0 is strictly sequential by design.** Parallelism is not needed until
the human reviewer becomes the bottleneck (typically 3+ tasks in flight
per day, month 4–6 of the project).

**The per-task event log does not need a rework.** Each task is an
independent event-sourced aggregate with its own `events.jsonl`, `State`,
and replay. Parallelism is *embarrassingly parallel at the task level*:
N tasks run as N independent state machines in N threads, each folding
its own log. The schemas of `Event`, `State`, `Command`, and the
signatures of `reduce` and `next_command` are unchanged.

**What parallelism adds is a coordination layer, not a log rework:**

1. A **factory-level event log** (`factory.jsonl` at the repo root)
   recording cross-task events: `TaskScheduled`, `TaskClaimed`,
   `TaskStarted`, `TaskFinished`, `BudgetReserved`, `BudgetReleased`.
   This is a second projection, not a replacement.
2. **Task claiming** via a file lock (`tasks/<id>/.lock`) or a
   `TaskClaimed` factory event, so no two workers run the same task.
3. **A shared budget and rate limiter** behind `Mutex`, reserved per
   task before `InvokeAgent`.
4. **Per-task git worktrees** (`git worktree add`) so concurrent tasks
   never share a working directory.

**No global sequence number is introduced.** Per-task local ordering plus
wall-clock timestamps plus explicit `TaskDependsOn` events (if and when
dependencies appear) are sufficient. A global counter would be a
coordination point without a consumer.

**Cheap intermediate step before building the scheduler:** run two `ev`
processes in two terminals on two tasks with separate worktrees. If this
works, the scheduler is pure automation of a proven design. If it breaks,
the breakage identifies the real coordination points to automate.

**Out of scope for v0 and v1:** two workers on the same task (crash
recovery), cross-task dependencies, distributed workers. Each of these
is a separate spec, not a refactor of this one.

### 13.7 Domain and public presence
`evengate.dev` availability, GitHub org name, whether to publish the crate publicly or keep it private. For a personal project, private is fine; if published, reserve the name on crates.io early.

---

## 14. First Concrete Step

Write `src/event.rs`, `src/state.rs`, `src/machine.rs`, and `src/executor.rs` for the single scenario:

> *"Create a new Rust binary crate `hello` from the template at `templates/bin/`, run `cargo check` and `cargo test` in a sandbox, and open a PR."*

No LLM. No knowledge. No review agent. Just the state machine, the event log, the reducer, and `ev replay` proving the loop is deterministic. Everything else in this spec is built on top of that foundation, and none of it can be safely built before it exists.

**Definition of done for this step:**

1. `ev run tasks/hello/spec.md` executes the scenario end-to-end without human input until the PR is opened.
2. `events.jsonl` contains a complete, ordered record of every step.
3. `ev replay tasks/hello` reconstructs a `state.json` byte-identical to the live run, with no network calls.
4. `cargo check --locked`, `cargo test --locked`, and `cargo clippy --locked -- -D warnings` all pass on Evengate itself.
5. The produced `hello` crate passes the same three checks inside the sandbox.
6. You merge the PR by hand. That merge is the first `HumanApproved` event in Evengate's history — and the moment the name becomes true: work moved through events and gates, and nothing moved without passing a gate.