# Continuous capture and declared scene completion

Date: 2026-08-14
Status: approved design, not yet implemented
Scope: sub-project **B-core + A** of the capture redesign

## Why

`record` decides when an app has finished responding by watching the PTY go
quiet. That decision is undecidable from timing alone, and every bug in the
capture path has been a wrong answer to it:

- **0.2.0's late-first-reply bug** — a quiet PTY reads the same whether the app
  has finished drawing or has not started. Fixed by `react_ms`, a floor on the
  wait for the first byte.
- **The disarm bug** (found 2026-08-13, test `settle_is_not_disarmed_by_the_previous_inputs_output`)
  — `read_loop` clears `awaiting_reply` on *any* byte, so output still draining
  from the *previous* input satisfies the react window. With `react_ms = 2000`,
  `settle` returned after 750ms on the previous key's output. `react_ms` cannot
  help: it is not waited out, it is satisfied.
- **The mdmost theme frame** (`docs/mdmost-theme-capture-findings.md`) — a
  recording that held a dark screen under a `theme: light` status bar. Not
  reproduced locally in 7 runs; still undiagnosed.

The common cause is architectural, not a specific timing constant: **capture
happens once per input, at a moment chosen by a heuristic.** If the heuristic
fires early, the state the app reached is lost for good.

This design removes the guess in two ways. Continuous sampling means a state the
app reached is *never lost* — at worst it appears a beat later than intended, so
being early stops being fatal. And `await` lets the script author *declare* what
completion looks like, replacing the guess with a fact for any scene that names
its result.

## Objectives

1. Record as fast as the app allows; the script dictates playback pacing.
2. Never emit a screen the app did not fully draw.
3. Never silently emit a wrong screen — a declared expectation that fails must
   abort the run.
4. Self-paced app activity (spinners, animations, late repaints) is recorded and
   played back at real elapsed time.

## In scope

- Continuous grid sampling with a state log.
- Stability-based pacing (`change_ms` grace, then `stable_ms` unchanged).
- `await` patterns with optional row scoping, and hard failure on timeout.
- Per-scene `animated` and global `realtime` declarations.
- Assembly as a pure function producing frames with authored or measured
  durations.

## Out of scope (later cycles)

- **B-rest** — typing jitter and human-pace mouse retiming.
- **C** — regex mouse targeting (`click = { find = "..." }`). Independent.
- **D** — latency diagnostics, tuning report, optional retry.
- **Stability masks** — region exclusion and changed-area thresholds. The
  stability predicate is a single function over a config struct, so these are a
  local addition later. Deferred because nothing in the mdmost tour exercises
  them and they would ship untested against a real case.
- Colour/attribute predicates for `await` (text only for now — see Limitations).

## Architecture

```
drive            capture              assemble
record.rs   →    sampler.rs      →    assemble.rs   →   raster.rs → encode.rs
send input       state log            FrameSpecs        render      webp
+ input marks    + wait()             (pure fn)
```

### term.rs

- `settle()` is **removed**.
- `Shared` gains `generation: u64`, incremented on every `parser.process()`.
- `Term` exposes a cloneable `ParserHandle` wrapping the existing
  `Arc<(Mutex<Shared>, Condvar)>`, so the sampler reads the screen without
  owning the child.

The generation counter lets the sampler skip grid conversion when no bytes have
arrived: an idle screen costs one guarded read per tick instead of converting
3000 cells.

### sampler.rs (new)

Owns the sampling thread and the state log. Every `sample_ms`:

1. Read `generation`; if unchanged since last tick, do nothing.
2. Convert grid + caret. If equal to the newest known state, do nothing.
3. Otherwise this is a new state: it becomes **pending**, timestamped.

A pending state is **committed** to the log once it has survived `persist_ms`.
Pending state still drives the stability timer, so flicker resets it correctly.

This applies the assembly filter at capture time and is what bounds memory (see
Resource bounds).

One blocking primitive, which is where **A** lives:

```rust
fn wait(&self, want: Option<&Pattern>, stable: Duration, timeout: Duration)
    -> WaitOutcome
```

`Pattern` is a compiled regex plus an optional row index; negative indexes count
from the bottom (`-1` is the last row). With no row, the pattern is matched
against the whole screen, rows joined by `\n`.

