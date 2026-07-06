# ansidrama

Turn a terminal session into a crisp, tiny, **animated WebP** — scripted scenes,
silent-movie title cards, deterministic frames. No browser, no ffmpeg, no
asciinema. One optional runtime dependency: `tmux`.

`ansidrama` renders each frame itself: it parses the terminal's ANSI cell grid
and rasterizes it with a bundled monospace font, hand-painting box-drawing and
block glyphs so `─│═▒█…` reach the exact cell edges and tile seamlessly. The
result is a lossless, sharp, loopable WebP that stays small — ideal for a README.

```
┌───────────┐     parse ANSI grid        rasterize (bundled font,      lossless
│ .ansi     │ ──▶ ┌──────────────┐ ──▶   hand-painted box/blocks) ──▶  animated
│ snapshots │     │ Vec<Vec<Cell>>│       ┌──────────┐                  WebP
└───────────┘     └──────────────┘       │ RgbaImage │                (loops)
   or a title card ───────────────────▶  └──────────┘
```

## Two commands

### `encode` — the primitive: frames → WebP

Bring your own frames. Each frame is a captured ANSI snapshot
(`tmux capture-pane -e -p -N > 001.ansi`, or from any harness) **or** a synthetic
title card, held for a duration.

```toml
# demo.toml
cols = 100
rows = 30
out  = "demo.webp"

[[frame]]
card    = { lines = ["eDAPtor", "", "a schema-driven LDAP editor"], fg = "#fef9c3", bg = "#111", bold = true }
hold_cs = 150

[[frame]]
file    = "001.ansi"
hold_cs = 200
```

```sh
ansidrama encode demo.toml            # writes demo.webp
ansidrama encode demo.toml -o out.webp --dump-png frames/
```

### `record` — drive a command in tmux, capture each frame

```toml
# record.toml
launch = "myapp --config demo.toml"
cols   = 100
rows   = 30
out    = "docs/demo/myapp.webp"
env    = { COLORTERM = "truecolor" }
quit_keys = ["C-c"]

[[scene]]
card    = { text = "A quick tour", fg = "#fef9c3" }
hold_cs = 150

[[scene]]
keys    = ["Down", "Down", "Enter"]
hold_cs = 130

[[scene]]
click   = { x = 10, y = 11 }          # friendly mouse — no escape codes
hold_cs = 40

[[scene]]
drag    = { from = [18, 8], to = [44, 20], steps = 4 }
hold_cs = 45
```

```sh
ansidrama record record.toml
```

## Scenes & frames

A **scene** (record) or **frame** (encode) is one held image plus one action:

| Field    | Meaning |
|----------|---------|
| `hold_cs`| how long to hold the frame, in centiseconds (default 100 = 1s) |
| `keys`   | named tmux keys sent in order, e.g. `["F10", "Down", "Enter"]` |
| `text`   | a string typed literally, one character at a time |
| `click`  | `{ x, y, button = "left" }` — press + release |
| `drag`   | `{ from = [x,y], to = [x,y], steps = 4, button = "left" }` |
| `scroll` | `{ x, y, dir = "down", n = 3 }` |
| `card`   | a title card (see below) — no terminal interaction |
| `file`   | (encode only) path to a captured `.ansi` snapshot |

Exactly one action per scene/frame. Coordinates are 1-based terminal columns/rows.
Mouse actions are expanded to SGR (1006) mouse reports under the hood, so you
never write `\x1b[<0;10;11M` by hand — but a raw escape in `keys` still works as
an escape hatch.

## Title cards

Silent-movie intertitles: a solid panel with centered text inside a double-line
frame.

```toml
card = { text = "Browse. Edit. Save.", fg = "white", bg = "black", bold = true, border = true }
# or multi-line:
card = { lines = ["Chapter one", "the directory tree"] }
```

Colours are `#rrggbb`, `#rgb`, or a basic name (`black white red green blue
yellow grey`). `border` (default `true`) draws the frame.

## Install

```sh
cargo install --path .        # or: cargo build --release
```

`record` needs **tmux ≥ 3.2** on `PATH` (for `-e` env passing). `encode` needs
nothing but the binary. The font (JetBrains Mono, OFL) is bundled.

## What it is not

This is a **deterministic slideshow of held frames**, not continuous-motion
capture. Each scene sends its input, waits for the screen to settle, and grabs
one frame. That makes runs reproducible and output tiny/sharp — great for menu
and dialog tours — but it does not record smooth typing or scrolling *motion*.
For that, reach for [VHS](https://github.com/charmbracelet/vhs) (headless
browser + ffmpeg → GIF/MP4). `ansidrama` trades motion for determinism, crispness
and a near-zero toolchain.

## License

MIT (see `LICENSE-MIT`). Bundled JetBrains Mono is under the SIL Open Font
License (see `assets/JetBrainsMono-LICENSE.txt`).
