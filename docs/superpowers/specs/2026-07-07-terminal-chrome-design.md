# Terminal chrome & padding — design

## Problem

Rendered frames run the cell grid right to the pixel edge, which looks cramped,
and there is no way to dress a recording up as a terminal *window*. Two related
features:

- **(a) Terminal frame** — optionally draw window chrome around the screen area.
  Two styles: a macOS-style title bar with traffic-light dots, and a generic
  Linux style with a single close button.
- **(b) Padding** — optional breathing room between the cells and the frame/edge,
  so the text does not touch the border.

## Approach: a post-process "matte" step

`Renderer::render` keeps producing the tight `content_w × content_h` cell image
exactly as today. A new `chrome` module takes that content image and *mattes* it
into the final canvas: terminal-background padding around the cells, a title bar
on top, and rounded corners with transparency outside.

The matte is applied uniformly to **every** frame — captured ANSI frames *and*
synthetic cards — so:

- all frames keep identical outer dimensions (the WebP encoder requires this), and
- the whole drama reads as one consistent window; content (terminal or card)
  swaps inside the same frame.

### Why not bake the offset into `Renderer`?

Baking padding into `frame_size` / `cell_origin` / `render` would entangle the
offset into every cell-drawing path and force cards, the block cursor, the mouse
pointer, and box-drawing to all account for it — more surface, more bugs.

The decisive advantage of matting as a post-process: the mouse pointer in
`record.rs` is stamped on the content image *before* matting, using the existing
content-relative `cell_origin`. It therefore stays correct with **zero** changes
to `Renderer`. Chrome is a cleanly separable, independently testable unit.

## Config: new `[chrome]` table

`frame` is unavailable (the encode `[[frame]]` array already owns it), so the
table is named `[chrome]`. It is added identically to both `EncodeConfig` and
`RecordConfig`.

```toml
[chrome]
style   = "macos"     # "macos" | "linux" | "none"  (default "none")
title   = "hello.sh"  # optional, shown in the title bar
padding = 12          # px inset around the cells, terminal-bg filled (default 0)
bar     = "#2b2b2b"   # optional title-bar color override
text    = "#d0d0d0"   # optional title-text color override
```

- `padding` is meaningful even with `style = "none"` — a plain terminal-bg inset,
  square and opaque (no bar, no rounding).
- A chrome style implies rounded corners + transparency (see per-style detail).
- When the whole `[chrome]` table is absent, behaviour is exactly as today
  (`style = "none"`, `padding = 0`) — no visual change, backwards compatible.
- `bar` / `text` accept the same color syntax as the rest of the config (via
  `color::parse`).

## Layout

With a chrome style set, the final image is:

- width  = `content_w + 2·padding`
- height = `titlebar_h + padding_top + content_h + padding_bottom`
        = `titlebar_h + content_h + 2·padding`

Regions:

- **Title bar** (top, full width, height `titlebar_h`): filled with `bar` color;
  holds the window controls and title text.
- **Content region** (below the bar): filled with the **terminal background**
  (black). The `padding` inset on all four sides of this region is what separates
  the cells from the bar and the outer edges. The tight cell image is blitted at
  `(padding, titlebar_h + padding)`.

With `style = "none"`:

- width  = `content_w + 2·padding`, height = `content_h + 2·padding`
- entire canvas filled with terminal bg; cells at `(padding, padding)`; square,
  opaque, no bar.

All chrome metrics — `titlebar_h`, dot radius, close-glyph size, corner radius,
title font size — scale from the renderer's `cell_h`, so they track `font_px`.
Proposed relations (tunable during implementation):

- `titlebar_h ≈ 1.6 · cell_h`
- `corner_radius ≈ 0.3 · titlebar_h`
- traffic-light dot radius ≈ `0.22 · titlebar_h`, dots spaced ≈ `0.9 · titlebar_h`

## The three styles

- **macos** — three traffic-light dots top-left in fixed colors
  `#ff5f56 / #ffbd2e / #27c93f`; `title` centered in the bar. Rounded top **and**
  bottom corners; pixels outside the rounded rect are transparent.
- **linux** — a single `✕` close glyph top-right; `title` left-aligned in the bar.
  Rounded top corners; transparent outside.
- **none** — no bar; padding only; square; fully opaque.

Corner rounding is implemented as an alpha mask applied after the bar and content
are drawn: pixels outside the rounded-rectangle are set to `alpha = 0`. This is
uniform across cards and terminal frames.

## Module & wiring

- **New `src/chrome.rs`**
  - `struct Chrome` holding the resolved style, title, padding, bar/text colors,
    terminal-bg color, and metrics derived from `cell_h`.
  - `Chrome::from_config(cfg_fields, cell_h, term_bg) -> Result<Chrome>` (parses
    colors once, up front, so bad colors fail fast).
  - `Chrome::matte(&self, content: &RgbaImage) -> RgbaImage` — pure; the whole of
    the feature. Returns `content` unchanged in dimensions only when
    `style == none && padding == 0`.
- **`lib.rs::encode`** — build `Chrome` once from `cfg`; wrap each frame image with
  `matte` before pushing to `frames` (and before `--dump-png`, so dumps show the
  final look).
- **`record::run` / `Recorder`** — build `Chrome` once; wrap in `push` and
  `push_card` after the pointer/caret is stamped on the content image.
- The trailing-duplicate logic in `encode_webp` is unaffected (matte happens
  before frames are handed to the encoder).

## Risks / to confirm early

- **Alpha through lossless WebP.** Confirm the `webp` crate's `AnimEncoder` with
  `config.lossless = 1` preserves per-frame alpha for the rounded transparent
  corners. Check first with a throwaway 2-frame encode + PNG dump before building
  the styles out. If alpha is not preserved, fall back to opaque square corners
  for the chrome styles (functionality (a)+(b) still land; only the rounded
  transparency is dropped) and note it.

## Testing

- `chrome.rs` unit tests (pure, no I/O):
  - `style = "none", padding = p` → output is `content + 2p` in each dimension,
    cells offset by `p`, fully opaque.
  - `style = "macos"` → output height includes `titlebar_h`; a probe pixel in the
    bar equals `bar` color; the four outer-corner pixels are transparent
    (`alpha == 0`); a traffic-light dot pixel is present.
  - `style = "linux"` → close glyph region non-empty top-right; top corners
    transparent.
  - Absent `[chrome]` (defaults) → matte is dimension-preserving and opaque.
- Config parse tests: `[chrome]` round-trips in both `EncodeConfig` and
  `RecordConfig`; unknown keys rejected (`deny_unknown_fields`); bad color errors.
- Manual/visual: extend or add a demo config with `[chrome] style = "macos"` and
  eyeball the WebP + `--dump-png`.

## Out of scope (YAGNI)

- Per-side padding (uniform only).
- Additional chrome styles beyond macos/linux/none.
- Configurable traffic-light colors, corner radius, or bar height (derived
  defaults only; revisit if asked).
- Drop shadows / outer glow.
