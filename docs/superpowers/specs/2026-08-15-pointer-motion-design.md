# A pointer that reports — design

Date: 2026-08-15
Status: approved, ready for a plan

## 1. The problem

`Recorder::move_to` (`src/record.rs:222`) animates the pointer one frame per cell and
**sends nothing**. Its own doc comment says so: "Nothing is sent and nothing is waited
for." The pointer a viewer watches glide across a recording is decoration; the
application never learns it moved.

For most of what a recording shows, that is invisible — `click` sends a real press and
release, and `drag` sends real motion with the button-held bit per cell
(`record.rs:306–310`). The gap is exactly one thing: **bare motion, no button held**,
which is what hover is.

The cost is paid downstream. `mdmost`'s demo wants a frame of a link lit under the
pointer with its URL in the status bar. It cannot ask for one, so it hand-writes the
wire form into a key scene:

```toml
[[scene]]
keys = ["[<35;12;19M"]
```

That works — a raw escape in `keys` reaches the terminal untouched — but a `keys` frame
draws the **text caret**, not the pointer. The published frame shows a lit link with no
arrow resting on it, and the script carries eleven lines of comment explaining why. It
also has to send a second report by hand to un-hover, because a pointer that never
reports leaving leaves a stale URL in the status bar over unrelated content.

A second, smaller gap makes the first unfixable from a script: **there is no bare move
action**. `Action` is `Keys | Text | Click | Drag | Scroll | Card` (`config.rs:398`), and
`move_to` is reachable only as the approach glide inside click, drag and scroll. Even a
pointer that reported could not be told to hover somewhere and stay.

## 2. Scope

Two components, together making a recorded hover real. Neither touches `encode`, the
sampler, or the assembler.

1. `move_to` sends bare motion when the application has asked for it.
2. A new `move` scene action, so a hover is a beat in its own right.

## 3. The mode gate

An application that never requested motion tracking must not receive motion reports.
`vt100` already parses the relevant DECSET modes and exposes the answer:
`Screen::mouse_protocol_mode()` returns `None | Press | PressRelease | ButtonMotion |
AnyMotion` (`screen.rs:617`, set at `1299–1308`).

`Term` grows one accessor over its existing `ParserHandle`, and `move_to` consults it per
glide:

| Observed mode | Behaviour |
| --- | --- |
| `AnyMotion` (`?1003h`) | emit `ESC [ < 35 ; x ; y M` per cell — button 3 plus the 32 motion bit |
| everything else | exactly today: cosmetic frames, no bytes |

The gate is what makes this safe to land unconditionally. No existing script changes
behaviour unless its application asked for `?1003h`, and one that did was already
receiving motion during every drag.

**Verified, not assumed:** `mdmost` sets `?1003h` via crossterm's `EnableMouseCapture`
(`src/tui/term.rs:249`). And tmux — which is the application ansidrama actually drives in
that demo, with `mdmost` inside a pane — propagates it: capturing tmux 3.4's output to its
outer terminal shows `ESC[?1003h` when the focused pane's application wants any-motion.

The propagation is conditional, and scripts should know it: tmux emits `?1003l` again when
focus moves to a pane whose application does not want motion. A glide reports only while
the interested pane is focused.

## 4. What a reporting glide costs

Once `move_to` sends bytes, each step must go through `capture()` as the drag loop already
does, or the repaint the motion provokes is raced rather than captured. The two paths
converge: a glide becomes a drag without the button.

Under `AnyMotion` a forty-cell glide therefore serves forty stability waits where it
previously served none. This is slower in real time; it is **not** slower in the output,
because `move_cs` still authors each step's hold. For a pointer crossing controls that
light and unlight, that wait is precisely what makes the frames truthful. Scripts should
keep glide paths short and deliberate.

## 5. The `move` action

```toml
[[scene]]
move = { x = 12, y = 19 }
hold_cs = 240
```

Glides from the last pointer position, sending motion per cell under the gate. The
**final cell carries the scene's `hold_cs` and is the scene's `want = true` capture**, so
a `move` scene can carry an `await` and assert that the hover actually landed:

```toml
[[scene]]
move = { x = 12, y = 19 }
await = { find = "example.com", row = -1 }
hold_cs = 240
```

Validation follows the rules already in place. `move` joins the `acts` vector at
`config.rs:409`, so the existing exactly-one-action check covers mutual exclusion with no
new code. An `await` that can never be honoured stays an error rather than a silent pass:
where it is detectable at load (on a card, on an animated scene, under `realtime`) it is
rejected there, and otherwise the normal `await_ms` abort names the pattern and dumps the
last screen.

`move` sets `last_mouse`, so a later click glides from where the pointer actually is.

## 6. Failure modes

- **A hover that never lands.** Without `await` this is silent, exactly as a missed click
  is silent today. The `move` action's support for `await` is the remedy, and scripts that
  care should use it.
- **Stale hover.** A pointer that reports arriving must also report leaving, or a URL
  stands in the status bar over content it has nothing to do with. With a real `move` this
  becomes an ordinary beat — move the pointer somewhere harmless — rather than a
  hand-written escape sequence.
- **A glide over a pane that is not focused** reports nothing, per §3. A script whose
  hover depends on focus should establish focus first.

## 7. Testing

- **The gate fires:** an application that sets `?1003h` and echoes its input visibly
  (`cat -v`) receives bare-motion reports per cell — asserted through `await`, so the
  recording aborts if the report never arrives.
- **The gate holds shut:** the same script against an application that sets only `?1000h`
  receives none. This is the case most worth pinning, because the gate's whole job is to
  not fire, and a gate that has quietly stopped gating looks identical to one that works.
- **Config:** `move` is mutually exclusive with the other action keys; a `move` scene
  accepts an `await`.
- The existing drive tests for click, drag and scroll must stay green unchanged — under an
  application in no mouse mode at all, this design is a no-op.

## 8. Out of scope

- Teaching `encode` anything. This is a `record`-side change.
- A `hover = true` per-scene override. It was designed as insurance against tmux not
  propagating `?1003h`; the probe in §3 shows it does, so the override would be a
  configuration knob with no case that needs it.
- Re-staging `mdmost`'s tour on the result. That is a separate project in a separate
  repository, and it depends on this one shipping.
