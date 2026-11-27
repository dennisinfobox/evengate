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
