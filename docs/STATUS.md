# Status

Last updated: 2026-11-28

**Phase:** 1 — First tasks (week 5–6). The first model-authored project is merged.

**Done since last handoff:**
- Closed the `tasks/hello` question: it stays frozen as the seed-run record. Its snapshot predates the mandatory `ReviewAgent` gate, so `ev replay tasks/hello` is expected to exit with the `merged without passed gates and human approval` anomaly. No gate event was fabricated and the snapshot was not regenerated. Spec v0.13 scopes the replay guarantee to the reducer that wrote the snapshot; ADR-0015 records the decision, `tasks/hello/README.md` states the provenance.
- README quickstart now replays and materializes `tasks/sum-cli` (byte-identical) and uses `<task-dir>` placeholders for decisions; spec version reference corrected.
- Re-verified the mechanism on copies: `replay tasks/hello` → mismatch plus anomaly, `replay tasks/sum-cli` → 37 events, byte-identical.
- Prepared `tasks/sum-cli-nary/spec.md`, the second task in the `sum-cli` project: sum any number of integer arguments with `i64` overflow reported as an error, plus tests for valid, negative, missing, invalid, and overflow inputs. The title stays `Sum CLI` so the project slug — and therefore `rules/sum-cli.md` — matches the first task. Verified the spec parses by starting it against a missing template directory; `TaskCreated` was recorded and the run stopped before any agent or gate.
- Prepared `tasks/sum-cli-help/spec.md`, the third task in the same project: add `--help` and `--version` while keeping negative addends and the overflow error, with tests for flags, invalid input, and overflow. Three review runs produce three lesson files, the precondition for a natural promotion pause. Same parse check, same passing checks.
- Loosened lesson matching so paraphrases count as repetitions: stemmed content-token sets, stopwords and advice framing dropped, Dice ≥ 3/5, negation-aware, grouped once per task file around the most-supported spelling. Rule dedupe, the already-promoted check, and `--lesson` selection use the same matcher. Spec v0.14, ADR-0016; tests cover paraphrase grouping, stable representatives, negation, and rule suppression (84 tests pass).

**In flight:** nothing.

**Next:**
1. Run `tasks/sum-cli-nary/spec.md`, then `tasks/sum-cli-help/spec.md`, to exercise the knowledge loop end to end (week 5–6 exit criterion). Each prompt must visibly carry the prior lesson files. After the third review, `lessons/` holds three files; a similar lesson present in all three pauses the task before its PR for `ev promote`/`ev dismiss`. Requires a live model endpoint in the shell (`EVENGATE_LLM_URL`, `EVENGATE_LLM_API_KEY`, `EVENGATE_MODEL` or per-role models).
2. `codebase-map.md` joins prompt assembly once a planner role has a consumer.

**Blockers:** No live-model configuration (`EVENGATE_*`) is present in this environment, so the second- and third-task runs cannot start here.

**Open questions:** `docs/spec.md` §13 items 1, 4, 5, 7 remain open; §13.1's provider/budget choice is configuration after ADR-0006. Item 2 is settled by ADR-0003, item 3 by ADR-0004, item 6 by ADR-0005. The exact-text repetition question is settled by ADR-0016.
