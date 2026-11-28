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

## ADR-0015 — The seed-kernel hello log is frozen historical evidence
Date: 2026-11-28
Status: accepted
Decision: `tasks/hello` stays in the repository unchanged as the record of the week-1–2 seed run. Its committed `state.json` is not regenerated, and `ev replay tasks/hello` reports the `merged without passed gates and human approval` anomaly because the log predates the mandatory `ReviewAgent` gate (ADR-0007). The byte-identical replay guarantee is scoped to the reducer that produced the snapshot; a later gate-set change surfaces as an anomaly, never as a rewritten snapshot.
Why: The log is the first end-to-end proof of the name — template, sandbox gates, PR, human merge — and deleting or overwriting it destroys the only record of that milestone. Regenerating it needs a live reviewer, spends tokens, and contradicts the task's own spec ("No review agent") while replacing the historical snapshot with a different run. Appending a synthetic `ReviewAgent` pass would be gate circumvention by definition. Replaying the log remains a pure fold; the mismatch is the honest signal that the mandatory gate set grew.
Rejected: Regenerating the snapshot with a live model (cost, credentials, and it replaces evidence with a new artifact); appending a `review_agent` pass event (fabricated completion, violates "gates define completion"); deleting the task (loses the seed-run record); grandfathering tasks by creation time (weakens the mandatory gate set for every future replay).

## ADR-0016 — Lesson repetition is lexical similarity, not exact text
Date: 2026-11-28
Status: accepted
Decision: `count_lessons` groups bullets by the Dice coefficient of their stemmed content-token sets (threshold 3/5), after lowercasing, dropping stopwords and generic advice framing, and folding simple English plurals. A negation (`not`, `no`, `never`, `avoid`, `cannot`) on one side and not the other blocks a match. A group counts once per task file; its representative is the exact spelling found in the most files, ties broken by canonical key. Rule deduplication, the "already promoted" check, and `--lesson` selection use the same matcher. This supersedes the exact-text sentence in ADR-0014.
Why: exact canonical matching required three independent reviews to emit a byte-identical sentence, which made the promotion pause practically unreachable — the reviewer sees the spec and artifact, not prior lessons, so it has no reason to repeat wording. Paraphrases are the normal case. Lexical grouping makes the designed feedback loop observable without a model, a vector index, or a new crate, and promotion stays a human decision, so a false merge is reviewed rather than applied.
Rejected: exact matching (never fires in practice); lowering `PROMOTION_THRESHOLD` to 2 (changes how much evidence a rule needs and still misses paraphrases); embeddings or an external NLP crate (network, dependency budget, non-determinism, and the vector store the spec defers); a full stemming algorithm (more surface for false merges than the conservative plural fold needs); transitive single-linkage clustering (a chain of weak overlaps can merge unrelated advice, so each group is anchored to its most-supported spelling).

## ADR-0017 — The review agent sees recent lessons and reuses their wording
Date: 2026-11-28
Status: accepted
Decision: The review prompt carries the five most recent `lessons/*.md` files — the same section the executor sees — and instructs the reviewer to copy an earlier lesson's text exactly when it still applies. New issues still get new wording.
Why: Promotion counts a lesson across tasks, but the reviewer's information set did not include prior lessons, so it reworded every observation. Three real runs produced three lesson files with a maximum cross-file Dice of 0.44, so the promotion pause never fired. Repetition is only meaningful if the critic knows what has already been said; verbatim reuse also keeps the promoted rule stable instead of choosing one paraphrase arbitrarily. The reviewer still judges the current artifact, not the earlier verdicts, and the current spec and rules win.
Rejected: Relying on lexical similarity alone (real paraphrases are too divergent — measured); a lower threshold (it would approach 0.2 and merge distinct advice); semantic embeddings (dependency, network, non-determinism, and the deferred vector store); letting the executor copy lessons (the critic, not the executor, owns the lesson verdict); a per-prior-lesson "still applies" boolean in the response schema (changes the contract and needs a merge rule for the promoted text).

## ADR-0018 — LiveClient requests JSON object responses
Date: 2026-11-28
Status: accepted
Decision: Every `LiveClient` request sets `response_format: {"type":"json_object"}`. Both roles already promise one JSON object; the server now enforces it.
Why: Local OpenAI-compatible servers emitted JSON with unescaped quotes inside Rust source (`println!("{}", sum)`), so the manifest could not parse and no event was recorded. JSON mode constrains decoding at the server and fixed it without post-processing model text. The field is standard in the OpenAI-compatible wire format; providers that ignore it behave as before.
Rejected: Repairing invalid JSON client-side (cannot distinguish a structural quote from a content quote); retrying until the model escapes correctly (nondeterministic and wasteful); base64 or a line envelope for file contents (larger protocol change for the same result); making JSON mode optional (both roles require JSON, so there is no case for off).

