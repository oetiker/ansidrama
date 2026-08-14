# Capture regression gate

Compare a recording against the pre-redesign output with the app-driven path
suppressed, so the frame sequence is comparable index-for-index.

1. Record with 0.2.0 (a separate checkout/worktree of `2e5195f`; its config
   still needs `settle_ms` — `git show 2e5195f:demo/hello.toml` gives you
   that config as-is), `--dump-png out-old/`.
2. Record with HEAD and `persist_ms = 3600000` so no state can ever be
   committed by persistence alone — every frame is then input-driven, exactly
   as 0.2.0 produced. `--dump-png out-new/`.
3. Compare: `for f in out-old/*.png; do cmp -s "$f" "out-new/$(basename $f)" || echo "DIFFERS: $f"; done`

Expect no output. Any difference is either a fixed frame or a regression —
inspect it.

**What this does not cover:** the default configuration, and therefore the
app-driven path. Those rules are covered by the `assemble` unit tests. It
also cannot exercise anything that only matters *through* the persist
window — `persist_ms` is pinned high specifically to suppress
persistence-driven commits — which is why the caret fix in Run 1/2 below
needed its own unit tests in `sampler::acc_tests`
(`a_transient_caret_hide_is_dropped_like_a_transient_grid_change` and
`a_caret_hidden_past_persist_is_committed`) rather than being provable by
the gate alone.

## Run log

This gate has caught a real bug once already — see Run 1 below. Keep both
entries: a gate whose history shows it catching something real is worth
running again; one that has only ever reported success is not.

### Run 1 — 2026-08-14, found a bug

**Against `2e5195f` (v0.2.0) vs HEAD at `0324580`
(`worktree-capture-core-await`, pre-fix), subject `demo/hello.toml`.**

Both binaries built `--release` from the same repository (only the config's
timing keys differ, so fonts/chrome/dimensions are identical by
construction):

- 0.2.0: built in a separate worktree
  (`/scratch/oetiker/claude-worktrees/ansidrama-old-2e5195f`, `git worktree
  add ... 2e5195f`), `CARGO_TARGET_DIR=/scratch/oetiker/cargo-target-old-2e5195f`.
- HEAD: built in this worktree, the normal shared
  `CARGO_TARGET_DIR=/home/oetiker/scratch/cargo-target`.

Configs used (both derived from `demo/hello.toml`, `out` and the scenes
byte-for-byte identical — only the timing keys at the top differ):

- old: `git show 2e5195f:demo/hello.toml` as-is (carries `settle_ms = 40`).
- new: current `demo/hello.toml` plus one added line,
  `persist_ms = 3600000`.

Commands:

```sh
ansidrama-0.2.0 record hello-old.toml --dump-png out-old/
ansidrama-head  record hello-new.toml --dump-png out-new-buggy-run1/
```

Frame counts: **59 frames on both sides** (0.2.0's own log: `scene 00 → 1
frames total`, `scene 01 → 58 frames total`, `scene 02 → 59 frames total`;
HEAD's `manifest.tsv` has 59 rows, all `kind = input-driven` or `card`, as
expected with `persist_ms` pinned high — no `app-driven` rows appear).

Comparison (`cmp` over all 59 pairs, matched by `frameNNNN.png`):

```
DIFFERS: frame0007.png
DIFFERS: frame0033.png
DIFFERS: frame0038.png
```

56 of 59 frames are byte-identical. The three that differ are **all and only**
the frames captured immediately after typing a literal space character —
`manifest.tsv` names their `input` ordinals as 6, 32 and 37, which are
exactly the space characters in the scene's typed string (`printf '...255mhi
from ansidrama...'`: index 6 is the space after `printf`, index 32 the space
after `hi`, index 37 the space after `from`).

