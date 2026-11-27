# Decisions

Append-only. Newest at the bottom. One entry per real decision; do not edit past entries — supersede them with a new one.

Format:

```
## ADR-NNNN — Title
Date: YYYY-MM-DD
Status: accepted | superseded by ADR-NNNN
Decision: one sentence.
Why: the constraint that forced it.
Rejected: alternatives and why they lost.
```

---

## ADR-0001 — Repository documentation is English-only
Date: 2026-11-24
Status: accepted
Decision: All repository artifacts — docs, specs, ADRs, comments, commit messages, status notes — are written in English, regardless of the language used in conversation.
Why: Mixed-language documentation fragments search, diffing, and agent context; English is the common denominator for tooling and future contributors.
Rejected: Bilingual docs (duplication drifts); Russian-only (narrows tooling and audience).

## ADR-0002 — Documentation layout: spec, status, decisions, contract
Date: 2026-11-24
Status: accepted
Decision: Design truth lives in `docs/spec.md`; session handoff in `docs/STATUS.md`; rationale in `docs/DECISIONS.md`; agent/contributor rules in `AGENTS.md`; `README.md` is the human entry point only.
Why: One owner per kind of information prevents divergence. `STATUS.md` is rewritten each session; specs and ADRs change only deliberately.
Rejected: Single monolithic document (status churn pollutes design history); wiki/issue-tracker-only (outside the repo, not versioned with the code).

## ADR-0003 — Sandbox: Docker with a locally built Rust image
Date: 2026-11-24
Status: accepted
Decision: Gates run in Docker containers from `evengate-sandbox:1.100`, built once from `sandbox/Dockerfile`. Containers run with `--network=none`, a bind mount of a throwaway copy of the project, a 300 s timeout, and 1 MiB output caps.
Why: Docker is available on the target Windows machine, unlike `bwrap`. The official `rust:1.100` image ships without clippy and rustfmt, so those components are installed at image-build time and never at gate time; gates therefore stay offline and repeatable.
Rejected: `bwrap` (Linux-only); pulling the official image per gate (no clippy/rustfmt); installing components during a gate (needs network, breaks determinism).

## ADR-0004 — v0 pull requests are pushed branches in a local bare repository
Date: 2026-11-24
Status: accepted
Decision: `OpenPullRequest` commits the generated project on a branch named `ev/<project>` and pushes it to `tasks/<task>/artifacts/remote.git`; the `PrOpened` event records a `file://` URL with the branch. Git hosting (open question 3 in the spec) stays out of the kernel until week 3.
Why: The seed kernel must run offline and deterministically. A local bare remote exercises the real git CLI path — branch, commit, push — without credentials, network, or repository-creation APIs.
Rejected: GitHub API (adds network, tokens, and a repo-creation side effect to the kernel); `git request-pull` files (not a branch that can be reviewed or merged).

## ADR-0005 — Concurrency: task-level parallelism behind a coordination layer
Date: 2026-11-25
Status: accepted
Decision: v0 stays strictly sequential. Future parallelism runs N independent per-task state machines and adds only a coordination layer — a factory-level `factory.jsonl`, task claiming, a shared budget/rate limiter, and per-task git worktrees — leaving `Event`, `State`, `Command`, `reduce`, and `next_command` unchanged.
Why: Each task is already an independent event-sourced aggregate with its own log and replay, so the existing design parallelizes at the task boundary without touching schemas. A global sequence number would be a coordination point with no consumer; per-task order, wall-clock timestamps, and explicit `TaskDependsOn` events suffice. Building the scheduler before two `ev` processes run in two worktrees would automate an unproven design.
Rejected: Global event log or sequence number (serializes independent tasks, no consumer); reworking per-task logs (unnecessary); scheduling two workers on one task, cross-task dependencies, and distributed workers (each is a separate spec, out of scope for v0 and v1).

