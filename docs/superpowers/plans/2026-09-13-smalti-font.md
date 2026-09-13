# Smalti Pixel Font + Italic Support — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add Smalti 8x16 as an opt-in pixel font that renders with zero anti-aliasing, and make ansidrama render SGR italic instead of silently discarding it.

**Architecture:** `Renderer` grows from two faces to a 2×2 table (bold × italic) selected from a `FontStack` enum, so both font families are the same shape of thing. Smalti's metrics (advance = ½ em, ascent − descent = 1 em) make the existing sub-pixel stretch correction a no-op and put every glyph origin on an integer pixel, which is what produces hard pixels. Two guards keep it that way: user-supplied sizes that are not multiples of 16 are rejected at config load, and free-size text origins are rounded to whole pixels.

**Tech Stack:** Rust, `ab_glyph` 0.2 (outline rasterizer — reads TTF outlines, *not* bitmap strikes), `image` 0.25, `serde`/`toml`.

**Spec:** [`docs/superpowers/specs/2026-09-13-smalti-font-design.md`](../specs/2026-09-13-smalti-font-design.md)

## Global Constraints

- **Shared machine — cap parallelism to 4 cores.** Prefix every cargo invocation with `CARGO_BUILD_JOBS=4`.
- **Cargo target dir is redirected** to `/home/oetiker/scratch/cargo-target`. The binary is *not* under `./target/`. Find it with `cargo metadata --format-version 1 | jq -r .target_directory`.
- **Long cargo calls need `timeout: 600000`** on the Bash call, and you must wait for them in the same turn — never end a turn with a build in flight.
- **English only** for comments, identifiers, and doc comments.
- **Branch:** `feat/smalti-font`, already created, spec already committed.
- **Commit message trailer**, on every commit:
  `Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>`
- **Smalti is 8x16 only.** Valid pixel sizes are whole multiples of **16**, floor 16: 16, 32, 48, 64.
- **`jetbrains` stays the default font.** Existing configs that contain no SGR 3 must render byte-identically.
- **Do not touch the fallback chain.** Symbols Nerd Font then JuliaMono, upright, anti-aliased, fitted per glyph. That seam is an accepted design decision (spec §3.2), not a bug to fix.

---

## File Structure

| File | Responsibility | Change |
|---|---|---|
| `assets/` | bundled font binaries + licences | add 4 Smalti faces, 2 JetBrains italics, 1 licence |
| `src/raster.rs` | font consts, `FontStack`, `Renderer`, all drawing | the bulk of the work |
| `src/grid.rs` | ANSI/SGR → `Cell` grid | `Sgr.italic`, `Cell.italic`, SGR 3/23 |
| `src/config.rs` | TOML schema + validation | `font` key on both configs, `Card.italic`, size validation |
| `src/lib.rs` | encode entry point | pass the font stack, validate sizes |
| `src/record.rs` | record entry point | same, two call sites |
| `src/frame.rs` | frame/card dispatch | pass `card.italic` |
| `src/chrome.rs` | window chrome | snap derived `title_px`, pass `italic: false` |
| `Cargo.toml` | deb/rpm packaging | add the Smalti licence to both asset lists |
| `README.md`, `man/`, `CHANGES.md` | docs | new key, size ladder, behaviour change |

---

### Task 1: Bundle the font files

Six binaries and one licence. The two metric assertions in this task are the load-bearing claims of the entire design — if either fails, stop and re-read spec §2 rather than adjusting the test.

**Files:**
- Create: `assets/Smalti8x16-Regular.ttf`, `assets/Smalti8x16-Bold.ttf`, `assets/Smalti8x16-Italic.ttf`, `assets/Smalti8x16-BoldItalic.ttf`, `assets/Smalti-LICENSE.txt`, `assets/JetBrainsMono-Italic.ttf`, `assets/JetBrainsMono-BoldItalic.ttf`
- Modify: `src/raster.rs:28-32` (font consts), `Cargo.toml:47-49` and `Cargo.toml:60-62` (package asset lists)
- Test: `src/raster.rs` `mod tests`

**Interfaces:**
- Consumes: nothing.
- Produces: the consts `FONT_JB_REGULAR`, `FONT_JB_BOLD`, `FONT_JB_ITALIC`, `FONT_JB_BOLD_ITALIC`, `FONT_SMALTI_REGULAR`, `FONT_SMALTI_BOLD`, `FONT_SMALTI_ITALIC`, `FONT_SMALTI_BOLD_ITALIC`, all `&'static [u8]`. `FONT_ICONS` and `FONT_SYMBOLS` keep their existing names.

- [ ] **Step 1: Download the Smalti faces and licence**

```bash
cd /home/oetiker/checkouts/ansidrama
B=https://github.com/oetiker/smalti/releases/download/v0.2.0
for f in Regular Bold Italic BoldItalic; do
  curl -fsSL -o "assets/Smalti8x16-$f.ttf" "$B/Smalti8x16-$f.ttf"
done
curl -fsSL -o assets/Smalti-LICENSE.txt \
  https://raw.githubusercontent.com/oetiker/smalti/v0.2.0/LICENSE.tamzen
ls -l assets/Smalti*
```

Expected: four `.ttf` files of roughly 80–88 KB each, and a short licence text file. If any download produces an HTML error page instead, `curl -f` will have failed loudly — do not proceed with a truncated font.

- [ ] **Step 2: Download the matching JetBrains Mono italics**

The bundled upright faces are version **2.304**. The italics must come from the same release or the metrics will not match.

```bash
cd /scratch/oetiker/claude-tmp
curl -fsSL -o jbm.zip \
  https://github.com/JetBrains/JetBrainsMono/releases/download/v2.304/JetBrainsMono-2.304.zip
unzip -o -q jbm.zip -d jbm
cp jbm/fonts/ttf/JetBrainsMono-Italic.ttf     /home/oetiker/checkouts/ansidrama/assets/
cp jbm/fonts/ttf/JetBrainsMono-BoldItalic.ttf /home/oetiker/checkouts/ansidrama/assets/
ls -l /home/oetiker/checkouts/ansidrama/assets/JetBrainsMono-*Italic.ttf
```

Expected: two files of roughly 270 KB each.

- [ ] **Step 3: Rename the existing consts and add the new ones**

In `src/raster.rs`, replace the four `const FONT_*` lines (currently at lines 28–32) with the block below. The two JetBrains consts are **renamed** (`FONT_REGULAR` → `FONT_JB_REGULAR`, `FONT_BOLD` → `FONT_JB_BOLD`) so that eight font consts read consistently; `Renderer::new` references them and will need the same rename in step 4.