## ADR-0019 — FormatClean is the fourth deterministic gate
Date: 2026-11-28
Status: accepted
Decision: `GateId::FormatClean` runs `cargo fmt --check` in the sandbox, after `LintsClean` and before `ReviewAgent`. The mandatory v0 set grows to five gates. The repository tree is rustfmt-clean under the pinned 1.100 toolchain, and `gate.rs` tests assert the command, slug, label, diff-line failure detail, and that the mandatory set only grows.
Why: Formatting is deterministic, offline, and cheap, and `cargo fmt --check` prints the first offending location, which is useful revision feedback. The sandbox image already installs rustfmt at build time (ADR-0003), so the gate needs no image, network, or dependency change. This is the first Month 2 self-edit.
Rejected: Running `cargo fmt` without `--check` (a gate must judge, never mutate the artifact); folding formatting into clippy (clippy does not check rustfmt output); formatting inside the executor (the executing agent would be marking its own work); a separate formatting service or tool (new dependency for a command already in the image).

## ADR-0020 — Gate sets are recorded per task, and drift needs a reasoned override
Date: 2026-11-28
Status: accepted
Decision: `TaskCreated` records the mandatory gate set in force (`gates`). `State.gates` carries that set for the task and `machine.rs` iterates it; logs written before the field fold to `LEGACY_V0` (the pre-`FormatClean` four) and serialize without the field, so replay stays byte-identical. Every phase change resets `State.phase_invocations`, and `next_command` stops with `RequestHumanReview` after `MAX_AGENT_INVOCATIONS_PER_PHASE = 5`. A task whose recorded set differs from the running binary's `MANDATORY_V0` cannot reach `Finalize { Merged }` on a bare approval: `ev approve --reason <text>` records `HumanApproved { reviewer, reason }`, the only override. A test asserts `LEGACY_V0` remains a subset of `MANDATORY_V0`.
Why: Adding `FormatClean` to a global list would have made every previously merged log fail `all_mandatory_passed`, turn its `Merged` event into an anomaly, and break the byte-identical replay guarantee. Recording the set in the event is the event-sourced answer: a task is judged by the gates it was created under, and gate-set history becomes explicit instead of implicit in the binary. The subset test is what rejects a self-edit that removes a historical gate: the artifact's own `TestsPass` gate fails. The reasoned approval is the merge-time guard for any drift the test cannot see, with no silent reconciliation. The per-phase counter is derived by the reducer, reset on every transition, and omitted when zero, so committed terminal snapshots and old logs stay unchanged.
Rejected: Reinterpreting old logs under the current set (breaks replay and rewrites evidence); recording a separate `GateSetVersion` event (duplicates `TaskCreated` and adds a second reducer input); making every approval carry a reason (noise, since only a real set difference needs one); keeping the counter in the executor (would break the pure `next_command` contract); diffing the artifact in `machine.rs` to detect gate edits (the machine has no base tree and the executor cannot certify its own artifact); relying on review alone (a gate weakening would be invisible to the machine and to replay).

## ADR-0021 — Self-edit tasks are marked by `## Target` and start from a repository copy
Date: 2026-11-28
Status: accepted
Decision: `Spec` gains an optional `target` list parsed from a `## Target` section. A target equal to `src` or starting with `src/` makes the task a self-edit. On its first invocation the executor finds the nearest ancestor of the task directory that contains `.git`, `Cargo.toml`, and `src/`, copies that repository into `project/` (skipping `.git`, `target`, `.tmp`, and generated `tasks/*/project` and `tasks/*/artifacts`), and sends the copied tree to the model. The prompt requires the complete tree back because the manifest replaces `project/`; the offline `TemplateClient` refuses the task. Targets must exist in the copy. Empty targets serialize like a legacy spec, so old logs replay byte-identically.
Why: The self-edit workflow needs a base artifact, and the event log must record how the task was classified. Deriving the base from the task directory rather than the caller's working directory keeps the copy deterministic and independent of where `ev` is invoked. A spec field is recorded inside `TaskCreated`, unlike an environment variable or CLI flag, so replay reaches the same conclusion. The copy is the separate checkout the gates judge: `project/` is gitignored and disposable, and the live tree is never mutated. Refusing `TemplateClient` prevents a self-edit from silently replacing the repository with `templates/bin`.
Rejected: An `EVENGATE_SELF_EDIT` environment variable or `--self-edit` flag (unrecorded, so replay could not reconstruct the decision); inferring a self-edit from the project slug (magic and easy to rename); copying from the process working directory (breaks when `ev` runs from another directory); keeping the live tree as `project/` (gates would mutate the repository); overlay manifests with deletions (needs a base-tree contract in the log; a full-tree manifest keeps `materialize` self-contained); refusing without a model but still seeding (leaves a misleading checkout after a failed run).

