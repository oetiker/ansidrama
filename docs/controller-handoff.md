# Controller Handoff — ansidrama capture redesign

> Starter pack for the next controller session. This handoff lives in ONE
> worktree — run `git worktree list` first and confirm this is the workstream
> you're resuming. Read this first, then `git log <handoff-commit>..HEAD` for
> everything that changed since. Detail is NOT here — it's in git + the
> superpowers plan/ledger/docs (§6). Before you rewrite this file at your own
> handoff: read the previous version (`git show HEAD:docs/controller-handoff.md`)
> and carry forward any lesson in §4/§5 that is still true. Fresh synthesis,
> not blank page. On merge into another branch, rewrite that branch's handoff
> to the merged reality — do not merge or preserve this text.

Handoff commit: `98a79e3`   Date: 2026-08-14   Reason: rollover before implementation
Worktree / branch: main checkout (`/home/oetiker/checkouts/ansidrama`) @ `main`
Trunk at time of writing: `main` @ `98a79e3` — **reader: if trunk has moved, §2 is provisionally stale; if trunk now contains this branch's HEAD, this file is a tombstone** (`git merge-base --is-ancestor HEAD main`)
Sibling worktrees: none — this is the only worktree (`git worktree list`). This line cannot see worktrees created later; check yourself.

## 1. Mission

`record` decides when an app has finished responding by watching the PTY go
quiet, then captures **one** frame. That question is undecidable from timing
alone — a quiet PTY reads the same whether the app finished drawing or hasn't
started — and because capture happens once, a wrong guess **loses** the state
rather than merely mistiming it.

The redesign replaces that with three phases: `record.rs` drives, `sampler.rs`
samples the grid continuously into a state log, `assemble.rs` turns the log
plus input marks into frames. Two consequences do the work: a state the app
reached is never lost (being early stops being fatal), and a script can
**declare** completion with `await = "..."`, so a failed expectation aborts the
run instead of silently writing a bad frame.

Design and plan are written, reviewed and committed. **Nothing is implemented.**
Next session starts at Task 1.

## 2. Where we are now

As of the handoff commit — re-derive per §8, do not inherit:

- `98a79e3` — the disarm fix + regression test + two debug facilities (see §4).
- `10865ae` — the 9-task implementation plan.
- `271dd87` — the design spec, revised after a `paad:pushback` review.
- `66c4449` — the original spec.

Working tree clean, 44 tests green, clippy clean. v0.2.0 is released and public.

The user chose **subagent-driven vs inline execution has NOT been answered** —
ask before starting (§7).

## 3. Do this next

1. Read `docs/superpowers/specs/2026-08-14-capture-core-await-design.md`, then
   `docs/superpowers/plans/2026-08-14-capture-core-await.md`. The plan argues
   from the spec; where they disagree, the spec wins.
2. Ask the user: subagent-driven (`superpowers:subagent-driven-development`,
   recommended in the plan) or inline (`superpowers:executing-plans`).
3. Start Task 1 (`Pattern`, `src/pattern.rs`). It is self-contained, pure, and
   adds `regex-lite` — a good shakedown of the loop before the PTY tasks.

## 4. Lessons & traps ← the irreplaceable part

**The reported bug was never reproduced, and the redesign does not claim to fix
it.** `docs/mdmost-theme-capture-findings.md` reports a frame showing a dark
screen under a `theme: light` status bar. I reproduced the exact failing script
(mdmost's tour truncated after act 6's `F F F f Enter`, plus a `t` scene) **7
times with zero failures** — 3 on a *deliberately unfixed* binary, 1 at
`settle_ms = 60`. Do not assume it is fixed when the redesign lands; do not
re-run those probes expecting a different answer.

Ruled out, each with evidence — do not re-investigate:

- **A control code `vt100` can't parse** (the user's best hypothesis, and the
  one that fit the evidence best). A parser gap fails deterministically; 7/7
  clean kills it.
- **A different mdmost binary.** The bisect used the build from the
  `mdmost-semantic-selection` worktree; `cmp` says it is **byte-identical** to a
  fresh build of mdmost `main`.
- **mdmost rendering in two stages.** `src/tui/app.rs:1691` `cycle_theme` sets
  theme, notice and re-render synchronously before any draw — the mixed frame
  is not a state mdmost can paint.
- **Machine load.** 5.67 across 128 cores.
- **`cap` exhaustion.** Traced: 254 settles, **all** ended on `idle`, none on
  `cap`.

**The trap that cost me an hour: I applied the fix to `term.rs` and then built
the release binary, so my first four "clean" probe runs used a *patched*
binary.** Always build and stash the baseline binary BEFORE touching source.
`cp $(cargo metadata --format-version 1 | jq -r .target_directory)/release/ansidrama /tmp/.../baseline`.

**The demo runs mdmost inside tmux**, in a split pane — ansidrama parses
*tmux's* output, not mdmost's. Any theory about byte ordering has to account
for tmux as a middleman that re-renders on its own schedule.

