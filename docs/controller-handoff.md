# Controller Handoff — ansidrama capture redesign

> **This workstream is complete.** The handoff it used to contain described work
> that had not started yet; that text is superseded by the implementation itself
> and is not preserved here. Kept as a short record of where the detail lives, so
> the next session does not mistake a finished workstream for a pending one.

Status: implemented, reviewed and merged as one branch (23 commits).
Date: 2026-08-14

## What landed

`record` no longer decides that an app has finished drawing by watching the PTY
go quiet. Three phases replace that guess: `record.rs` drives (send input, wait,
mark), `sampler.rs` samples the grid continuously into a state log on its own
thread, and `assemble.rs` turns that log plus input marks into frames as a pure
function. Only surviving frames are rasterised. `term::settle()` and its config
keys are gone.

Two consequences do the work: a state the app reached is never lost, so being
early stops being fatal; and a script can **declare** completion with
`await = "..."`, so a failed expectation aborts the run instead of silently
writing a bad frame.

## Where the detail lives

- Design (binding): `docs/superpowers/specs/2026-08-14-capture-core-await-design.md`
- Implementation plan: `docs/superpowers/plans/2026-08-14-capture-core-await.md`
- User-facing config, `await`, `animated`, and the frame manifest: `README.md`
  and `man/ansidrama.1`
- Acceptance procedure and its run history: `docs/regression-gate.md`
- Breaking changes and migration: `CHANGES.md`

## Still open — for whoever picks this up next

- **The mdmost tour has not been re-recorded.** `demo/mdmost.toml` in that repo
  still carries `settle_ms`, which now fails to parse — that is the intended
  migration, not a regression. Its `docs/maintainer-notes.md` bisect procedure
  also derives a frame index from the run log's running total, which no longer
  works now that a scene can contribute app-driven frames; `manifest.tsv` exists
  to replace that arithmetic with a lookup.
- **The headline performance claim is unmeasured.** The design's motivation was
  ~76s of dead air across the tour's 254 waits collapsing to roughly 10s. Nothing
  has re-measured it on the real tour. `ANSIDRAMA_TRACE=1` was kept alive
  specifically so it can be.
- **The mdmost theme frame remains undiagnosed** and was never reproduced (7/7
  clean, including on a deliberately unfixed binary). This work does not claim to
  fix it; it changes the failure mode from "wrong screen held" to "right screen,
  one beat late", and makes the scene declarable via `await`.
- Three residual test-debt items are recorded in the branch's final review.
