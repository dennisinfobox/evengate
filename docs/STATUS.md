# Status

Last updated: 2026-11-28

**Phase:** Month 2 — the first self-edit produced by the factory is merged. A self-edit task recorded its base, the executor returned one file, all five gates passed on the copy, the reviewer approved, and the PR diff contained only the change.

**Done since last handoff:**
- `TaskCreated` is followed by `SelfEditBase`, the repository tree recorded before any model call. `project/` and the PR base are rebuilt from the log; the live checkout is never mutated.
- A self-edit executor prompt carries the `## Target` files and a repository path map. A single-file target uses `RequestFormat::SingleFile` (plain text, a code-editor system prompt, and no JSON `response_format`); a directory or multi-file target keeps the JSON manifest. Responses overlay the base, so unlisted files survive; deletions are not supported in v0.
- The review prompt for a self-edit carries the task spec, the path map, and only the files that differ from `SelfEditBase`.
- `ev materialize` writes the base and then overlays every executor response. A self-edit PR commits the base on `main` first, so `git diff main..ev/<project>` is the change.
- First real self-edit: `tasks/self-edit-slug-test` added two tests for `slug` separator runs. `qwen2.5-coder:14b` returned the file on the first attempt; `Compiles`, `TestsPass`, `LintsClean`, `FormatClean`, and `ReviewAgent` all passed; the human approved; the patch is applied to the live tree. Tests are now 110.
- Model failures found and handled: JSON manifest truncation at an unescaped serde quote, a JSON map instead of a file, and `{}` from JSON mode were each fixed by the single-file protocol and system prompt. A behavior-changing target-validation task still exceeded two revisions and was abandoned without a merge.
- Checks: 110 tests per binary, `cargo check --locked`, `cargo clippy --locked -- -D warnings`, and `cargo fmt --check` pass. `sum-cli` (37), `sum-cli-nary` (39), `sum-cli-help` (47), and `word-count` (28) replay byte-identically.

**In flight:** nothing.

**Next:**
1. Run a self-edit that changes behavior, not only tests. Split target validation into a narrower task (reject `/...` and `..` only) or use a stronger model for the full rule.
2. Decide whether a single-file response needs a pre-gate sanity check; the deterministic gates currently catch truncation, but the failure costs a revision.
3. Decide whether a self-edit may delete a file; the overlay protocol has no deletion field.

**Blockers:** none.

**Open questions:** `docs/spec.md` §13 items 1, 4, 5, 7 remain open; item 1's provider choice is configuration and the last factory runs used local Ollama. Items 2, 3, and 6 are settled by ADR-0003, ADR-0004, and ADR-0005.