```rust
const FONT_JB_REGULAR: &[u8] = include_bytes!("../assets/JetBrainsMono-Regular.ttf");
const FONT_JB_BOLD: &[u8] = include_bytes!("../assets/JetBrainsMono-Bold.ttf");
const FONT_JB_ITALIC: &[u8] = include_bytes!("../assets/JetBrainsMono-Italic.ttf");
const FONT_JB_BOLD_ITALIC: &[u8] = include_bytes!("../assets/JetBrainsMono-BoldItalic.ttf");

// Smalti is a pixel font delivered as outlines: every glyph is axis-aligned
// rectangles on a grid where one pixel is exactly 64 font units. Its advance is
// exactly half the em and its ascent-descent span is exactly one em, so at any
// whole multiple of 16px the cell math below needs no stretch correction and
// every glyph origin lands on an integer pixel — coverage comes back 0 or 1.
const FONT_SMALTI_REGULAR: &[u8] = include_bytes!("../assets/Smalti8x16-Regular.ttf");
const FONT_SMALTI_BOLD: &[u8] = include_bytes!("../assets/Smalti8x16-Bold.ttf");
const FONT_SMALTI_ITALIC: &[u8] = include_bytes!("../assets/Smalti8x16-Italic.ttf");
const FONT_SMALTI_BOLD_ITALIC: &[u8] = include_bytes!("../assets/Smalti8x16-BoldItalic.ttf");

const FONT_ICONS: &[u8] = include_bytes!("../assets/SymbolsNerdFontMono-Regular.ttf");
const FONT_SYMBOLS: &[u8] = include_bytes!("../assets/JuliaMono-Regular.ttf");
```

- [ ] **Step 4: Fix the two references to the renamed consts**

In `Renderer::new` (`src/raster.rs:60-61`):

```rust
        let regular = FontRef::try_from_slice(FONT_JB_REGULAR).expect("regular font parses");
        let bold = FontRef::try_from_slice(FONT_JB_BOLD).expect("bold font parses");
```

- [ ] **Step 5: Write the failing metric tests**

Add to `mod tests` in `src/raster.rs`:

```rust
    /// The whole pixel-font design rests on these two ratios. Smalti's advance is
    /// exactly half the em and its ascent-descent span is exactly one em, which is
    /// what makes `Renderer::new`'s stretch correction a no-op and puts glyph
    /// origins on integer pixels. If this fails, the font changed — do not adjust
    /// the test, re-read the design.
    #[test]
    fn smalti_advance_is_half_an_em_and_line_is_one_em() {
        let f = FontRef::try_from_slice(FONT_SMALTI_REGULAR).expect("smalti parses");
        let em = 1024.0_f32; // scale so one em == 1024px, i.e. font units
        let s = f.as_scaled(PxScale::from(em));
        assert_eq!(s.h_advance(f.glyph_id('M')), em / 2.0, "advance must be half an em");
        assert_eq!(s.ascent() - s.descent(), em, "line must be exactly one em");
        assert_eq!(s.line_gap(), 0.0, "line gap must be zero");
    }

    /// Adding italics must not move the cell by a pixel: the cell is derived from
    /// the advance and the ascent-descent span, and all four JetBrains faces are
    /// the same 2.304 release with identical metrics.
    #[test]
    fn jetbrains_italics_have_the_same_metrics_as_the_upright_faces() {
        let up = FontRef::try_from_slice(FONT_JB_REGULAR).expect("regular parses");
        let u = up.as_scaled(PxScale::from(1000.0));
        for (name, bytes) in [
            ("italic", FONT_JB_ITALIC),
            ("bold italic", FONT_JB_BOLD_ITALIC),
        ] {
            let it = FontRef::try_from_slice(bytes).expect("italic parses");
            let i = it.as_scaled(PxScale::from(1000.0));
            assert_eq!(i.h_advance(it.glyph_id('M')), u.h_advance(up.glyph_id('M')), "{name} advance");
            assert_eq!(i.ascent(), u.ascent(), "{name} ascent");
            assert_eq!(i.descent(), u.descent(), "{name} descent");
        }
    }

    /// All four Smalti faces must load, including the two sheared ones.
    #[test]
    fn every_smalti_face_parses() {
        for bytes in [
            FONT_SMALTI_REGULAR,
            FONT_SMALTI_BOLD,
            FONT_SMALTI_ITALIC,
            FONT_SMALTI_BOLD_ITALIC,
        ] {
            let f = FontRef::try_from_slice(bytes).expect("smalti face parses");
            assert_ne!(f.glyph_id('A').0, 0, "face must carry 'A'");
        }
    }
```

- [ ] **Step 6: Run the tests**

Run: `CARGO_BUILD_JOBS=4 cargo test --lib raster::tests 2>&1 | tail -20`
Expected: PASS, three new tests among them. (They pass immediately — the assets are the implementation.)

- [ ] **Step 7: Capture a render baseline, before anything in the renderer changes**

Spec §4 asks for a guard that JetBrains Mono output is unchanged for input
containing no SGR 3. That guard is only worth anything if the baseline is taken
**now**, from code no later task has touched. Task 8 compares against it.

```bash
cd /home/oetiker/checkouts/ansidrama
mkdir -p /scratch/oetiker/claude-tmp/ansidrama-baseline
CARGO_BUILD_JOBS=4 cargo run -- encode demo/readme.toml \
  -o /scratch/oetiker/claude-tmp/ansidrama-baseline/readme.gif
sha256sum /scratch/oetiker/claude-tmp/ansidrama-baseline/readme.gif \
  | tee /scratch/oetiker/claude-tmp/ansidrama-baseline/readme.sha256
```

Record the hash in your report. If `demo/readme.toml` needs files that are not in
the repo, use `demo/hello.toml` instead and say which you used.

Check whether the demo capture contains italic, because that decides what Task 8
should expect:

```bash
grep -lP '\x1b\[[0-9;]*3[;m]' demo/*.ansi 2>/dev/null || echo "no SGR 3 in the demo captures"
```

- [ ] **Step 8: Add the Smalti licence to both package asset lists**

In `Cargo.toml`, after the `JuliaMono-LICENSE.txt` line in the **deb** list (line 49):

```toml
    ["assets/Smalti-LICENSE.txt", "usr/share/doc/ansidrama/Smalti-LICENSE.txt", "644"],
```

and after the `JuliaMono-LICENSE.txt` line in the **rpm** list (line 62):

```toml
    { source = "assets/Smalti-LICENSE.txt", dest = "/usr/share/doc/ansidrama/Smalti-LICENSE.txt", mode = "644", doc = true },
```

- [ ] **Step 9: Verify the whole crate still builds**

Run: `CARGO_BUILD_JOBS=4 cargo build 2>&1 | tail -5`
Expected: success. Binary grows by roughly 900 KB.

- [ ] **Step 10: Commit**

