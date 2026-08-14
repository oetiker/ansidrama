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
app-driven path. Those rules are covered by the `assemble` unit tests.

## Run log

**2026-08-14, against `2e5195f` (v0.2.0) vs HEAD (`0324580`,
`worktree-capture-core-await`), subject `demo/hello.toml`.**

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
ansidrama-head  record hello-new.toml --dump-png out-new/
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
- `/scratch/oetiker/claude-tmp/claude-1003/-home-oetiker-checkouts-ansidrama/e27b5064-be70-416d-81cd-b237f244fed9/scratchpad/gate/out-new/`
- `/scratch/oetiker/claude-tmp/claude-1003/-home-oetiker-checkouts-ansidrama/e27b5064-be70-416d-81cd-b237f244fed9/scratchpad/gate/diff0007.png`,
  `diff0033.png`, `diff0038.png`

**Read of the finding, not a fix:** a plain space keystroke moves the
terminal's caret but produces no visible change to the grid's cell contents
(a blank cell before and after looks the same). If the new sampler's
stability/commit logic keys off grid-cell equality rather than caret
position, a space-only input could satisfy "nothing changed" and get
captured a sample-tick early, before the caret has advanced — which matches
the symptom exactly (content identical, caret one cell behind) and would
explain why it is specific to space and to no other character in this
script. This is a plausible mechanism, not a confirmed root cause — it needs
someone to trace `StateAccumulator`/`Sampler`'s change detection
(`src/sampler.rs`) against `caret` to confirm before deciding whether it's a
real (very minor, one-column, one-frame-in-many) regression or intentional
behavior worth accepting.

Frame counts, the assembled scene structure, and every non-space keystroke's
result are otherwise identical between 0.2.0 and HEAD.
