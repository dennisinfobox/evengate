# Status

Last updated: 2026-11-28

**Phase:** Month 2 continues. The self-edit path is now usable with a local model: the repository base is recorded in the log, the executor sees only target files, responses overlay that base, the reviewer sees only the changed files, and the PR diff contains only the change.

**Done since last handoff:**
- `TaskCreated` is followed by `SelfEditBase`, the repository tree recorded before any model call (excluding `.git`, `target`, `.tmp`, and generated task trees). The task log is sufficient to rebuild `project/` and the PR base.
- A self-edit executor prompt carries the `## Target` files and a repository path map, not the whole tree. The response manifest overlays `project/`; unlisted files survive. Deletions are not supported in v0. The offline `TemplateClient` still refuses self-edits.
- The review prompt for a self-edit carries the task spec, the path map, and only the files that differ from `SelfEditBase`, plus a deleted-file list. A non-self-edit task still reviews the complete project.
- `ev materialize` writes `SelfEditBase` first and then overlays every executor response, so execution replay needs no live checkout.
- A self-edit PR commits `SelfEditBase` on `main` as `ev: initialize <project>` before the feature branch; `git diff main..ev/<project>` contains only the change. Generated projects keep the empty base commit.
- The first two real self-edit attempts exposed model failures: `qwen2.5-coder:14b` truncated the JSON manifest at the first unescaped serde attribute quote, and `llama3.1:8b` returned a JSON map instead of the file. A single-file self-edit now switches to `RequestFormat::SingleFile`, whose system prompt demands the file and nothing else; the format is in the prompt hash, the request omits JSON mode, a fence is unwrapped, and a trailing newline is ensured. A third attempt returned `{}` because JSON mode was still set; that task was also abandoned and removed.
- Checks: 108 tests per binary, `cargo check --locked`, `cargo clippy --locked -- -D warnings`, and `cargo fmt --check` pass. `sum-cli` (37), `sum-cli-nary` (39), `sum-cli-help` (47), and `word-count` (28) replay byte-identically.

**In flight:** nothing.

**Next:**
1. Run the first real self-edit through the factory: write `tasks/<task-id>/spec.md` with a small, testable change and a `## Target` under `src/`, point `EVENGATE_LLM_URL` at the local Ollama endpoint, run `ev run`, review the PR diff, and merge the reviewed change.
2. After that run, decide whether a self-edit task should be able to delete a file; the current overlay protocol has no deletion field.

**Blockers:** none.

**Open questions:** `docs/spec.md` §13 items 1, 4, 5, 7 remain open; item 1's provider choice is configuration and the last factory runs used local Ollama. Items 2, 3, and 6 are settled by ADR-0003, ADR-0004, and ADR-0005.