### record.rs

Reduced to driving: send input, call `wait`, push an
`InputMark { t, scene, kind, mouse, authored_cs, hit_cap }`. **No rendering** —
rasterisation moves to assembly, so capture is never blocked by a 728x501
render.

### assemble.rs (new)

Pure function `(state log, input marks, config) -> Vec<FrameSpec>`. No PTY, no
clock, no rendering — this is where the interesting rules live and where they
are unit-tested. `raster.rs` and `encode.rs` are unchanged, and only frames that
survive assembly are ever rasterised.

## Config schema

Breaking changes are acceptable; ansidrama is early in its cycle and there is no
compatibility layer.

### Top level

```toml
sample_ms   = 10     # grid snapshot interval
change_ms   = 150    # grace for the app's first grid change after an input
stable_ms   = 40     # unchanged-for, to call a screen settled (pacing)
persist_ms  = 40     # unchanged-for, to earn a frame in the movie (assembly)
wait_cap_ms = 3000   # bound when a scene has no `await`
await_ms    = 5000   # default `await` timeout
realtime    = false  # whole recording plays at measured time
startup_ms  = 900    # unchanged: floor before the first capture
max_states  = 20000  # backstop; exceeding it is an error
```

**Removed:** `settle_ms`, `react_ms`.

`stable_ms` and `persist_ms` default the same but stay separate: they answer
different questions (when to send the next input vs. what earns a frame). A
two-stage-redraw scene raises the first; a fast-animation scene lowers the
second.

### Per scene

```toml
[[scene]]
keys = ["t"]
await = "theme: light"                        # bare string, whole screen
# await = { find = "theme: light", row = -1 } # row-scoped
# await_ms = 8000                             # per-scene timeout override
# animated = true                             # this screen never holds still
hold_cs = 280
```

Existing scene keys (`hold_cs`, `type_cs`, `move_cs`, `keys`, `text`, `click`,
`drag`, `scroll`, `card`) are unchanged.

## Semantics

### Waiting, per input

```
after input at t_i:
  phase 1 — wait up to `change_ms` for a grid change after t_i
  phase 2 — then wait until the newest state has held for `stable_ms`
            (and matches `await`, if given)
  bounded by wait_cap_ms, or await_ms when `await` is set
```

Phase 1 is essential: stability alone is true the instant an input is sent,
because the pre-input screen has been unchanged for a long time. Without it this
design rebuilds 0.2.0's late-first-reply bug.

Measuring the grace on **grid changes rather than PTY bytes** is what makes it
correct. Leftover bytes from the previous input cannot satisfy it, because they
do not change the grid — which is exactly the disarm bug. Neither can a tmux
status tick or a cursor move. `change_ms` is spent in full only by an input that
genuinely draws nothing.

With `await` set, phase 1 is subsumed: waiting for the match implies waiting for
a change.

`await` applies only to a scene's **final** input. Intermediate keys in
`keys = ["F", "F", "F"]` use stability alone.

Mouse-move frames (pointer animation between positions) send nothing and do not
wait at all; they reuse the current state and take `move_cs`.

**Startup** keeps `startup_ms` as a floor — a short stability window would
otherwise settle during the quiet *before* a slow first paint and seed a blank
screen. After the floor, a normal stability wait (no `await`, bounded by
`wait_cap_ms`) seeds the first state. A first scene may also carry an `await`,
which is the more precise way to say "wait for the prompt".

### Animated scenes

A scene with `animated = true`, or any scene under global `realtime = true`,
skips the stability wait entirely: dwell for the authored time, then proceed.
Everything the app draws meanwhile is captured as app-driven frames.

Under global `realtime`, the script's timing values change meaning: they become
**send delays** rather than playback durations, since typing must still be
injected at human speed even when playback is measured.

Because no `wait` runs for an animated scene, there is no state to carry an
authored duration: **every frame in an animated scene is measured.** The
authored time is spent as the dwell, not as a playback duration.

### Assembly

For each input at `t_i`, take the committed states timestamped in
`[t_i, t_{i+1})`:

| State | Duration |
|---|---|
| the one `wait` returned | **authored** — `type_cs` / `move_cs`, or `hold_cs` if it is the scene's last input |
| any other committed state | **measured** — its real elapsed duration |
| never committed (persisted < `persist_ms`) | dropped |

One uniform rule; the frame taxonomy falls out of it:

- **Input-driven** frames — one per input, authored duration.
- **App-driven** frames — everything else, real elapsed time, giving self-paced
  activity its realism.

Torn mid-repaint screens are excluded because they never persist. A genuine
two-stage redraw yields the first stage at authored time and the second at its
real delay.

The pointer position for a frame comes from the most recent input mark; the
caret is drawn when there is no pointer, as today. The final state of the run
takes the last scene's `hold_cs`.

Card scenes are synthetic and emitted in scene order, unchanged.

## Error handling

- **`await` timeout** — abort the run. Report scene index, pattern, elapsed
  time, and the last screen's text, and write that screen's PNG beside the
  output path so the author can see why it did not match.
- **Pattern compilation** — all patterns compile at config load, before the
  child is spawned. A bad regex fails in milliseconds, not four minutes in.
- **Child exits early** — a legitimate end if the script is finished; a hard
  error naming the remaining scenes if not.
- **`wait_cap_ms` reached without `await`** — not an error. Proceed, and flag
  the mark `hit_cap` for D's report. A deliberately animated screen is normal;
  warning on every run would train the author to ignore the signal.
- **Sampler thread death or poisoned lock** — surfaced as an error, never a
  hang.
- **`max_states` exceeded** — error naming the limit and the config key.

## Resource bounds

At `sample_ms = 10` a never-still app produces 100 states/second at ~40KB per
state (100x30 cells): **4MB/s**, so a two-minute `animated` recording would be
~480MB.

The pending/committed split solves this: only states surviving `persist_ms` are
committed, bounding the log to ~25 states/second (~1MB/s). This costs nothing,
because a state that did not persist was going to be dropped at assembly anyway.
`max_states` backstops the pathological case.

## Testing

**Unit (pure, no PTY):**

- Assembly: transient dropped; two-stage redraw yields two frames with the
  second measured; authored duration lands on the state `wait` returned; final
  state takes `hold_cs`.
- Pattern matching: whole-screen, row-scoped, negative row indexes, out-of-range
  rows.
- Stability: pending/committed transitions, flicker resets the timer.

**PTY integration:**

- `await` returns promptly on match.
- A never-matching `await` errors within its timeout, and the error names the
  pattern.
- An input that draws nothing costs exactly `change_ms`.
- The two `settle` regression tests are **ported, not deleted** — they are the
  two known failure modes and precisely what `change_ms` exists to prevent:
  - `settle_waits_for_a_late_first_reply` (0.2.0's bug)
  - `settle_is_not_disarmed_by_the_previous_inputs_output` (the disarm bug)

**Regression gate:**

Re-record the mdmost tour and diff frame-by-frame against the 0.2.0 output.
Expectation: pixel-identical except where 0.2.0 was wrong. This is why B-core
holds the output shape at one input-driven frame per input — if frame count and
timing shifted at the same time, the diff would be uninterpretable.

## Limitations

- `await` matches **text only**. It cannot assert styling, so
  `await = "theme: light"` confirms the notice is on screen, not that the body
  actually went light. For mdmost these coincide (both are set in one handler
  before any draw), but it is not a general guarantee. A colour/attribute
  predicate is a possible later addition.
- Being early is no longer fatal, but it is not free either: a late repaint
  lands as an app-driven frame after the authored one, so pacing can still slip.
  Detecting that is D's job.
- This design does not explain the mdmost theme frame, which remains
  unreproduced. It changes the failure mode from "wrong screen held" to "right
  screen, one beat late", and makes the scene declarable via `await`.

## Migration

For the mdmost tour: delete `settle_ms`, and optionally add `await` to the theme
scene. Nothing else changes.

## Debug facilities (already in the tree)

- `ANSIDRAMA_DUMP_PTY=<path>` — tees every byte the child writes, so a suspect
  repaint can be replayed through the parser offline.
- `ANSIDRAMA_TRACE=1` — one line per wait: why it ended, how long it took, and
  how much the child said since the input was sent.

Both should survive this redesign; the trace's fields adapt to the new wait
phases.