```bash
git add assets/ Cargo.toml src/raster.rs
git commit -m "$(cat <<'EOF'
feat(assets): bundle Smalti 8x16 and the JetBrains Mono italics

Smalti ships as outline TTF, so ab_glyph loads it with no new dependency.
The JetBrains italics are the same 2.304 release as the bundled upright
faces, with identical metrics, so the cell does not move.

Tests assert the two ratios the pixel design depends on: Smalti's advance
is exactly half an em and its ascent-descent span is exactly one em.

Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

### Task 2: Carry italic through the ANSI parser

`apply_sgr` handles bold (`1`/`22`) and reverse (`7`/`27`) and has no case for italic. The flag is discarded before it ever reaches the rasterizer. This task makes it arrive; nothing renders differently yet.

**Files:**
- Modify: `src/grid.rs:13-18` (`Cell`), `src/grid.rs:41-55` (`Sgr` + `reset`), `src/grid.rs:59` (`apply_sgr`), `src/grid.rs:160` (`Cell` construction)
- Modify: every other `Cell { … }` literal in the crate (the compiler lists them)
- Test: `src/grid.rs` `mod tests`

**Interfaces:**
- Consumes: nothing from Task 1.
- Produces: `Cell { ch: char, fg: Rgb, bg: Rgb, bold: bool, italic: bool }` — Task 3 reads `cell.italic`.

- [ ] **Step 1: Write the failing test**

Add to `mod tests` in `src/grid.rs`:

```rust
    /// SGR 3 turns italic on, 23 turns it off, 0 resets it with everything else.
    /// Before this existed, italic text recorded as upright — a fidelity bug for
    /// any capture of Claude Code, bat, or a man page.
    #[test]
    fn sgr_italic_sets_clears_and_resets() {
        let g = parse_grid("\x1b[3mA\x1b[23mB\x1b[3mC\x1b[0mD");
        let flags: Vec<bool> = g[0].iter().take(4).map(|c| c.italic).collect();
        assert_eq!(flags, vec![true, false, true, false]);
    }

    /// Italic and bold are independent attributes.
    #[test]
    fn bold_and_italic_combine() {
        let g = parse_grid("\x1b[1;3mX");
        assert!(g[0][0].bold, "bold");
        assert!(g[0][0].italic, "italic");
    }
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `CARGO_BUILD_JOBS=4 cargo test --lib grid::tests::sgr_italic 2>&1 | tail -20`
Expected: FAIL to compile — `no field 'italic' on type 'Cell'`.

- [ ] **Step 3: Add the field to `Cell`**

`src/grid.rs:13`:

```rust
pub struct Cell {
    pub ch: char,
    pub fg: Rgb,
    pub bg: Rgb,
    pub bold: bool,
    pub italic: bool,
}
```

- [ ] **Step 4: Add the field to `Sgr` and its reset**

`src/grid.rs:41` and `:47`:

```rust
struct Sgr {
    fg: Col,
    bg: Col,
    bold: bool,
    italic: bool,
    reverse: bool,
}

impl Sgr {
    fn reset() -> Self {
        Sgr {
            fg: Col::Default,
            bg: Col::Default,
            bold: false,
            italic: false,
            reverse: false,
        }
    }
}
```

- [ ] **Step 5: Handle SGR 3 and 23**

In `apply_sgr` (`src/grid.rs:59`), next to the existing bold arms:

```rust
            1 => state.bold = true,
            22 => state.bold = false,
            3 => state.italic = true,
            23 => state.italic = false,
```

- [ ] **Step 6: Populate the field where cells are built**

At `src/grid.rs:160`, add `italic: state.italic,` to the `Cell { … }` literal, next to the existing `bold:` line. Match whatever the surrounding code calls the SGR state variable.

- [ ] **Step 7: Fix every other `Cell` literal**

Run: `CARGO_BUILD_JOBS=4 cargo build 2>&1 | grep -A3 "missing field"`

Every remaining site is a test fixture or a synthetic card. Add `italic: false,` to each — these are upright by construction.

- [ ] **Step 8: Run the tests**

Run: `CARGO_BUILD_JOBS=4 cargo test --lib 2>&1 | tail -20`
Expected: PASS, including the two new tests.

- [ ] **Step 9: Commit**

```bash
git add src/
git commit -m "$(cat <<'EOF'
feat(grid): parse SGR 3/23 into a Cell italic flag

apply_sgr handled bold and reverse and dropped italic on the floor, so
italic text was flattened to upright before reaching the rasterizer.
The flag now arrives; nothing renders differently yet.

Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

### Task 3: A font stack and a 2×2 face table

`Renderer` holds `regular` and `bold` and picks between them with `if bold` in four places. It becomes a stack plus four faces. This is the task that makes `font = "smalti"` and italic both actually draw.

**Files:**
- Modify: `src/raster.rs` — add `FontStack`, replace the `regular`/`bold` fields, `Renderer::new`, `text_extents`, `draw_text`, `render_card`, `draw_block_cursor`, `render`
- Modify: `src/chrome.rs:118`, `:133` (two `draw_text` calls), `:299-334` (test constructors)
- Modify: `src/frame.rs:37` (`render_card` call)
- Modify: `src/config.rs:30-52` (`Card` gains `italic`)
- Modify: `src/lib.rs:66`, `src/record.rs:446`, `src/record.rs:542` (constructor calls — pass `FontStack::JetBrainsMono` for now; Task 4 wires the config key)
- Test: `src/raster.rs` `mod tests`

**Interfaces:**
- Consumes: the font consts from Task 1; `Cell.italic` from Task 2.
- Produces:
  - `pub enum FontStack { JetBrainsMono, Smalti }` — `Copy`, `Clone`, `PartialEq`, `Eq`, `Debug`, `Default` (default `JetBrainsMono`), `serde::Deserialize` with `#[serde(rename_all = "lowercase")]`.
  - `Renderer::new(px: f32, stack: FontStack) -> Renderer`
  - `Renderer::draw_text(&self, img, x, baseline, text, px, color, bold: bool, italic: bool)`
  - `Renderer::render_card(&self, w, h, lines, fg, bg, bold: bool, italic: bool, border, title_px, subtitle_px)`
  - `Renderer::is_pixel_exact(&self) -> bool` — true for `Smalti`; Task 5 and Task 6 use it.
  - `Card.italic: bool` (serde default `false`)

- [ ] **Step 1: Write the failing tests**

Add to `mod tests` in `src/raster.rs`:

