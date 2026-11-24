# Evengate

A deterministic, Rust-native software factory that builds and improves itself under human oversight.

**Evengate** = *event* + *gate*: all state is derived from an append-only event log, and nothing moves without passing a gate.

Status: seed kernel works for one scenario — generate a crate from `templates/bin/`, gate it, open a pull request. See `docs/STATUS.md`.

## Documentation

| File | Purpose |
|---|---|
| [`docs/spec.md`](docs/spec.md) | Authoritative design, v0.2 |
| [`docs/STATUS.md`](docs/STATUS.md) | Current phase, next step, blockers |
| [`docs/DECISIONS.md`](docs/DECISIONS.md) | Decision log with rationale |
| [`AGENTS.md`](AGENTS.md) | Rules for agents and contributors |

## Quickstart

```sh
docker build -t evengate-sandbox:1.100 sandbox   # once: clippy + rustfmt inside
cargo build --locked

cargo run --locked --bin ev -- run tasks/hello/spec.md
cargo run --locked --bin ev -- replay tasks/hello

# after reviewing the branch:
cargo run --locked --bin ev -- approve tasks/hello --reviewer <your-name>
cargo run --locked --bin ev -- replay tasks/hello
```

Binaries: `ev` (daily use) and `evengate` (scripts, CI), both built from `src/main.rs`.

Environment: `EVENGATE_SANDBOX_IMAGE`, `EVENGATE_SANDBOX_TIMEOUT_SECS`, `EVENGATE_TEMPLATES`.

## Working on it

Read `AGENTS.md`, then `docs/STATUS.md`. All repository documentation is written in English.