## ADR-0006 — Agent output is a JSON file manifest over an OpenAI-compatible endpoint
Date: 2026-11-27
Status: accepted
Decision: Every `LlmClient` returns a `{"files": {"relative/path": "contents"}}` manifest; the executor validates it, replaces `project/`, and only then runs gates. `LiveClient` speaks OpenAI-compatible chat completions configured entirely by environment variables; `TemplateClient` stays the deterministic offline default, and `RecordedClient` replays logged responses only when the role and prompt hash match.
Why: Gates judge files, not prose, so the agent boundary must produce a project tree in one recorded, bounded, diffable answer. A path→contents manifest is trivial to validate and record verbatim; the OpenAI-compatible wire format keeps spec §13.1 (provider and budget) a configuration decision, while `ureq` keeps the control plane blocking and free of an async runtime.
Rejected: Unified diffs (need a repository-state contract and fail opaquely); tool-call loops (scheduler-like complexity and more hidden state for no v0 consumer); hardcoding one vendor's API (forecloses §13.1); keeping the template copy outside the agent boundary (would leave response recording with no production consumer).

## ADR-0007 — ReviewAgent is a mandatory fourth gate that reviews the project tree
Date: 2026-11-27
Status: accepted
Decision: `ReviewAgent` is the last mandatory v0 gate. It runs after `Compiles`, `TestsPass`, and `LintsClean`, sends the task spec plus every project text file (excluding `.git` and `target`) to the Reviewer model, records the request and raw response as `AgentInvoked`/`AgentResponded` with role `reviewer`, and maps `{approve, issues[]}` to `Pass` or `Fail`. A withheld approval stops the run for a human immediately instead of re-asking the same tree; an unusable verdict becomes `NeedsHuman`. Without a model the gate refuses to run, and the task resumes at this gate once one is configured.
Why: The spec's gate table requires an LLM-backed review before the PR, and gates must judge the real artifact rather than the executing agent's summary. Re-sending a rejected artifact to the same model burns tokens without changing the artifact, so the human is the next step; recording the verdict and its token usage keeps replay deterministic and cost visible. A missing model must never become an implicit approval.
Rejected: A unified-diff input (needs a base tree that does not exist before the PR is opened, and adds a second artifact contract); automatic executor re-invocation on review failure (needs a revision phase and feedback-in-prompt plumbing; not in this slice); treating `TemplateClient` as a reviewer (self-certification by the executing agent); retrying the review gate (same artifact, extra cost, nondeterministic verdict).

## ADR-0008 — Gate failure recovery is a bounded, human-initiated revision
Date: 2026-11-27
Status: accepted
Decision: A task blocked by a gate (a verdict still failing after `MAX_GATE_ATTEMPTS`, or a review rejection) accepts `RevisionRequested` only in `Verifying`. The reducer snapshots the non-passing verdicts plus optional human guidance into `State`, clears all gate verdicts and attempt counters, increments a revision counter, and enters `Revising`; `next_command` re-invokes the executor once with those failures in its context, and the response returns the task to `Verifying` so every gate reruns on the new tree. Revisions are capped at `MAX_REVISIONS = 2`. `HumanAbandoned` ends any non-terminal task without a PR; `TaskAbandoned` stays the consequence of a human PR rejection.
Why: A blocked run with no path forward leaves the only recovery outside the event log (deleting state, editing files by hand). Revision keeps the loop inside the same contract: the executor is still the only producer of the artifact, the gates re-judge it, and the human decides when to spend another round. Snapshotting the verdicts in the reducer makes the re-invocation prompt a pure function of state and keeps replay byte-identical. Clearing attempts per revision is safe only because the revision count is bounded; together they cap total retries.
Rejected: Automatic re-invocation without a human event (the gate would become advisory and burn tokens indefinitely); carrying stale gate verdicts across revisions (the executor might replace the tree while the old passes still count); unbounded revisions (a broken template or endpoint could loop forever); a separate `RetryGate` command that resends the same tree (the review verdict is about the artifact, not the request); editing `project/` by hand between runs (unrecorded state outside the log).