```rust
    /// The point of the whole exercise: at a valid size, Smalti puts down only
    /// foreground or background pixels. Any intermediate value means a glyph
    /// origin drifted off the integer grid, or the scale stopped being exact.
    /// This one assertion covers the metrics, the scale, and the origin at once.
    #[test]
    fn smalti_renders_with_no_anti_aliasing() {
        let r = Renderer::new(32.0, FontStack::Smalti);
        let fg = (255u8, 255u8, 255u8);
        let bg = (0u8, 0u8, 0u8);
        // ASCII only, and nothing hand-painted: this must exercise the font path.
        let text = "Hello, Smalti! 0123 gjpqy";
        let grid: Vec<Vec<Cell>> = vec![text
            .chars()
            .map(|ch| Cell { ch, fg, bg, bold: false, italic: false })
            .collect()];
        let img = r.render(&grid, text.chars().count() as u32, 1);
        for (x, y, p) in img.enumerate_pixels() {
            let got = (p[0], p[1], p[2]);
            assert!(
                got == fg || got == bg,
                "pixel at ({x},{y}) is {got:?} — neither fg nor bg, so anti-aliasing crept in"
            );
        }
    }

    /// Smalti's cell is exactly half as wide as it is tall, at every valid size.
    #[test]
    fn smalti_cell_is_half_as_wide_as_tall() {
        assert_eq!(Renderer::new(16.0, FontStack::Smalti).cell_size(), (8, 16));
        assert_eq!(Renderer::new(32.0, FontStack::Smalti).cell_size(), (16, 32));
        assert_eq!(Renderer::new(48.0, FontStack::Smalti).cell_size(), (24, 48));
    }

    /// Italic must actually select a different face, not silently fall back to
    /// the upright one.
    #[test]
    fn italic_draws_differently_from_upright() {
        for stack in [FontStack::JetBrainsMono, FontStack::Smalti] {
            let r = Renderer::new(32.0, stack);
            let cell = |italic| Cell {
                ch: 'a',
                fg: (255, 255, 255),
                bg: (0, 0, 0),
                bold: false,
                italic,
            };
            let upright = r.render(&[vec![cell(false)]], 1, 1);
            let slanted = r.render(&[vec![cell(true)]], 1, 1);
            assert_ne!(
                upright.as_raw(),
                slanted.as_raw(),
                "{stack:?}: italic 'a' must not be identical to upright 'a'"
            );
        }
    }

    /// Bold and italic select four distinct faces, not three.
    #[test]
    fn all_four_faces_are_distinct() {
        let r = Renderer::new(32.0, FontStack::Smalti);
        let mut seen: Vec<Vec<u8>> = Vec::new();
        for (bold, italic) in [(false, false), (true, false), (false, true), (true, true)] {
            let c = Cell { ch: 'm', fg: (255, 255, 255), bg: (0, 0, 0), bold, italic };
            let img = r.render(&[vec![c]], 1, 1).as_raw().clone();
            assert!(!seen.contains(&img), "face (bold={bold}, italic={italic}) duplicates another");
            seen.push(img);
        }
    }
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `CARGO_BUILD_JOBS=4 cargo test --lib raster::tests::smalti 2>&1 | tail -20`
Expected: FAIL to compile — `cannot find type 'FontStack'`, and `Renderer::new` takes one argument.

- [ ] **Step 3: Add the `FontStack` enum**

In `src/raster.rs`, after the font consts:

```rust
/// Which family of faces a `Renderer` draws text from.
///
/// The two are not interchangeable in one respect: Smalti is a pixel font, exact
/// only at whole multiples of its 16px design size. `is_pixel_exact` is what the
/// config validator and the free-size text path consult to know that.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Deserialize)]
pub enum FontStack {
    /// JetBrains Mono — an outline font, exact at any size.
    #[default]
    #[serde(rename = "jetbrains")]
    JetBrainsMono,
    /// Smalti 8x16 — a pixel font, exact only at whole multiples of 16px.
    #[serde(rename = "smalti")]
    Smalti,
}

impl FontStack {
    /// The four faces, in `[regular, bold, italic, bold_italic]` order.
    fn faces(self) -> [&'static [u8]; 4] {
        match self {
            FontStack::JetBrainsMono => [
                FONT_JB_REGULAR,
                FONT_JB_BOLD,
                FONT_JB_ITALIC,
                FONT_JB_BOLD_ITALIC,
            ],
            FontStack::Smalti => [
                FONT_SMALTI_REGULAR,
                FONT_SMALTI_BOLD,
                FONT_SMALTI_ITALIC,
                FONT_SMALTI_BOLD_ITALIC,
            ],
        }
    }

    /// One "pixel" of a pixel font, in output pixels — the step every user-supplied
    /// size must be a whole multiple of. `None` for an outline font, which is exact
    /// at any size.
    pub fn pixel_step(self) -> Option<f32> {
        match self {
            FontStack::JetBrainsMono => None,
            FontStack::Smalti => Some(16.0),
        }
    }
}
```

- [ ] **Step 4: Replace the two face fields with a table**

In `struct Renderer` (`src/raster.rs:35-37`), replace `regular` and `bold` with:

```rust
    /// `[regular, bold, italic, bold_italic]` — see `face`.
    faces: [FontRef<'static>; 4],
    stack: FontStack,
```

Add the accessor in `impl Renderer`:

```rust
    /// The face for a given weight and slant.
    fn face(&self, bold: bool, italic: bool) -> &FontRef<'static> {
        &self.faces[(bold as usize) | ((italic as usize) << 1)]
    }

    /// True when this renderer reproduces its font's pixels exactly, which holds
    /// only at whole multiples of the design size. Free-size text must round its
    /// origins to whole pixels when this is true, or sub-pixel positioning puts
    /// the anti-aliasing straight back.
    pub fn is_pixel_exact(&self) -> bool {
        self.stack.pixel_step().is_some()
    }

    /// Round to a whole pixel for a pixel font; identity otherwise, so no existing
    /// outline-font render moves.
    fn snap(&self, v: f32) -> f32 {
        if self.is_pixel_exact() { v.round() } else { v }
    }
