# AGENTS.md

Operating contract for every agent and automation working in this repository.
Read this first, then `docs/STATUS.md`, then `git log -10`.

## Documentation language

All repository artifacts are written in **English**: documentation, specs, ADRs, code comments, commit messages, status notes, and file names. Conversation with the owner may happen in any language; the repository does not. This applies to new files and to edits of existing ones.

## What this project is

Evengate is a personal software factory: a deterministic Rust control plane that moves work through an append-only event log and a fixed set of gates. The human is the owner and arbiter; agents execute and are checked by gates. v0 produces Rust projects only and is CLI-driven (binary `ev`, alias `evengate`).

## Non-negotiables

- **Event log is the source of truth.** All state is a pure `fold` over append-only events. `ev replay` must reconstruct byte-identical state.
- **Gates define completion.** The executing agent may never mark its own task done.
- **No async runtime.** No `tokio`, `reqwest`, `hyper`. Blocking threads and `std::sync` only.
- **Dependency budget.** At most 7 direct crates in `Cargo.toml`; each one justified.
- **Flat files, JSONL, git.** No database in v0.
- **Specs are Markdown with fixed headings** (`docs/spec.md` §8), not a DSL.
- **The name is the contract.** Any change that bypasses the event log or circumvents a gate is a regression by definition.

## Where things live

| Path | Contents |
|---|---|
| `docs/spec.md` | Authoritative design. Change deliberately, bump the version line. |
| `docs/STATUS.md` | Session handoff. Rewritten every session; one screen. |
| `docs/DECISIONS.md` | Append-only decision log (ADR-lite). One entry per real decision. |
| `src/` | Code, laid out as `docs/spec.md` §7 specifies. |
| `tasks/` | Task specs produced and consumed by the factory. |

`docs/spec.md` is the single source of truth for design. Do not copy its content into `AGENTS.md`, `README.md`, or `STATUS.md`; link instead.

## Work loop

1. Read `AGENTS.md`, `docs/STATUS.md`, `git log -10`.
2. Take the next item from `STATUS.md` → *Next*. If it is ambiguous, stop and ask the owner.
3. Implement the smallest vertical slice that satisfies the acceptance criteria.
4. Verify once, at the end, with the narrowest checks that cover the change:
   - `cargo check --locked`
   - `cargo test --locked`
   - `cargo clippy --locked -- -D warnings`
5. Rewrite `docs/STATUS.md` and append an ADR if a decision was made.
6. Commit with a one-line subject and a short body.

## Definition of done

- Behavior matches `docs/spec.md`; no stubs, placeholders, or "follow-up" deferrals.
- The checks in step 4 pass.
- `docs/STATUS.md` reflects the new reality.
- Decisions made along the way are in `docs/DECISIONS.md`.
- No unreferenced dead files, no duplicate specs, no chat transcripts committed.

## Writing `STATUS.md`

Rewrite, never append. Keep it under one screen:

```
Phase: ...
Done since last handoff: ...
In flight: ...
Next: ...
Blockers: ...
Open questions: ...
```