## ADR-0009 — Committed task logs keep absolute home paths
Date: 2026-11-27
Status: accepted
Decision: `tasks/<task>/events.jsonl` and `state.json` may contain absolute paths from the owner's machine, such as `C:\Users\Dennis\...`, in recorded command output. These paths are not treated as sensitive and stay in the repository history. The pre-push scan still blocks API keys, tokens, private keys, passwords, credential files, and authenticated URLs.
Why: The event log is the evidence of a run and must replay byte-identically; rewriting recorded paths would either alter that evidence or require a history rewrite. A local username alone is low risk in the owner's personal repository, and the generated `project/` and `artifacts/` trees that produce most paths are already gitignored.
Rejected: Rewriting history and force-pushing (breaks every clone and commit SHA for a username); excluding logs from git (loses the replay evidence and `git log` trace); normalizing paths at write time (changes recorded command output and diverges from what the process actually printed).

## ADR-0010 — Lessons travel from the review verdict into the next executor prompt
Date: 2026-11-27
Status: accepted
Decision: The review protocol gains `lessons: [String]` (3–5 bullets). When the reviewer returns lessons, the executor writes `lessons/<task-id>.md` under the lessons root (`lessons/` in the working directory by default, `EVENGATE_LESSONS_DIR` overrides), overwriting the file on a revision. Executor prompt assembly reads the five most recent `*.md` files, newest first by modification time with file-name ties, and renders them under `# Lessons from previous tasks` with an explicit statement that the current spec and constraints win. No new event is added: the raw verdict stays in `AgentResponded`, which is the log source for the file. `rules/<project>.md` and `codebase-map.md` remain future slices.
Why: The reviewer is already the critic that judges the artifact, so asking it for lessons costs no extra call and keeps one agent boundary. Lessons must outlive `project/`, which is gitignored and replaced on every revision, so they live in a separate versioned directory. Recording the raw verdict preserves replay: `ev replay` stays a pure fold, while an execution replay can rebuild the file. Recency is the spec's designated order for v0; reading mtimes happens only in the impure executor, never in `reduce` or `next_command`.
Rejected: A separate planner call for lessons (extra tokens and a second protocol for the same critic); a `LessonRecorded` event (duplicates the verdict already in `AgentResponded` and has no state-machine consumer); writing lessons inside `project/` (destroyed by every revision and gitignored); a similarity index (spec defers it); ordering lessons by task id or a global counter (no global order exists without a coordination layer).

## ADR-0011 — Execution replay rebuilds project/ and lessons from the log
Date: 2026-11-27
Status: accepted
Decision: `ev materialize <task-dir>` applies `AgentResponded` events in log order to disk: each executor manifest replaces `project/`, and the last usable reviewer verdict rewrites `lessons/<task-id>.md`. It never calls a model, spawns a process, mutates `state.json`, or appends events.
Why: `project/` and the lesson file are gitignored derivatives of the log, so a fresh clone must be able to reproduce the artifact a human reviewed without spending tokens or trusting a hand-edited directory. Keeping it a separate command preserves the pure-fold contract of `ev replay`: replay proves state equality, materialize proves artifact equality. The executor already owns manifest validation and lesson writing, so reconstruction uses the same code path as the live run.
Rejected: Extending `ev replay` with side effects (makes the determinism check depend on disk state and weakens the name); rebuilding from `CommandExecuted` output (contains compiler noise, not the manifest); recording a separate materialized event (duplicates the log and has no reducer consumer); requiring the network or a model (defeats the purpose of deriving from the log).

