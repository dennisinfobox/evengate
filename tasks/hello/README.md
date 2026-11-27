# hello — frozen seed-kernel run

This directory is the event log of the week-1–2 seed run from `docs/spec.md` §14:
the template agent copied `templates/bin/`, the deterministic gates passed, the
PR was opened against a local bare remote, and the human merged it.

It is historical evidence, not a template for new tasks. Its `state.json` was
written by the seed-kernel reducer before the `ReviewAgent` gate became
mandatory (ADR-0007). Replaying it under the current gate set records
`merged without passed gates and human approval` and exits with a snapshot
mismatch; that anomaly is expected and is the reason the snapshot is not
regenerated (§6.3, ADR-0015). The spec explicitly forbids a review agent, so do
not re-run it as a current task.
