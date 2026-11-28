# Status

Last updated: 2026-11-28

**Phase:** Month 2 continues. The executor now has a self-edit path: a spec can mark `## Target` files, a target under `src/` seeds `project/` from a copy of this repository, and the sandbox image can build Evengate offline so its own gates run on that copy.

**Done since last handoff:**
- `Spec` and `Context` carry an optional `target` list parsed from `## Target`. A target equal to `src` or under `src/` is a self-edit; empty targets serialize like a legacy spec, so `sum-cli` (37 events), `sum-cli-nary` (39), `sum-cli-help` (47), and `word-count` replay unchanged.
- On its first invocation a self-edit finds the nearest ancestor with `.git`, `Cargo.toml`, and `src/`, copies the repository into `project/` (skipping `.git`, `target`, `.tmp`, and generated `tasks/*/project|artifacts`), and sends that tree to the executor. The live checkout is never touched; the gates judge the copy. Missing targets fail before the model call, and `TemplateClient` refuses self-edits so `templates/bin` cannot replace the repository.
- The self-edit prompt requires the complete tree back. The manifest still replaces `project/`, so `ev materialize` reconstructs self-edits from the log without a base tree.
- `sandbox/Dockerfile` now builds from the repository root and runs `cargo fetch --locked` at image-build time. With the rebuilt `evengate-sandbox:1.100`, `cargo check --locked`, `cargo test --locked`, `cargo clippy --locked -- -D warnings`, and `cargo fmt --check` all pass `--network=none` on a copy of this repository.
- `validate_path` rejects `:` in manifest paths, so the Windows-drive test passes on Linux too. Without that, the artifact's own `TestsPass` gate would block every self-edit.
- Verified end to end with a throwaway self-edit task and a mock OpenAI-compatible endpoint: the executor was seeded from a repository copy, all five gates passed on that copy, the review verdict was recorded, and `PrOpened` fired. The smoke task, lesson, and mock were deleted.
- Checks: 102 tests per binary, `cargo check --locked`, `cargo clippy --locked -- -D warnings`, and `cargo fmt --check` pass. `sum-cli` (37), `sum-cli-nary` (39), `sum-cli-help` (47), and `word-count` (28) replay byte-identically.

**In flight:** nothing.

**Next:**
1. Run the first self-edit through the factory: write `tasks/<task-id>/spec.md` with a real Month 2 change and a `## Target` under `src/`, set `EVENGATE_LLM_URL`, run `ev run`, and review the PR that the gates produce.
2. Decide whether a self-edit PR should carry a base commit so the human reviews a diff instead of the whole repository; the current PR base is still an empty commit.

**Blockers:** none.

**Open questions:** `docs/spec.md` §13 items 1, 4, 5, 7 remain open; item 1's provider choice is configuration and the last factory runs used local Ollama. Items 2, 3, and 6 are settled by ADR-0003, ADR-0004, and ADR-0005.