```

- [ ] **Step 5: Rewrite `Renderer::new`**

Replace the body of `Renderer::new` (`src/raster.rs:58-88`). The cell arithmetic is unchanged — only where the faces come from changes.

```rust
    pub fn new(px: f32, stack: FontStack) -> Self {
        let px = px.max(6.0);
        let faces: [FontRef<'static>; 4] = stack
            .faces()
            .map(|bytes| FontRef::try_from_slice(bytes).expect("bundled face parses"));
        let scaled = faces[0].as_scaled(PxScale::from(px));
        let adv = scaled.h_advance(faces[0].glyph_id('M')); // monospace: one advance
        let asc = scaled.ascent();
        let line = asc - scaled.descent(); // descent is negative
        let cell_w = adv.round().max(1.0) as u32;
        let cell_h = line.round().max(1.0) as u32; // no line_gap — box-drawing must fill the cell
        let scale = PxScale {
            x: px * cell_w as f32 / adv,
            y: px * cell_h as f32 / line,
        };
        let ascent = asc * cell_h as f32 / line;

        let fallbacks = [FONT_ICONS, FONT_SYMBOLS]
            .iter()
            .map(|bytes| FontRef::try_from_slice(bytes).expect("fallback font parses"))
            .collect();

        Renderer {
            faces,
            stack,
            fallbacks,
            cell_w,
            cell_h,
            scale,
            ascent,
        }
    }
```

For Smalti this correction is a no-op by construction: `adv` is exactly `px/2` and `line` is exactly `px`, so `scale.x == scale.y == px` and `ascent` is a whole number of pixels. Leave the arithmetic alone — it is what keeps JetBrains Mono's box-drawing tiling.

- [ ] **Step 6: Thread the slant through the drawing methods**

Four edits in `src/raster.rs`:

`text_extents` — replace `self.regular` with `self.faces[0]` throughout. All four faces share metrics, so the extents do not depend on the slant.

`draw_text` — add an `italic: bool` parameter after `bold`, and replace the face pick:

```rust
        let font = self.face(bold, italic);
```

`render_card` — add an `italic: bool` parameter after `bold`, and replace `let font = if bold { &self.bold } else { &self.regular };` with:

```rust
        let font = self.face(bold, italic);
```

`draw_block_cursor` and `render` — replace both occurrences of
`let font = if cell.bold { &self.bold } else { &self.regular };` with:

```rust
            let font = self.face(cell.bold, cell.italic);
```

- [ ] **Step 7: Give cards an italic flag**

In `src/config.rs`, after the `bold` field of `Card` (line 41):

```rust
    /// Draw the card in the slanted face.
    #[serde(default)]
    pub italic: bool,
```

In `src/frame.rs:37`, pass it through:

```rust
    Ok(r.render_card(w, h, &lines, fg, bg, card.bold, card.italic, card.border, tpx, spx))
```

- [ ] **Step 8: Update the remaining call sites**

Run: `CARGO_BUILD_JOBS=4 cargo build 2>&1 | grep -E "^error" -A5 | head -40`

Fix each:
- `src/chrome.rs:118` and `:133` — add `false` for `italic` after the existing bold argument.
- `src/chrome.rs:299,311,320,334` and `src/raster.rs:689,702,731` (test constructors) — `Renderer::new(20.0, FontStack::JetBrainsMono)`.
- `src/lib.rs:66`, `src/record.rs:446`, `src/record.rs:542` — `Renderer::new(cfg.font_px, FontStack::JetBrainsMono)`. Task 4 replaces the hard-coded stack with the config key.

- [ ] **Step 9: Run the full test suite**

Run: `CARGO_BUILD_JOBS=4 cargo test 2>&1 | tail -25`
Expected: PASS, including the four new tests.

If `smalti_renders_with_no_anti_aliasing` fails, do not weaken it. Print the offending pixel's coordinates (the message already does) and check whether `scale` came out non-integral or `ascent` landed on a fraction — that is a metrics problem, not a test problem.

- [ ] **Step 10: Commit**

```bash
git add src/
git commit -m "$(cat <<'EOF'
feat(raster): font stacks and a 2x2 face table

Renderer held two faces and picked between them with `if bold` in four
places. It now holds four, indexed by weight and slant, drawn from a
FontStack — JetBrains Mono or Smalti.

Smalti's advance is exactly half an em and its line exactly one em, so the
existing sub-pixel stretch correction becomes a no-op and every glyph
origin lands on an integer pixel. A test asserts the consequence: at 32px,
Smalti puts down only foreground or background pixels, never a blend.

Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

### Task 4: The `font` config key

Both config structs gain the key, and the three real constructor calls stop hard-coding the stack.

**Files:**
- Modify: `src/config.rs` — `EncodeConfig` (near line 113), `RecordConfig` (near line 197)
- Modify: `src/lib.rs:66`, `src/record.rs:446`, `src/record.rs:542`
- Test: `src/config.rs` `mod tests`

**Interfaces:**
- Consumes: `FontStack` from Task 3.
- Produces: `EncodeConfig.font: FontStack` and `RecordConfig.font: FontStack`, both serde-defaulted to `FontStack::JetBrainsMono`.

- [ ] **Step 1: Write the failing test**

Add to `mod tests` in `src/config.rs`:

```rust
    #[test]
    fn font_defaults_to_jetbrains_and_accepts_smalti() {
        let d: EncodeConfig = toml::from_str(
            "cols = 80\nrows = 24\n[[frame]]\nfile = \"a.ansi\"\n",
        )
        .expect("parses without a font key");
        assert_eq!(d.font, crate::raster::FontStack::JetBrainsMono);

        let s: EncodeConfig = toml::from_str(
            "cols = 80\nrows = 24\nfont = \"smalti\"\nfont_px = 32\n[[frame]]\nfile = \"a.ansi\"\n",
        )
        .expect("parses with font = smalti");
        assert_eq!(s.font, crate::raster::FontStack::Smalti);
    }

    #[test]
    fn an_unknown_font_name_is_rejected() {
        let e = toml::from_str::<EncodeConfig>(
            "cols = 80\nrows = 24\nfont = \"comic-sans\"\n[[frame]]\nfile = \"a.ansi\"\n",
        )
        .expect_err("an unknown font must not parse");
        assert!(
            e.to_string().contains("comic-sans"),
            "error should name the bad value, got: {e}"
        );
    }
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `CARGO_BUILD_JOBS=4 cargo test --lib config::tests::font 2>&1 | tail -20`
Expected: FAIL — `no field 'font' on type 'EncodeConfig'`.

- [ ] **Step 3: Add the key to both config structs**

In `EncodeConfig`, immediately before the `font_px` field (`src/config.rs:114`), and identically in `RecordConfig` (`src/config.rs:198`):

```rust
    /// Which bundled font family to draw with: `"jetbrains"` (default, an outline
    /// font, exact at any size) or `"smalti"` (a pixel font, exact only at whole
    /// multiples of 16px — see `check_pixel_size`).
    #[serde(default)]
    pub font: crate::raster::FontStack,
```

Both structs carry `#[serde(deny_unknown_fields)]`, so a misspelled key is already an error; the `rename_all = "lowercase"` on `FontStack` is what rejects a misspelled *value*.

- [ ] **Step 4: Run the tests**

Run: `CARGO_BUILD_JOBS=4 cargo test --lib config::tests::font 2>&1 | tail -20`
Expected: PASS.

- [ ] **Step 5: Use the key at the three constructor calls**

`src/lib.rs:66`, `src/record.rs:446`, `src/record.rs:542` — replace the hard-coded stack from Task 3:

```rust
    let renderer = Renderer::new(cfg.font_px, cfg.font);
```

- [ ] **Step 6: Run the full suite**

Run: `CARGO_BUILD_JOBS=4 cargo test 2>&1 | tail -20`
Expected: PASS.

- [ ] **Step 7: Commit**

```bash
git add src/
git commit -m "$(cat <<'EOF'
feat(config): a `font` key selecting the bundled font family

`font = "smalti"` on either config picks the pixel font; the default stays
"jetbrains", so every existing config renders exactly as before.

Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

### Task 5: Reject sizes a pixel font cannot draw

Smalti is exact only at whole multiples of 16. The current defaults — `font_px = 18`, `card_font_px = 44`, `card_subtitle_px = 22` — are all invalid for it, which is intended: there is no Smalti size near the existing 11×24 cell, so choosing the font means choosing a new output resolution.

Reject, do not snap. `font_px` decides the output resolution and people size captures to fit a README column; moving it silently would change the image behind the user's back.

**Files:**
- Modify: `src/config.rs` — add `check_pixel_size` and a validator on each config struct
- Modify: `src/lib.rs` (after the config parses), `src/record.rs` (both entry points)
- Test: `src/config.rs` `mod tests`

**Interfaces:**
- Consumes: `FontStack::pixel_step()` from Task 3; `EncodeConfig.font` / `RecordConfig.font` from Task 4.
- Produces: `EncodeConfig::check_font_sizes(&self) -> Result<()>` and `RecordConfig::check_font_sizes(&self) -> Result<()>`.

- [ ] **Step 1: Write the failing test**

Add to `mod tests` in `src/config.rs`:

```rust
    #[test]
    fn smalti_rejects_a_size_that_is_not_a_multiple_of_sixteen() {
        let cfg: EncodeConfig = toml::from_str(
            "cols = 80\nrows = 24\nfont = \"smalti\"\nfont_px = 18\n[[frame]]\nfile = \"a.ansi\"\n",
        )
        .unwrap();
        let e = cfg.check_font_sizes().expect_err("18 is not a multiple of 16");
        let m = e.to_string();
        assert!(m.contains("font_px"), "names the key: {m}");
        assert!(m.contains("16") && m.contains("32"), "offers both neighbours: {m}");
    }

    #[test]
    fn smalti_rejects_a_bad_card_size_and_names_that_key() {
        let cfg: EncodeConfig = toml::from_str(
            "cols = 80\nrows = 24\nfont = \"smalti\"\nfont_px = 32\ncard_font_px = 44\n\
             card_subtitle_px = 16\n[[frame]]\nfile = \"a.ansi\"\n",
        )
        .unwrap();
        let m = cfg.check_font_sizes().expect_err("44 is invalid").to_string();
        assert!(m.contains("card_font_px"), "names the key: {m}");
        assert!(m.contains("32") && m.contains("48"), "offers both neighbours: {m}");
    }

    #[test]
    fn smalti_rejects_a_per_card_override() {
        let cfg: EncodeConfig = toml::from_str(
            "cols = 80\nrows = 24\nfont = \"smalti\"\nfont_px = 32\ncard_font_px = 32\n\
             card_subtitle_px = 16\n[[frame]]\n[frame.card]\nlines = [\"hi\"]\nfont_px = 40\n",
        )
        .unwrap();
        let m = cfg.check_font_sizes().expect_err("40 is invalid").to_string();
        assert!(m.contains("card.font_px"), "names the per-card key: {m}");
    }

    #[test]
    fn smalti_accepts_valid_sizes() {
        let cfg: EncodeConfig = toml::from_str(
            "cols = 80\nrows = 24\nfont = \"smalti\"\nfont_px = 32\ncard_font_px = 48\n\
             card_subtitle_px = 16\n[[frame]]\nfile = \"a.ansi\"\n",
        )
        .unwrap();
        cfg.check_font_sizes().expect("all multiples of 16");
    }

    /// The outline font is exact at any size — validation must not touch it.
    #[test]
    fn jetbrains_accepts_the_existing_defaults() {
        let cfg: EncodeConfig = toml::from_str(
            "cols = 80\nrows = 24\n[[frame]]\nfile = \"a.ansi\"\n",
        )
        .unwrap();
        cfg.check_font_sizes().expect("defaults are fine for an outline font");
    }
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `CARGO_BUILD_JOBS=4 cargo test --lib config::tests::smalti 2>&1 | tail -20`
Expected: FAIL — `no method named 'check_font_sizes'`.

- [ ] **Step 3: Write the size check**

In `src/config.rs`:

```rust
/// Reject a pixel size a pixel font cannot reproduce exactly.
///
/// Smalti draws one font pixel per 16 output pixels, so it is exact at 16, 32, 48…
/// and blurry everywhere between. `key` names the config key in the error, and the
/// message offers the two nearest valid sizes so the fix is one edit. An outline
/// font has no such constraint and always passes.
pub fn check_pixel_size(font: crate::raster::FontStack, key: &str, px: f32) -> Result<()> {
    let Some(step) = font.pixel_step() else {
        return Ok(());
    };
    if px >= step && (px / step).fract() == 0.0 {
        return Ok(());
    }
    let lower = (px / step).floor().max(1.0) * step;
    anyhow::bail!(
        "{key} = {px} is not valid for font = \"smalti\" \
         (must be a multiple of {step}); try {lower} or {}",
        lower + step
    );
}
```

- [ ] **Step 4: Write the per-config validators**

Also in `src/config.rs`. The two are near-identical because the two config types are, but `EncodeConfig` finds cards under `frames` and `RecordConfig` under `scenes` — check the field name on `RecordConfig` and follow it to its `Option<Card>` (there is one at `src/config.rs:344`).

```rust
impl EncodeConfig {
    /// Every user-supplied pixel size must be one this font can draw exactly.
    pub fn check_font_sizes(&self) -> Result<()> {
        check_pixel_size(self.font, "font_px", self.font_px)?;
        check_pixel_size(self.font, "card_font_px", self.card_font_px)?;
        check_pixel_size(self.font, "card_subtitle_px", self.card_subtitle_px)?;
        for f in &self.frames {
            if let Some(c) = &f.card {
                if let Some(px) = c.font_px {
                    check_pixel_size(self.font, "card.font_px", px)?;
                }
                if let Some(px) = c.subtitle_px {
                    check_pixel_size(self.font, "card.subtitle_px", px)?;
                }
            }
        }
        Ok(())
    }
}
```

Write the `RecordConfig` twin the same way, iterating its scenes.

- [ ] **Step 5: Run the tests**

Run: `CARGO_BUILD_JOBS=4 cargo test --lib config::tests 2>&1 | tail -20`
Expected: PASS.

- [ ] **Step 6: Call the validator at every entry point**

In `src/lib.rs`, right after the `EncodeConfig` is parsed (just below line 51, before the empty-frames check):

```rust
    cfg.check_font_sizes()?;
```

Do the same in `src/record.rs` after each `RecordConfig` parse, ahead of the `Renderer::new` calls at lines 446 and 542.

- [ ] **Step 7: Verify the error reads well from the command line**

```bash
cat > /scratch/oetiker/claude-tmp/bad.toml <<'EOF'
cols = 80
rows = 24
font = "smalti"
[[frame]]
file = "nope.ansi"
EOF
CARGO_BUILD_JOBS=4 cargo run -- encode /scratch/oetiker/claude-tmp/bad.toml -o /scratch/oetiker/claude-tmp/out.gif 2>&1 | tail -5
```

Expected: it refuses, naming `font_px`, the default `18`, and both `16` and `32`. Read the sentence — if it does not tell someone what to type next, fix the wording before committing.

- [ ] **Step 8: Commit**

```bash
git add src/
git commit -m "$(cat <<'EOF'
feat(config): reject sizes Smalti cannot draw exactly

Smalti is a pixel font: exact at whole multiples of 16px, blurry between.
Every user-supplied size is checked at config load, and the error names the
key and both neighbouring valid sizes.

Rejected rather than snapped, because font_px decides the output resolution
and captures are sized to fit a README column — moving it silently would
change the image behind the user's back.

Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

### Task 6: Whole-pixel origins for free-size text

`render_card` centres each line with float arithmetic, so `(w − text_width) / 2` lands on a half-pixel about half the time, and `blit_glyph` hands that straight to `with_scale_and_position` (`src/raster.rs:442-444`). A half-pixel origin reintroduces exactly the grey this design exists to avoid. Task 3 added `snap`; this task uses it.

**Files:**
- Modify: `src/raster.rs` — `render_card` (`:338`), and the border geometry inside it
- Test: `src/raster.rs` `mod tests`

**Interfaces:**
- Consumes: `Renderer::snap` and `Renderer::is_pixel_exact` from Task 3.
- Produces: nothing new.

- [ ] **Step 1: Write the failing test**

```rust
    /// A card is centred with float arithmetic, so its text origins land on
    /// half-pixels about half the time — which puts the anti-aliasing straight
    /// back. Same property as the grid test, one path further out.
    #[test]
    fn smalti_cards_render_with_no_anti_aliasing() {
        let r = Renderer::new(32.0, FontStack::Smalti);
        let fg = (255u8, 255u8, 255u8);
        let bg = (0u8, 0u8, 0u8);
        // An odd character count and an odd width both force a fractional centre.
        let lines = vec!["ansidrama".to_string(), "a pixel card".to_string()];
        let img = r.render_card(641, 385, &lines, fg, bg, false, false, true, 48.0, 16.0);
        for (x, y, p) in img.enumerate_pixels() {
            let got = (p[0], p[1], p[2]);
            assert!(
                got == fg || got == bg,
                "card pixel at ({x},{y}) is {got:?} — neither fg nor bg"
            );
        }
    }
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `CARGO_BUILD_JOBS=4 cargo test --lib raster::tests::smalti_cards 2>&1 | tail -20`
Expected: FAIL with a pixel that is neither fg nor bg.

- [ ] **Step 3: Snap the text origins**

In `render_card`, in the per-line loop (`src/raster.rs:399-409`):

```rust
        let block_h: f32 = per_line.iter().map(|m| m.3).sum();
        let mut y = self.snap((h as f32 - block_h) / 2.0);
        for (line, &(px, adv, asc, line_h)) in lines.iter().zip(&per_line) {
            let text_w = line.chars().count() as f32 * adv;
            let mut x = self.snap((w as f32 - text_w) / 2.0);
            let baseline = self.snap(y + asc);
            for ch in line.chars() {
                self.blit_glyph(&mut img, font, ch, x, baseline, PxScale::from(px), fg);
                x += adv;
            }
            y += line_h;
        }
```

`x += adv` stays exact without snapping: for a pixel font `adv` is a whole number of pixels, so every subsequent origin inherits the snapped start. `y` accumulates from a snapped start and each `baseline` is snapped again, which covers a fractional `line_h`.

- [ ] **Step 4: Run the test**

Run: `CARGO_BUILD_JOBS=4 cargo test --lib raster::tests::smalti_cards 2>&1 | tail -20`
Expected: PASS.

If it still fails, the remaining grey is the **border**, not the text: `rect_outline` takes integers already, but check that `inset`, `gap` and `t` (`src/raster.rs:356-358`) do not produce a zero-thickness or off-by-one edge at 48px. The border is drawn with integer rectangles, so it should be exact — if it is not, fix the geometry, not the test.

- [ ] **Step 5: Verify no existing render moved**

`snap` is the identity for an outline font, so this must be a no-op for JetBrains Mono.

Run: `CARGO_BUILD_JOBS=4 cargo test 2>&1 | tail -20`
Expected: PASS — every pre-existing test included.

- [ ] **Step 6: Commit**

```bash
git add src/
git commit -m "$(cat <<'EOF'
fix(raster): round free-size text origins to whole pixels for pixel fonts

render_card centres each line with float arithmetic, so origins landed on
half-pixels about half the time and ab_glyph anti-aliased them — undoing
the whole point of a pixel font. snap() rounds them, and is the identity
for an outline font so no existing render moves.

Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

### Task 7: Snap the chrome title, which ansidrama sizes itself

Window chrome draws its title at `title_px = 0.52 * bar_h` (`src/chrome.rs:78`), where `bar_h = round(1.55 * cell_h)`. At `font_px = 32` that is 26 — not a valid Smalti size. Task 5's rule cannot apply: there is no config key to name in an error and no user to tell, because ansidrama chose the number.

So this one snaps, rounding **down** to the nearest multiple of 16 with a floor of 16. Down rather than to-nearest because 26 rounded up to 32 would make the title taller than the 50-pixel bar containing it.

| `font_px` | `bar_h` | raw `title_px` | snapped |
|---:|---:|---:|---:|
| 16 | 25 | 13.00 | 16 |
| 32 | 50 | 26.00 | 16 |
| 48 | 74 | 38.48 | 32 |
| 64 | 99 | 51.48 | 48 |

**Files:**
- Modify: `src/chrome.rs:44-80` (`Chrome::from_config` and `Chrome::disabled`)
- Test: `src/chrome.rs` `mod tests`

**Interfaces:**
- Consumes: `FontStack::pixel_step()` from Task 3.
- Produces: `Chrome::from_config` gains a `font: FontStack` parameter.

- [ ] **Step 1: Write the failing test**

```rust
    #[test]
    fn chrome_title_snaps_down_to_a_drawable_size_for_a_pixel_font() {
        // cell_h, expected title_px
        for (cell_h, want) in [(16u32, 16.0_f32), (32, 16.0), (48, 32.0), (64, 48.0)] {
            let cfg = ChromeConfig {
                style: ChromeStyle::Mac,
                ..Default::default()
            };
            let c = Chrome::from_config(&cfg, cell_h, (0, 0, 0), FontStack::Smalti).unwrap();
            assert_eq!(c.title_px, want, "cell_h = {cell_h}");
        }
    }

    #[test]
    fn chrome_title_is_untouched_for_an_outline_font() {
        let cfg = ChromeConfig {
            style: ChromeStyle::Mac,
            ..Default::default()
        };
        let c = Chrome::from_config(&cfg, 24, (0, 0, 0), FontStack::JetBrainsMono).unwrap();
        let bar_h = (1.55 * 24.0_f32).round();
        assert_eq!(c.title_px, 0.52 * bar_h);
    }
```

`ChromeConfig` may not derive `Default` — if it does not, build the struct literally with the same fields `Chrome::from_config` reads, or add `#[derive(Default)]` if every field already has a serde default.

`title_px` is a private field; make it `pub(crate)` so the test can read it, or add a `pub(crate) fn title_px(&self) -> f32` accessor. Prefer the accessor.

- [ ] **Step 2: Run the test to verify it fails**

Run: `CARGO_BUILD_JOBS=4 cargo test --lib chrome::tests::chrome_title 2>&1 | tail -20`
Expected: FAIL — `from_config` takes three arguments.

- [ ] **Step 3: Snap the derived size**

In `Chrome::from_config` (`src/chrome.rs:44`), add a `font: crate::raster::FontStack` parameter, and replace the `title_px` line (`:78`):

```rust
            title_px: snap_title_px(0.52 * bar_h as f32, font),
```

Add the helper to `src/chrome.rs`:

```rust
/// The chrome title size is derived from the bar height, not configured, so an
/// invalid value cannot be reported to anyone — it is rounded instead.
///
/// Down, not to-nearest: at cell_h = 32 the raw size is 26, and rounding that up
/// to 32 would make the title taller than the 50-pixel bar around it. The floor of
/// one step means the smallest sizes get a title that fills more of the bar than
/// it does higher up the ladder.
fn snap_title_px(px: f32, font: crate::raster::FontStack) -> f32 {
    match font.pixel_step() {
        None => px,
        Some(step) => ((px / step).floor() * step).max(step),
    }
}
```

- [ ] **Step 4: Update the two call sites**

`src/lib.rs:69` and the matching line in `src/record.rs` — pass `cfg.font`:

```rust
        Some(c) => Chrome::from_config(c, cell_h, (0, 0, 0), cfg.font).context("chrome config")?,
```

- [ ] **Step 5: Run the tests**

Run: `CARGO_BUILD_JOBS=4 cargo test 2>&1 | tail -20`
Expected: PASS.

- [ ] **Step 6: Look at the `font_px = 16` case with your eyes**

The spec flags this row as the weak one: a 16px title inside a 25px bar is tight.

```bash
cat > /scratch/oetiker/claude-tmp/tiny.toml <<'EOF'
cols = 60
rows = 12
font = "smalti"
font_px = 16
card_font_px = 32
card_subtitle_px = 16
[chrome]
style = "mac"
title = "ansidrama"
[[frame]]
[frame.card]
lines = ["Smalti", "8x16, at one pixel per pixel"]
EOF
CARGO_BUILD_JOBS=4 cargo run -- encode /scratch/oetiker/claude-tmp/tiny.toml \
  -o /scratch/oetiker/claude-tmp/tiny.gif
```

Open the GIF. If the title overflows the bar or collides with the traffic-light dots, say so in your report rather than silently changing the rule — the spec asks for a judgement here, not a fix decided alone.

- [ ] **Step 7: Commit**

```bash
git add src/
git commit -m "$(cat <<'EOF'
feat(chrome): snap the derived title size for a pixel font

title_px is 0.52 * bar_h — a number ansidrama computes, not one the user
types, so an invalid value cannot be reported to anyone. It rounds down to
the nearest drawable size instead. Down rather than to-nearest: at
cell_h = 32 the raw 26 would round up to 32 and overflow its 50px bar.

Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

### Task 8: Documentation and a demo

**Files:**
- Modify: `README.md` (the config example near line 67, the key table near line 125)
- Modify: the man page under `man/`
- Modify: `CHANGES.md`
- Create: `demo/smalti.toml`

**Interfaces:**
- Consumes: everything above.
- Produces: nothing code depends on.

- [ ] **Step 1: Add the key to the README example and table**

In the example block around `README.md:67`:

```toml
font         = "jetbrains"  # or "smalti" — a pixel font, sizes must be /16
font_px      = 18         # terminal font (small is fine — it's a dense capture)
```

In the key table around `README.md:125`, after the `font_px` row:

```markdown
| `font` | global | `jetbrains` (default) or `smalti`, a bundled pixel font |
```

- [ ] **Step 2: Add a README section on the pixel font**

Place it near the existing font discussion:

```markdown
### The pixel font

`font = "smalti"` swaps JetBrains Mono for [Smalti](https://github.com/oetiker/smalti)
8x16, a pixel font derived from Tamzen. Glyphs are reproduced exactly — hard edges,
no anti-aliasing, no grey.

That exactness has a price: **every size must be a whole multiple of 16.**

| `font_px` | cell | 80 columns |
|---:|---|---:|
| 16 | 8×16 | 640 px |
| 32 | 16×32 | 1280 px |
| 48 | 24×48 | 1920 px |

`card_font_px`, `card_subtitle_px` and any per-card override follow the same rule.
ansidrama refuses to start on a size it cannot draw, and tells you the two nearest
valid ones. No Smalti size lands near the JetBrains Mono default, so switching fonts
means choosing a new output resolution — that is inherent to a pixel font, not an
oversight.

Smalti carries no box-drawing or block glyphs, which costs nothing: ansidrama paints
those itself so they reach the exact cell edges. It does carry all 256 Braille
patterns, which JetBrains Mono does not. Icons still come from the Nerd Font
fallback and are anti-aliased, so they look softer than the text around them.
```

- [ ] **Step 3: Document italic**

Add near the SGR/capture discussion:

```markdown
Italic (`SGR 3`) is rendered. Before v0.5.0 it was parsed and discarded, so italic
text in a capture came out upright.
```

- [ ] **Step 4: Update the man page**

Add `font` to the key list alongside `font_px`, `card_font_px` and the rest. Grep the man source for `card_subtitle_px` to find every list that needs it.

- [ ] **Step 5: Write the CHANGES entry**

Two entries under a new unreleased heading — the second is a behaviour change, not a feature, and must read as one:

```markdown
## Unreleased

- **New:** `font = "smalti"` selects a bundled 8x16 pixel font, rendered with no
  anti-aliasing. Sizes must be whole multiples of 16; ansidrama rejects others and
  names the nearest valid ones.
- **Changed:** italic (`SGR 3`) is now rendered instead of discarded. Captures
  containing italic text — man pages, `bat`, most TUI help panes, Claude Code —
  will look different from previous releases, because they were previously wrong.
```

- [ ] **Step 6: Add a demo config**

```bash
cat > demo/smalti.toml <<'EOF'
# Smalti, the bundled pixel font. Every size is a multiple of 16.
cols = 80
rows = 24
font = "smalti"
font_px = 32
card_font_px = 48
card_subtitle_px = 16

[[frame]]
hold_cs = 200
[frame.card]
lines = ["ansidrama", "one pixel per pixel"]
EOF
CARGO_BUILD_JOBS=4 cargo run -- encode demo/smalti.toml -o /scratch/oetiker/claude-tmp/smalti.gif
```

Open the GIF and confirm the card reads crisply with no grey fringes.

- [ ] **Step 7: Compare against the Task 1 baseline**

This is spec §4's regression guard. The default font path must be untouched.

```bash
CARGO_BUILD_JOBS=4 cargo run -- encode demo/readme.toml \
  -o /scratch/oetiker/claude-tmp/readme-after.gif
sha256sum /scratch/oetiker/claude-tmp/readme-after.gif
cat /scratch/oetiker/claude-tmp/ansidrama-baseline/readme.sha256
```

Expected: **identical hashes**, if Task 1 step 7 found no SGR 3 in the demo
captures.

If they differ, do not wave it through. Either the capture does contain italic —
in which case dump both to PNG with `--dump-png` and confirm the *only* difference
is that some text is now slanted — or something in Tasks 3–7 leaked into the
outline-font path, which is a bug. Report which of the two it was, with the
evidence.

- [ ] **Step 8: Run the full suite and the linters**

Run: `CARGO_BUILD_JOBS=4 cargo test 2>&1 | tail -20`
Run: `cargo fmt --check && CARGO_BUILD_JOBS=4 cargo clippy -- -D warnings 2>&1 | tail -20`
Expected: all clean. CI has failed on `fmt` in this repo before — do not skip it.

- [ ] **Step 9: Commit**

```bash
git add README.md man/ CHANGES.md demo/smalti.toml
git commit -m "$(cat <<'EOF'
docs: the pixel font, its size rule, and the italic behaviour change

Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Done when

- `cargo test`, `cargo fmt --check` and `cargo clippy -- -D warnings` are all clean.
- `demo/smalti.toml` produces a GIF whose every pixel is foreground or background.
- `demo/readme.toml` produces a GIF identical to the one on `main`, unless its capture contains SGR 3 — in which case the only difference is slanted text.
- `font = "smalti"` with `font_px = 18` refuses to run and says what to type instead.