**Measured, and the number the spec's motivation rests on:** across the tour's
254 waits the app answers in **0–15ms**, then the recorder waits out the
remaining ~300ms every time — **~76s of dead air** in one recording. That is
what `stable_ms = 40` recovers.

**`react_ms` was a no-op in the reporter's probe for two independent reasons.**
It is satisfied rather than waited out (the disarm bug), *and*
`cap = max(idle*8, 1500, react + idle*2)` is 2800ms for both `react_ms = 500`
and `react_ms = 2000` at `settle_ms = 350` — the ceiling was byte-identical
across their "four times the default" experiment.

**Design insight that took several rounds to reach, don't re-derive it:**
stability alone reintroduces the original bug, because before the app reacts
the pre-input screen has *already* been unchanged for a long time. Hence the
two-phase wait: a `change_ms` grace measured on **grid changes, not PTY bytes**.
Grid-based is what makes it correct — leftover bytes, tmux status ticks and
cursor moves all produce traffic without changing the screen.

**Frame taxonomy (the user's idea, and it unified the design):** the first
stable state after an input is that input's result and takes the *authored*
duration; anything after it before the next input is the app moving on its own
and takes its *measured* duration. This auto-detects self-paced animation and
retired a per-scene `realtime` declaration that had been proposed.

**Two probe sockets are mine and now dead** (`mdmost-claudeprobe`,
`mdmost-cpfast`). About ten others under `/tmp/tmux-1003/` are the user's from
their bisect — **do not kill them**, and do not touch `default`.

## 5. Don'ts & constraints

- **Cap cargo at 4 cores** — `CARGO_BUILD_JOBS=4` on every invocation. Shared
  128-core machine, not ours alone.
- Target dir is `/home/oetiker/scratch/cargo-target`, **not** `./target/`.
- **Breaking config changes are explicitly approved** — ansidrama is early, no
  compatibility layer. `deny_unknown_fields` means old configs carrying
  `settle_ms` fail loudly at parse. That is the migration, not a regression.
- **Settled, do not relitigate:** approach 1 (sampler thread + state log) over
  polling; `regex-lite` over `regex`; `await` composes with stability rather
  than replacing it; region-exclusion and changed-area stability masks are
  deferred to a later cycle; auto-slower-re-record is out (declared `await`
  makes it unnecessary).
- **Additive first, deletions last.** The plan's task order exists so the tree
  compiles and tests pass after *every* task. `settle()` and its config keys die
  only in Task 7.
- Ask before `ssh`; ask before destructive commands.

## 6. Where the detail lives

- Change history: `git log 98a79e3..HEAD`
- Spec: `docs/superpowers/specs/2026-08-14-capture-core-await-design.md`
- Plan: `docs/superpowers/plans/2026-08-14-capture-core-await.md` (9 tasks)
- The original bug report: `docs/mdmost-theme-capture-findings.md` (untracked)
- `src/term.rs:207` — `settle()`, the code being replaced
- `src/record.rs:110` — `capture()`, one frame per input, the model being replaced
- mdmost demo: `~/checkouts/mdmost/demo/mdmost.toml`; the walk is at lines
  400–417, and the removed theme beat is documented at line ~420

## 7. Open questions / pending decisions

- **Execution mode not chosen** — subagent-driven or inline. Ask first.
- **The mdmost theme frame is undiagnosed.** The tool to settle it now exists:
  `ANSIDRAMA_DUMP_PTY=<path>` captures the byte stream, so the next time it
  fires the bytes can be replayed through the parser offline. Ask the user to
  set it when re-recording the tour.
- **The disarm fix adds ~200ms per input** (`react` became an unconditional
  floor). Reverting is one line in `98a79e3`; `settle()` dies in Task 7 anyway.
  Flagged to the user; not objected to, but not explicitly approved either.
- **`change_ms = 150` is the one number in the spec derived from nothing.**
  Worth revisiting once the sampler exists and real latencies are visible.

## 8. Staleness watch

- **Integration state must be re-derived, never inherited.** Whether this
  branch is merged, pushed or superseded is not knowable from this file:
  `git merge-base --is-ancestor HEAD main`, `git log --oneline HEAD..main`,
  `git branch -a --contains HEAD`. If this branch is merged, stop reading and
  go to the successor's handoff.
- **Sibling worktrees / other workstreams may exist that this file cannot
  name** — anything started after the handoff commit is invisible here.
- The plan names exact line numbers (`src/term.rs:207`, `src/record.rs:110`).
  Any commit after `98a79e3` may have moved them — re-grep, don't trust.
- `regex-lite` was verified at 0.1.9 on 2026-08-14. Re-check the version before
  `cargo add`.
- The 7-clean-runs result is from 2026-08-13/14 against mdmost `56da66b`. If
  mdmost has moved, it says nothing about the new HEAD.
