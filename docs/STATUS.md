# Status

Last updated: 2026-11-27

**Phase:** 1 — First tasks (week 5–6). The first model-authored project is merged.

**Done since last handoff:**
- Ran real tasks against the local Ollama OpenAI-compatible endpoint with `qwen2.5-coder:14b`.
- `tasks/sum-cli`: first manifest was malformed JSON (retried), then the deterministic gates passed but the reviewer rejected weak tests. One revision produced a pure `parse_args`, real valid/invalid tests, and all four gates passed. The owner approved; the task merged at `c8ab29c`, and 37 events replay byte-identically. Lessons were written to `lessons/sum-cli.md`.
- `tasks/word-count`: tests failed because the model used the forbidden `tempfile` crate. Revision 1 fixed that but broke a test import; revision 2 fixed byte counting but repeated the same import error. The two-revision cap stopped the task, as designed.
- These runs exposed and fixed two defects: gate failures now report the first actionable diagnostic (compiler error, panic, clippy warning) instead of cargo's `Compiling`/`Checking` line, and revision prompts now include the current project tree instead of only the reference template.
- Spec bumped to v0.8 (§7.3, §7.4); README updated.
- Checks pass: `cargo check --locked`, `cargo test --locked` (67 tests), `cargo clippy --locked -- -D warnings`.

**In flight:** nothing.

**Next:**
1. Decide the capped `word-count` task: abandon it or rerun from a sharper spec.
2. Sandbox hardening; an execution replay path to rebuild a gitignored `project/` from the log.
3. Wire `rules/<project>.md` and `codebase-map.md` into prompt assembly, and implement the ≥3-occurrence promotion path from §7.8.

**Blockers:** none.

**Open questions:** `docs/spec.md` §13 items 1, 4, 5, 7 remain open; §13.1's provider/budget choice is configuration after ADR-0006. Item 2 is settled by ADR-0003, item 3 by ADR-0004, item 6 by ADR-0005.
