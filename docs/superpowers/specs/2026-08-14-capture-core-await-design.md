# Continuous capture and declared scene completion

Date: 2026-08-14
Status: approved design, not yet implemented
Scope: sub-project **B-core + A** of the capture redesign

## Why

`record` decides when an app has finished responding by watching the PTY go
quiet, then captures one frame. That decision is undecidable from timing alone:
a quiet PTY reads the same whether the app has finished drawing or has not
started. Worse, **capture happens once per input, at a moment chosen by that
heuristic** — so when it fires early, the state the app reached is lost for
good, not merely mistimed.

This design changes three things that follow from that:

1. **Recording gets much faster.** Today every input costs a full `settle_ms`
   window regardless of how fast the app answered. Instrumenting the mdmost tour
   (`ANSIDRAMA_TRACE=1`, 254 waits) showed the app answering in **0–15ms** and
   the recorder then waiting out the remaining ~300ms every time — about **76
   seconds of dead air** in one recording. Stability-based pacing at 40ms cuts
   that to roughly 10 seconds.
2. **Being early stops being fatal.** Continuous sampling means a state the app
   reached is never lost; at worst it appears a beat later than intended.
3. **Completion can be declared instead of guessed.** `await` lets the script
   author say what the finished screen looks like, and a declared expectation
   that fails **aborts the run** rather than silently writing a bad frame into
   the output.

Point 3 matters most in practice. Today a wrong frame is silent — the only way
to find one is to dump every PNG and look, which is how the investigation below
proceeded.

### The investigation that exposed this

The architecture's limits surfaced while chasing a reported bad frame in
mdmost's demo (`docs/mdmost-theme-capture-findings.md`): a recording that held a
dark screen under a `theme: light` status bar for 2.8 seconds.

That investigation found a genuine, reproducible defect in the current design —
**the disarm bug** (test `settle_is_not_disarmed_by_the_previous_inputs_output`):
`read_loop` clears `awaiting_reply` on *any* byte, so output still draining from
the *previous* input satisfies the react window that `89e5ba3` added. With
`react_ms = 2000`, `settle` returned after 750ms on the previous key's output.
Raising `react_ms` cannot help, because react is not waited out — it is
satisfied.

**The reported mdmost frame itself was never reproduced** (7 runs, including 3
on an unfixed binary and one at a 5× narrower settle window) and remains
undiagnosed. It is *not* the justification for this design, and this design does
not claim to fix it — see Limitations. It is the reason the capture path got
read closely enough to find the disarm bug and to measure the dead air above.

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

A pending state is **committed** to the log when either:

- it has survived `persist_ms`, **or**
- it is the state a `wait` returned as an input's settled result.

The second condition is load-bearing, not a detail. Without it, a config with
`stable_ms = 40` and `persist_ms = 200` — legitimate, since the two knobs are
documented as independently tunable — lets `wait` return at 40ms on a state that
is then superseded at 100ms and never committed, leaving assembly with **no
frame at all** for that input and nowhere to put its authored duration. With it,
**every input has exactly one input-driven frame** as a structural invariant
rather than as a consequence of the two defaults happening to be equal.

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

### Frame manifest

`record` prints `scene 58 -> 907 frames total`, and that mapping is load-bearing
outside this repo: mdmost's `docs/maintainer-notes.md` bisect procedure depends
on deriving a frame index from the running total, and it is how the frame in the
investigation above was located. App-driven frames break it, because a scene no
longer contributes a predictable count.

So `--dump-png` also writes a manifest (TSV) beside the frames:

```
frame	scene	input	kind		hold_cs
0905	57	4	input-driven	280
0906	58	0	input-driven	280
0907	58	-	app-driven	41
```

This makes the bisect procedure better than it is today — "the input-driven
frame for scene 58" becomes a lookup rather than arithmetic across scenes — and
it carries exactly the data the regression gate needs, so the two share one
mechanism.

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
max_capture_mb = 256 # backstop on accumulated grid memory; exceeding it errors
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

`await` applies only to a scene's **final** input — the last thing actually sent
to the app. Intermediate sends use stability alone. Per action type:

| Action | Final input |
|---|---|
| `keys` / `text` | the last key or character |
| `click` | the **release** (mouse-up) |
| `drag` | the **release**, after the motion steps |
| `scroll` | the last wheel sequence |

The app sees the mouse-up, so it is genuinely the last input; no separate rule
is needed. An app that acts on *press* and ignores release still works, because
phase 2 matches `await` against the **current** screen — already matching, and
stable — rather than requiring the change to occur after the final send. The
only cost in that case is a wasted `change_ms` in phase 1.

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
- **`max_capture_mb` exceeded** — error naming the limit, the elapsed time, and
  the two knobs that resolve it.

## Resource bounds

At `sample_ms = 10` a never-still app produces 100 states/second at ~40KB per
state (100x30 cells): **4MB/s**, so a two-minute `animated` recording would be
~480MB.

The pending/committed split solves this: only states surviving `persist_ms` are
committed, bounding the log to ~25 states/second (~1MB/s). This costs nothing,
because a state that did not persist was going to be dropped at assembly anyway.

`max_capture_mb` backstops the pathological case, and is measured in **bytes of
accumulated grid, not a state count**. A count-based limit expresses the bound
in a unit nobody reasons in and silently changes meaning with grid size — a
200x60 recording is 4x the per-state cost, so the same count quadruples the real
ceiling. Exceeding it is an error naming what actually helps:

```
recording exceeded max_capture_mb = 256 after 4m12s (6400 states)
raise max_capture_mb, or raise persist_ms to commit fewer states
```

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

Re-record the mdmost tour with `persist_ms` pinned high enough that no
app-driven frame can form, so the output shape is identical to 0.2.0's by
construction, and diff frame-by-frame. Expectation: **pixel-identical**, except
where 0.2.0 was wrong.

A frame-index diff against the *default* configuration would not work. Today's
~300ms dwell absorbs anything the app does within it into a single frame; under
the new design a change at +50ms becomes a committed state and therefore an
extra frame. Inserted frames desynchronise every subsequent index, so the diff
becomes unreadable exactly where it matters. Pinning `persist_ms` removes that
variable and makes the comparison exact.

**What this gate does not cover:** it passes without exercising the default
configuration, so the app-driven path gets no end-to-end coverage. Its rules are
covered directly by the assembly unit tests above — which is why that is
acceptable, but the limit should be understood rather than assumed away.

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

## Debug facilities

Added during the investigation and **not yet committed** at the time of writing
(`M src/term.rs`). They need committing independently of this work:

- `ANSIDRAMA_DUMP_PTY=<path>` — tees every byte the child writes, so a suspect
  repaint can be replayed through the parser offline.
- `ANSIDRAMA_TRACE=1` — one line per wait: why it ended, how long it took, and
  how much the child said since the input was sent.

Both should survive this redesign; the trace's fields adapt to the new wait
phases.
