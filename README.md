# Evengate

A deterministic, Rust-native software factory that builds and improves itself under human oversight.

**Evengate** = *event* + *gate*: all state is derived from an append-only event log, and nothing moves without passing a gate.

Status: seed kernel (v0). The system does not exist yet; it is being built from `docs/spec.md`.

## Documentation

| File | Purpose |
|---|---|
| [`docs/spec.md`](docs/spec.md) | Authoritative design, v0.2 |
| [`docs/STATUS.md`](docs/STATUS.md) | Current phase, next step, blockers |
| [`docs/DECISIONS.md`](docs/DECISIONS.md) | Decision log with rationale |
| [`AGENTS.md`](AGENTS.md) | Rules for agents and contributors |

## Quickstart

No code yet. Planned interface:

```sh
ev run tasks/hello/spec.md   # execute a task spec end to end
ev replay tasks/hello        # reconstruct state offline, byte-identical
```

Planned binaries: `ev` (daily use) and `evengate` (scripts, CI), both built from `src/main.rs`.

## Working on it

Read `AGENTS.md`, then `docs/STATUS.md`. All repository documentation is written in English.