`compare -metric AE` (ImageMagick) puts each diff at exactly **288 pixels**
(two cell-widths worth, at this config's ~8x18px cells) — i.e. one cursor
cell lit in the old frame, a different (adjacent) cursor cell lit in the new
frame, and nothing else different anywhere in the 536x232 image. Visually:
HEAD's cursor block sits one column to the *left* of where 0.2.0 draws it,
specifically on the space's own cell rather than past it — in every other
frame (including the frames immediately before and after each of these
three) the two builds agree pixel-for-pixel.

Diff images: `frame0007.png`/`0033`/`0038` differences render as small block
artifacts left of the "correct" cursor position; original pairs and
ImageMagick diffs are at:

- `/scratch/oetiker/claude-tmp/claude-1003/-home-oetiker-checkouts-ansidrama/e27b5064-be70-416d-81cd-b237f244fed9/scratchpad/gate/out-old/`
- `/scratch/oetiker/claude-tmp/claude-1003/-home-oetiker-checkouts-ansidrama/e27b5064-be70-416d-81cd-b237f244fed9/scratchpad/gate/out-new-buggy-run1/`
- `/scratch/oetiker/claude-tmp/claude-1003/-home-oetiker-checkouts-ansidrama/e27b5064-be70-416d-81cd-b237f244fed9/scratchpad/gate/diff0007.png`,
  `diff0033.png`, `diff0038.png`

**Root cause, confirmed:** `StateAccumulator::observe` (`src/sampler.rs`)
compared only the grid when deciding whether a new sample is "the same
screen" as the newest known state. A plain space overwrites a blank cell
with a blank cell — grid byte-identical, only the caret moves — so `observe`
called it unchanged and the committed state kept its stale caret. Fixed by
comparing grid *and* caret together, matching the design spec's own wording
for this step (`docs/superpowers/specs/2026-08-14-capture-core-await-design.md`):
*"Convert grid + caret. If equal to the newest known state, do nothing."* —
the implementation plan's Task 3 code compared grids only, a defect that
survived three reviews before this gate caught it.
Regression guard: `sampler::acc_tests::a_caret_only_change_is_a_new_state`.
As a secondary benefit, phase 1's `moved` check (keyed off the same
`last_change`) now also fires promptly on a caret-only reply instead of
always burning the full `change_ms` grace; guarded by
`sampler::pty_tests::a_caret_only_reply_ends_phase_one_promptly`.

### Run 2 — 2026-08-14, after the fix

Same subject, same configs, same 0.2.0 binary; HEAD rebuilt at the commit
carrying the `observe` fix above.

```sh
ansidrama-0.2.0 record hello-old.toml --dump-png out-old/
ansidrama-head  record hello-new.toml --dump-png out-new-fixed-run2/
```

Frame counts: 59 on both sides again (HEAD's `manifest.tsv`: 59 rows, all
`input-driven`/`card`, no `app-driven`).

Comparison: **59/59 byte-identical. No output from the `cmp` loop.**

Artifacts:

- `/scratch/oetiker/claude-tmp/claude-1003/-home-oetiker-checkouts-ansidrama/e27b5064-be70-416d-81cd-b237f244fed9/scratchpad/gate/out-old/`
- `/scratch/oetiker/claude-tmp/claude-1003/-home-oetiker-checkouts-ansidrama/e27b5064-be70-416d-81cd-b237f244fed9/scratchpad/gate/out-new-buggy-run1/` (Run 1's HEAD output, kept for reference)
- `/scratch/oetiker/claude-tmp/claude-1003/-home-oetiker-checkouts-ansidrama/e27b5064-be70-416d-81cd-b237f244fed9/scratchpad/gate/out-new-fixed-run2/` (Run 2's HEAD output — the clean run)

### Run 3 — 2026-08-14, after the final fix wave

Same subject, same `hello-new.toml`, same Run 1/2 `out-old/` baseline (0.2.0
at `2e5195f`, kept rather than rebuilt). HEAD rebuilt `--release` with the
whole-branch review's fix wave applied — the `max_capture_mb` check on the
animated path, the propagated `manifest.tsv` write error, the `owns`
predicate extraction in `assemble`, saturating `cs()`, and the await-failure
PNG.

Of those, two touch the gate's own path and are the reason it was re-run:
extracting `owns` rewrites the window predicate all three emitting branches
share, and a saturating `cs()` changes the arithmetic every measured frame
goes through.

```sh
ansidrama-head record hello-new.toml --dump-png out-new-fixwave/
```

Frame counts: 59 on both sides. HEAD's `manifest.tsv`: 59 rows, 58
`input-driven` + 1 `card`, no `app-driven` (as expected with `persist_ms`
pinned high).

Comparison: **59/59 byte-identical, 0 differ, 0 missing.**

Artifacts:

- `/scratch/oetiker/claude-tmp/claude-1003/-home-oetiker-checkouts-ansidrama/e27b5064-be70-416d-81cd-b237f244fed9/scratchpad/gate/out-new-fixwave/`
- comparison script: `.../scratchpad/gate/compare-fixwave.sh`