## ADR-0012 — Gate containers are read-only and resource-capped
Date: 2026-11-28
Status: accepted
Decision: Every gate container runs with `--init`, `--network=none`, `--read-only`, a read-only project bind mount, a 1 GiB executable `/tmp` tmpfs, `--memory`/`--memory-swap` 2 GiB, `--pids-limit` 256, `--cpus` 2, `--cap-drop=ALL`, and `no-new-privileges`. Memory, CPU, and PID caps are overridable with `EVENGATE_SANDBOX_MEMORY`, `EVENGATE_SANDBOX_CPUS`, and `EVENGATE_SANDBOX_PIDS`.
Why: Model-generated crates are untrusted code. The existing no-network, timeout, and output caps did not stop a build script or test from consuming host memory, forking without bound, spinning every core, or writing outside the disposable copy. A read-only root and project mount leave a bounded tmpfs as the only writable surface, and dropping capabilities removes privileged operations a root process in the container would otherwise retain. Docker 27 on the target Windows host accepts these flags, and a real `cargo test`/`cargo clippy` run passes with them.
Rejected: Leaving the rootfs writable (a build script can modify the image layer or `/etc`); a writable project mount (tests can mutate the artifact outside the log); swap instead of a hard memory cap (hides pressure and slows the host); per-gate bespoke limits (one policy, environment-overridable); `bwrap`/`firejail` (Linux-only, ADR-0003).

## ADR-0013 — Project rules are prompt context for the executor and the reviewer
Date: 2026-11-28
Status: accepted
Decision: The executor and the review gate read `rules/<project>.md` at prompt-assembly time, where `<project>` is the spec-title slug and the root is `rules/` or `EVENGATE_RULES_DIR`. A present file becomes a `# Project rules` section after the task spec; a missing or blank file adds nothing; an unreadable file fails the invocation. Rules are never cached in `State`.
Why: Rules are human-authored project constraints, so a task must see them on first generation and on every revision, and the critic that approves the artifact must check the same constraints. Reading them inside the impure executor keeps `machine.rs` and `state.rs` pure, adds no event, and lets a human edit apply to the next invocation while the recorded prompt hash still proves which prompt produced the response. `codebase-map.md` stays out because v0 has no planner consumer.
Rejected: Storing rules in `State` (needs an event and a reducer rule for external, editable configuration); folding rules into `Spec` (breaks the immutable spec input and forces log rewrites when a rule changes); executor-only wiring (the reviewer could approve an artifact that violates a project rule); a `RuleRead` event (no reducer consumer and duplicates the file); silently ignoring an unreadable rules file (a gate-adjacent constraint must fail loudly).

## ADR-0014 — Lesson promotion pauses the task before its PR
Date: 2026-11-28
Status: accepted
Decision: After a review writes lessons, the executor counts each canonical lesson once per lesson file across `lessons/*.md` and records `LessonRepeated` for counts of at least 3 that are not already bullets in `rules/<project>.md`. The reducer stores those counts. Once every gate passes, `machine.rs` emits `RequestHumanReview` with artifact kind `promotion` before `OpenPullRequest`. `ev promote` appends the lesson to `rules/<project>.md` and records `LessonPromoted`; `ev dismiss` records `LessonDismissed` only. One lesson is decided per invocation, in canonical key order; the task then asks for the next decision or continues.
Why: The spec requires a human decision on repeated lessons and the machine is pure, so repetition must reach `State` through events. Counting in the impure executor can see every task's lesson file; recording only the aggregate keeps `reduce` free of I/O and replay a pure fold. Pausing after the gates means the artifact already exists and no tokens are spent after the proposal. `RequestHumanReview` already means "stop for a human", so reusing it keeps one interaction path. Dismissal is task-scoped because per-task logs have no factory projection (ADR-0005); the next task proposes an unpromoted lesson again.
Rejected: Counting inside `reduce` (reads files, breaks purity and byte-identical replay); a global promotion log (coordination layer out of scope, ADR-0005); writing `rules/` automatically (promotion is a human decision); a separate `Promote` command variant (duplicates the review stop and its CLI path); appending dismissed lessons to rules (would make them binding prompt content); blocking before the review gate (the decision could change the rules after a review call was already paid for).
