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

