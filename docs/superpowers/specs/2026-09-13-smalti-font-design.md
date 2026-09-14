# A pixel font, and the italics that were being thrown away — design

Date: 2026-09-13
Status: approved, ready for a plan

## 1. The problem

Two problems, one change. They are bundled because they touch the same four lines
of face-selection code, and separating them would mean editing `Renderer`'s face
table twice.

**ansidrama renders one look.** Every capture comes out in JetBrains Mono, and the
only knob is `font_px`. That is a fine default and a poor ceiling: the tool draws
terminal recordings, and terminal recordings have an aesthetic tradition —
pixels — that a hinted outline font cannot reach at any size.

[Smalti](https://github.com/oetiker/smalti) is a pixel font with 1005 mapped
codepoints in four faces at two cell sizes, derived from Tamzen. It is delivered
as **outline TTF**, not as a bitmap strike, which matters more than it sounds:
`ab_glyph` reads outlines and cannot read strikes, so Smalti needs no new
dependency and no second render path. It loads exactly the way JetBrains Mono
does.

**Italic is parsed nowhere and rendered never.** `apply_sgr` (`src/grid.rs:59`)
handles SGR `1`/`22` for bold and `7`/`27` for reverse. There is no case for `3`
or `23`. Italic text in a capture is silently flattened to upright before it ever
reaches the rasterizer, and `assets/` carries no italic face to render it with
even if it arrived.

This is a fidelity bug, not a missing nicety. Claude Code — a plausible subject
for an ansidrama recording — uses italic extensively, as do `bat`, man pages
through `less`, and the help panes of most TUIs. Every one of those currently
records wrong.

## 2. Why the metrics make this cheap

The reason this is a small change rather than a rewrite is arithmetic. Read the
release files, not the README:

| | units/em | `M` advance | ascent | descent | line gap |
|---|---:|---:|---:|---:|---:|
| Smalti 8x16 | 1024 | 512 | 768 | −256 | 0 |
| JetBrains Mono 2.304 | 1000 | 600 | 1020 | −300 | 0 |

Smalti's advance is **exactly half the em**, and `ascent − descent` is **exactly
one em**. Now look at what `Renderer::new` already computes (`src/raster.rs:63–73`):
`cell_w` from the `M` advance, `cell_h` from `ascent − descent`, then a sub-pixel
`scale` correction that exists to drag JetBrains Mono's fractional metrics out to
integer cell edges so box-drawing tiles.

With Smalti at `font_px = P`, the advance is `P/2` and the line is `P`. Both are
whole numbers when `P` is even, so **the stretch correction becomes a no-op**. The
hack has nothing to do.

Three consequences follow, and they are the whole design:

- **`font_px` becomes literally the cell height in pixels.** `font_px = 32` means
  32-pixel-tall cells, 16 wide.
- **The baseline lands on an integer pixel row** — the ascent is 12/16 of the cell.
  Smalti's outlines are axis-aligned rectangles on a 64-units-per-pixel grid, so an
  integer origin plus integer rectangles means `outline.draw` returns coverage of
  exactly 0 or 1. No grey. Real pixels.
- **This holds for the oblique faces too.** Smalti shears by moving whole pixels
  per row rather than rotating outlines. Verified by reading `glyf` directly:
  **0 of 50,964 control points in Smalti8x16-Italic sit off the 64-unit grid**, and
  0 of 47,724 in BoldItalic.

The JetBrains italics are the same 2.304 release as the bundled faces and carry
identical metrics (`upm=1000 adv=600 asc=1020 desc=-300 gap=0`), so adding them
**does not move the cell by a pixel**. Only glyphs that were upright become slanted.

## 3. Scope

### 3.1 Config surface

One new global key, added to both config structs (`src/config.rs:113` and `:197`):

```toml
font    = "smalti"    # or "jetbrains" (default)
font_px = 32
```

Not per-frame and not per-card. Mixing two font stacks inside one animation has no
use worth the doubled validation surface.

`"smalti"` means 8x16. If 7x14 is ever added it becomes `"smalti-7x14"` and
`"smalti"` stays an alias for 8x16, so no config breaks.

### 3.2 The font stack

`Renderer` holds `regular` and `bold` and chooses between them with `if bold` in
four places. It becomes a stack plus a 2×2 face table:

```rust
pub enum FontStack { JetBrainsMono, Smalti }

// in Renderer
faces: [FontRef<'static>; 4],   // [regular, bold, italic, bold_italic]
fn face(&self, bold: bool, italic: bool) -> &FontRef<'static>
```

`Renderer::new(px)` becomes `Renderer::new(px, stack)`. Three call sites in `src/`
(`lib.rs:66`, `record.rs:446`, `record.rs:542`) and the test constructors in
`chrome.rs` and `raster.rs`.

The fallback chain is **unchanged**: Symbols Nerd Font then JuliaMono, consulted in
order, fitted per glyph, upright, anti-aliased. An icon inside an italic run does
not slant, and in Smalti mode it is visibly softer than the text around it.

That seam is accepted deliberately. What actually reaches the fallback in a real
capture is icons: box-drawing and block elements are hand-painted upstream
(`src/raster.rs:4`), and Smalti covers Braille and Arrows at 100% where JetBrains
Mono has no Braille at all. Smalti's own coverage of Box Drawing and Block Elements
is 0%, which costs nothing for exactly that reason — those glyphs are never
requested from the text font.

### 3.3 Italic through the parser

`apply_sgr` gains `3 => state.italic = true` and `23 => state.italic = false`;
`Sgr` and `Cell` gain the flag; `0` resets it with the rest. Everything downstream
that picks a face takes it: `render`, `draw_block_cursor`, `draw_text`,
`render_card`.

`Card` gains `italic: bool` alongside its existing `bold`, so title cards can use
the slanted face.

**This changes existing output.** A capture containing SGR 3 renders slanted where
it rendered upright. It belongs in `CHANGES.md` as a behaviour change, not a
feature. It is the one intended break in this design, and it applies to the default
font stack, not only to Smalti.

### 3.4 Keeping the pixels hard

Two rules, both inert for JetBrains Mono.

**Sizes must be whole multiples of 16.** One pixel is `1024/64 = 16` font units, so
exact reproduction requires `px / 16 ∈ ℤ`, with a floor of 16. Validated at config
load for `font_px`, `card_font_px`, `card_subtitle_px`, and the per-card `font_px`
and `subtitle_px` overrides.

The error names the key and both neighbouring valid sizes:

    card_font_px = 44 is not valid for font = "smalti"
    (must be a multiple of 16); try 32 or 48.

Rejection, not snapping. `font_px` determines output resolution and people size
captures to fit a README column; silently moving it would change the image behind
the user's back. Failing at config load costs one edit and is never a surprise.

Note that the current defaults — `font_px = 18`, `card_font_px = 44`,
`card_subtitle_px = 22` — are all invalid for Smalti. Choosing `font = "smalti"`
therefore requires setting the sizes explicitly. That is intended: there is no
Smalti size near the current 11×24 cell, so a user switching fonts is choosing a
new output resolution whether they think about it or not.

**Text origins must be whole pixels.** `render_card` (`src/raster.rs:338`) centres
lines with float arithmetic, so `(w − text_width) / 2` lands on a half-pixel about
half the time, and `blit_glyph` passes that straight to
`with_scale_and_position` (`src/raster.rs:442–444`) — which reintroduces exactly
the grey this design exists to avoid. A `snap()` helper rounds glyph origins and
baselines when the stack is pixel-exact and is the identity otherwise, so no
existing render moves.

The grid path needs no such fix: `cell_origin` (`src/raster.rs:198`) is already
integer, and the ascent is a whole number of pixels at every valid size.

**Derived sizes snap; user-supplied sizes do not.** Window chrome draws its title
at `title_px = 0.52 * bar_h` (`src/chrome.rs:78`), where `bar_h` is itself
`round(1.55 * cell_h)`. At `font_px = 32` that is 26 — not a valid Smalti size. The
rule in §3.4 cannot apply: there is no config key to reject and no user to tell,
because ansidrama chose the number itself.

So in Smalti mode `title_px` is rounded **down** to the nearest multiple of 16, with
a floor of 16:

| `font_px` | `cell_h` | `bar_h` | raw `title_px` | snapped |
|---:|---:|---:|---:|---:|
| 16 | 16 | 25 | 13.0 | 16 (floor) |
| 32 | 32 | 50 | 26.0 | 16 |
| 48 | 48 | 74 | 38.48 | 32 |
| 64 | 64 | 99 | 51.48 | 48 |

Down rather than to-nearest, because rounding 26 up to 32 would put the title
taller than the 50-pixel bar containing it. The floor of 16 means that at
`font_px = 16` the title fills more of the bar than it does at larger sizes; the
implementation should eyeball that case and say so if it looks wrong.

The asymmetry is the point: snapping a value the user typed changes their output
behind their back, which §3.4 rejects. Snapping a value ansidrama computed is just
ansidrama computing it correctly for the font in use.

## 4. Testing

The load-bearing test is a **crispness property**: render known text at Smalti
32px onto a chosen fg/bg, then assert every pixel in the image is exactly fg or
exactly bg, with no intermediate value anywhere. That single assertion proves the
metrics, the scale, the integer origin and the snapping simultaneously, and it
fails loudly if any later change reintroduces sub-pixel positioning. It must use
text drawn from Smalti's own coverage, since a fallback glyph is anti-aliased by
design and would fail it correctly but uninformatively.

Around it:

- Cell size: Smalti at 32 gives 16×32; at 16 gives 8×16.
- Config rejection: `font = "smalti"` with `font_px = 18` errors, and the message
  names 16 and 32.
- SGR round-trip: `3` sets italic, `23` clears it, `0` resets it.
- Italic renders differently from regular — the two images must not be equal.
- Chrome title snapping: at `font_px = 48` the derived `title_px` is 32, and the
  chrome title passes the crispness property too.
- Regression guard: JetBrains Mono output is unchanged for input containing no
  SGR 3.

## 5. Assets, packaging, docs

Four Smalti 8x16 faces (~336 KB) plus `Smalti-LICENSE.txt`, and the two JetBrains
Mono 2.304 italics (~544 KB). `assets/` grows from 6.1 MB to about 7.0 MB, and
since these are `include_bytes!`, so does the binary.

Smalti's licence is Tamzen's, which grants permission to "use, copy, modify, and
distribute it as you see fit". The `(c) 2015 Scott Fial` notice stays in the font's
name table. The licence file joins the `deb` and `rpm` asset lists in `Cargo.toml`
alongside the three already there.

README gains the `font` row in its key table, the valid-size ladder
(16, 32, 48, 64), and a note that italic is now rendered. The man page gains `font`
in its key list. `CHANGES.md` gets two entries: the new font option, and the italic
behaviour change.

## 6. Out of scope

- **Smalti 7x14.** Both strikes have the same 1:2 proportion and differ only in
  detail; shipping one keeps one size ladder to document and test. The naming in
  §3.1 leaves the door open.
- **Italic fallback faces.** Neither Symbols Nerd Font nor JuliaMono has one, and
  synthesising a shear would look worse than the upright glyph.
- **Per-card or per-frame font selection.**
- **Changing the default font.** `jetbrains` stays the default; Smalti is opt-in.
