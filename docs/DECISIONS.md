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