## ADR-0022 — The sandbox image pre-fetches Evengate's locked dependencies
Date: 2026-11-28
Status: accepted
Decision: `sandbox/Dockerfile` is built with the repository root as context, installs clippy and rustfmt, then runs `cargo fetch --locked` in `/opt/evengate-deps` against `Cargo.toml` and `Cargo.lock`. `.dockerignore` excludes `.git`, `.tmp`, `target`, and generated task trees. Gate containers remain `--network=none` with `CARGO_NET_OFFLINE=true`. `validate_path` also rejects `:` so a Windows drive prefix is rejected identically on Linux.
Why: A self-edit gate compiles Evengate, whose sources depend on `serde`, `serde_json`, and `ureq`; the previous image had an empty registry and failed with `no matching package named serde found`, so the self-edit path could not reach a gate. Fetching at image-build time is the only place network is available while gates stay offline and repeatable. The cross-platform colon check surfaced because the same tests now run in Linux. Updating the build context is required for the image to see the manifests.
Rejected: Running self-edit gates on the host (breaks the sandbox invariant and pushes network into the gate); mounting the host cargo registry (environment-specific and non-reproducible); vendoring crates into `project/` (huge manifests, binary-ish files in the PR, and review noise); allowing network during gates (nondeterministic); accepting the failing test on Linux (the artifact's own `TestsPass` gate would block every self-edit).

## ADR-0023 — Self-edits record a base tree and return partial manifests
Date: 2026-11-28
Status: accepted; supersedes the full-tree requirement in ADR-0021
Decision: `ev run` records the repository tree as `SelfEditBase` immediately after `TaskCreated`, before any model call, excluding `.git`, `target`, `.tmp`, and generated task trees. The executor prompt carries only the `## Target` files plus a repository path map; the model returns only the changed files. A self-edit manifest overlays `project/` instead of replacing it, and deletions are unsupported in v0. `ev materialize` writes `SelfEditBase` first and then overlays every executor manifest. The review prompt carries the repository path map and only the files that differ from the base. The offline `TemplateClient` still refuses self-edits.
Why: The full-tree protocol was unusable with the available local model. The repository prompt was about 414 KB (roughly 103k tokens) while `qwen2.5-coder:14b` has a 32k context, and the required full-tree response exceeds any practical output budget even on a 131k-context model. Recording the base in the log keeps materialization independent of the live checkout, while a target-scoped prompt and partial response fit the model and keep the log small. Overlay semantics match "edit these files"; rejecting deletions is an explicit v0 limit rather than a silent data-loss path.
Rejected: Keeping the full-tree protocol and using a hosted model (not available in the runtime and still expensive for a 400 KB echo); scoping only the prompt while replacing the tree (would delete every unlisted file); ambient base reconstruction from the live repository (not replayable after the repository moves); deleting files through the manifest (needs a second response field and a deletion policy; deferred); silent partial replacement (a stale artifact without a recorded base).

## ADR-0024 — A self-edit PR commits the base tree before the change
Date: 2026-11-28
Status: accepted
Decision: When a self-edit opens its PR and `project/` has no Git history yet, the executor moves the edited tree aside, writes the recorded `SelfEditBase` files into `project/`, initializes Git, and commits that tree on `main` as `ev: initialize <project>`. It then clears the worktree (keeping `.git`), restores the edited tree, checks out `ev/<project>`, and commits the change. Generated projects keep the existing empty base commit. `git diff main..ev/<project>` therefore contains only the self-edit.
Why: An empty base made every self-edit PR a repository-sized addition, so the human could not review the change. The base tree is already in the event log (ADR-0023), so the base commit is reproducible from the log and needs no live checkout. Reusing the existing push path keeps one PR implementation.
Rejected: Keeping the empty base (unreviewable diff); creating the base commit from the live repository (drifts from the recorded base and breaks replay); pushing only a patch file (not a branch that can be reviewed or merged); a separate `git worktree` per task (more moving parts for one commit).
