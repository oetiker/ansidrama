# A theme switch that records as the old screen — findings from mdmost's demo

Date: 2026-08-13
Against: ansidrama **0.2.0** (`2e5195f`), recording `mdmost`'s demo
Reporter: the mdmost side of the tour (`~/checkouts/mdmost/demo/mdmost.toml`)

## The symptom

In one specific recording, the frame after a `t` keypress shows the **old (dark) screen
carrying the new status bar** — `theme: light` written across the bottom of a screen that
is still wearing the dark theme. It holds for the whole scene. This is the exact symptom
0.2.0's changelog claims to have fixed ("a status bar naming a theme the screen was not
wearing"), and **0.2.0 does not fix this instance of it.**

## It is not the capture race `react_ms` addresses

`react_ms = 2000`, four times the default, changes the frame not at all. That is the
evidence for this claim rather than an opinion about it: if the capture were merely early,
a two-second grace would have caught it.

## It is not the application

`mdmost` repaints correctly every time under the identical key sequence in a live pane:

```sh
tmux -L probe -f demo/tmux.conf new-session -x 100 -y 30 \
  "mdmost --mouse --config demo/config.toml demo/tour.md"
# then: G, F, F, F, f, Enter, t   (0.4 s apart)
tmux -L probe capture-pane -p -e | grep -o '48;2;[0-9;]*' | sort -u
# → 48;2;253;252;249   the light theme's background, correctly applied
```

Only the recording disagrees.

## Bisect — the trigger is a keyboard walk

Method: truncate `mdmost/demo/mdmost.toml` at a line, append a `t` scene and an empty
scene, record with `--dump-png`, and look at the frame the run log attributes to the `t`
scene. (Note the log **counts** frames — `scene 49 → 833 frames total` means
`frame0832.png`.) One probe is about four minutes.

| Script | Theme frame | Result |
| --- | --- | --- |
| acts 1–5 — two panes, drag, nano, copy, pane close | 807 | light, **correct** |
| acts 1–6 up to `# No mouse from here` — hover, footnote popup, click away | 901 | light, **correct** |
| the same, padded with ten empty scenes | 911 | light, **correct** |
| the same, with a plain `G` in place of the walk | 902 | light, **correct** |
| **the full tour — the walk `F F F f Enter`, then `t`** | **906** | **DARK under `theme: light`** |

**Dead hypotheses**, each killed by a row above — worth not re-running:

- the pane kill and resize in act 5 (a minimal two-pane script that kills one and
  switches theme records correctly)
- the alternate screen left by `nano` in act 4 (same script with nano opened and closed
  first records correctly)
- accumulated frame count — the failing frame is 906, and a *correct* probe reaches 911
- scroll position, or the sheer size of the repaint — `G` moves the entire screen and
  records correctly
- the `react_ms` capture race, as above

The one thing that distinguishes the failing script is that the five inputs before `t`
are a **keyboard walk**: `F`, `F`, `F`, `f` step a *painted* cursor between controls, and
`Enter` follows an anchor, which scrolls. Each redraws; they arrive close together.

## Two candidate mechanisms

**(a) The react window is disarmed by the previous input's output.** In
`src/term.rs:207`, `awaiting_reply` is cleared by *any* byte arriving after the send. If
the walk's last repaint is still draining when `t` is sent, those leftover bytes satisfy
the react condition, `idle` takes over, and a quiet gap before the real theme repaint ends
the wait early. This explains why a larger `react_ms` cannot help: react is not being
waited out, it is being *satisfied* — by output belonging to the previous input. It also
fits every row of the table, since only the failing script has several redraws landing
back to back.

**The objection to (a), which I could not resolve:** a truncated write should show the
*new* body and the *old* status bar, because ratatui writes its diff row-major and the
status bar is the last row. The observed frame is the other way round — old body, new
status bar. That ordering is hard to get from a partially-applied single write, and it is
the reason I am not asserting (a) as the cause.

**(b) The parser applies the repaint but the grid snapshot predates it.** If the capture
reads `grid()` while the reader thread is mid-apply, a screen mixing old body rows with a
new last row is possible without any timing gap at all — and would also be untouched by
`react_ms`. I have no evidence for this beyond the ordering objection to (a).

## Suggested next probes

Both are one truncation and one record, on the mdmost side:

1. **The walk with `Enter` removed** — separates the painted cursor from the anchor jump
   and its scroll.
2. **The walk replaced by five presses of an unbound key** — separates "five keystrokes
   in quick succession" from "the keyboard cursor specifically". If this reproduces, the
   cursor is irrelevant and mechanism (a) gains a lot of weight.

A third, entirely on this side: log the byte offset and timestamp of each PTY read
alongside each capture, then run the failing script once. If the capture timestamp falls
between the first and last byte of the theme repaint, (b) is confirmed and (a) is dead.

## Reproducing without mdmost

The failing script needs a `mdmost` binary and its demo assets. If that is inconvenient,
probe 2 above is the one to try first: any full-screen TUI that redraws on an unbound key
should stand in, as long as several redraws land immediately before the input under test.
