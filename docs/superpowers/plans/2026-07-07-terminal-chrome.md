# Terminal Chrome & Padding Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Optionally wrap each rendered frame in window chrome (macOS-style dots or a generic Linux close button) plus terminal-bg padding, so recordings look like a window and cells don't touch the edge.

**Architecture:** A new `chrome` module takes the tight cell image `render()` already produces and *mattes* it into a larger canvas: terminal-bg padding around the cells, a title bar on top, rounded corners with transparency outside. Matting is a pure post-process applied uniformly to every frame (captured ANSI *and* cards) in both the `encode` and `record` paths, so all frames keep identical dimensions and the mouse pointer (stamped on the content image before matting) stays correct with zero `Renderer` changes.

**Tech Stack:** Rust, `image` (RgbaImage), `webp` (AnimEncoder/AnimDecoder), `ab_glyph` (via existing `Renderer`), `serde`/`toml`, `anyhow`.

## Global Constraints

- No new crates — reuse `image`, `webp`, `ab_glyph`, `serde`, `toml`, `anyhow`.
- English for all code, comments, and config keys.
- Cap build/test parallelism to 4 cores: prefix every cargo command with `CARGO_BUILD_JOBS=4`.
- All config structs keep `#[serde(deny_unknown_fields)]`.
- Backward compatible: a config with no `[chrome]` table renders exactly as today (no size change, fully opaque).
- `Rgb` is `type Rgb = (u8, u8, u8)` (from `src/color.rs`); `color::parse(&str) -> Result<Rgb, String>`.
- Terminal background is black `(0, 0, 0)` — `render()` fills it; the matte fills padding with the same.
- Chrome metrics derive from the renderer's cell height (`Renderer::cell_size().1`): `bar_h = round(1.55·cell_h)`, `corner = round(0.30·bar_h)`, `dot_d = round(0.42·bar_h)`, `dot_gap = round(0.20·bar_h)`, `inset = round(0.55·bar_h)`, `title_px = 0.52·bar_h`.
- Traffic-light colors are fixed: red `#ff5f56` = `(255,95,86)`, yellow `#ffbd2e` = `(255,189,46)`, green `#27c93f` = `(39,201,63)`.

---

### Task 1: Chrome config schema

**Files:**
- Modify: `src/config.rs` (add `ChromeStyle` enum, `ChromeConfig` struct, `chrome` field on `EncodeConfig` and `RecordConfig`; add tests)

**Interfaces:**
- Produces:
  - `pub enum ChromeStyle { None, Macos, Linux }` — `Deserialize`, `Clone`, `Copy`, `PartialEq`, `Debug`, `Default` (default `None`), serde `rename_all = "lowercase"`.
  - `pub struct ChromeConfig { pub style: ChromeStyle, pub title: String, pub padding: u32, pub bar: String, pub text: String }` — `Deserialize`, `Clone`, `deny_unknown_fields`.
  - `EncodeConfig::chrome: Option<ChromeConfig>` and `RecordConfig::chrome: Option<ChromeConfig>`.

- [ ] **Step 1: Write the failing tests**

Add to the `#[cfg(test)] mod tests` block at the bottom of `src/config.rs`:

```rust
    #[test]
    fn parse_encode_chrome() {
        let cfg: EncodeConfig = toml::from_str(
            r##"
            cols = 80
            rows = 24
            [chrome]
            style = "macos"
            title = "hello.sh"
            padding = 12
            [[frame]]
            file = "0.ansi"
            "##,
        )
        .unwrap();
        let ch = cfg.chrome.unwrap();
        assert_eq!(ch.style, ChromeStyle::Macos);
        assert_eq!(ch.title, "hello.sh");
        assert_eq!(ch.padding, 12);
        assert_eq!(ch.bar, "#2b2b2b"); // default
        assert_eq!(ch.text, "#d0d0d0"); // default
    }

    #[test]
    fn parse_record_chrome_linux() {
        let cfg: RecordConfig = toml::from_str(
            r##"
            launch = "x"
            cols = 1
            rows = 1
            [chrome]
            style = "linux"
            "##,
        )
        .unwrap();
        assert_eq!(cfg.chrome.unwrap().style, ChromeStyle::Linux);
    }

    #[test]
    fn chrome_absent_is_none_option() {
        let cfg: EncodeConfig =
            toml::from_str("cols = 1\nrows = 1\n[[frame]]\ncard = { text = \"x\" }").unwrap();
        assert!(cfg.chrome.is_none());
    }

    #[test]
    fn chrome_rejects_unknown_key() {
        let e: Result<EncodeConfig, _> = toml::from_str(
            r##"
            cols = 1
            rows = 1
            [chrome]
            style = "macos"
            bogus = 1
            [[frame]]
            card = { text = "x" }
            "##,
        );
        assert!(e.is_err());
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `CARGO_BUILD_JOBS=4 cargo test --lib config::tests::parse_encode_chrome`
Expected: FAIL — `ChromeStyle`/`chrome` field do not exist (compile error).

- [ ] **Step 3: Add the schema**

In `src/config.rs`, after the `df_true` helper near the top, add the defaults:

```rust
fn default_chrome_bar() -> String {
    "#2b2b2b".into()
}
fn default_chrome_text() -> String {
    "#d0d0d0".into()
}
```

Then add the enum and struct (place them just above the `// --- encode ---` section comment):

```rust
/// Window-chrome style drawn around the terminal screen area.
#[derive(Deserialize, Clone, Copy, PartialEq, Debug, Default)]
#[serde(rename_all = "lowercase")]
pub enum ChromeStyle {
    /// No title bar — padding only (or nothing).
    #[default]
    None,
    /// macOS: three traffic-light dots top-left, title centered.
    Macos,
    /// Generic Linux: a single close button top-right, title left-aligned.
    Linux,
}

/// Optional window chrome + padding around the cell grid. Absent ⇒ no change.
#[derive(Deserialize, Clone)]
#[serde(deny_unknown_fields)]
pub struct ChromeConfig {
    #[serde(default)]
    pub style: ChromeStyle,
    /// Title shown in the bar (empty ⇒ blank bar).
    #[serde(default)]
    pub title: String,
    /// Terminal-bg-filled inset (px) around the cells (works even with style "none").
    #[serde(default)]
    pub padding: u32,
    /// Title-bar color.
    #[serde(default = "default_chrome_bar")]
    pub bar: String,
    /// Title-text (and Linux close-glyph) color.
    #[serde(default = "default_chrome_text")]
    pub text: String,
}
```

Add the field to `EncodeConfig` (after `pub out: Option<String>,`):

```rust
    #[serde(default)]
    pub chrome: Option<ChromeConfig>,
```

Add the identical field to `RecordConfig` (after its `pub out: Option<String>,`):

```rust
    #[serde(default)]
    pub chrome: Option<ChromeConfig>,
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `CARGO_BUILD_JOBS=4 cargo test --lib config::tests`
Expected: PASS (all config tests, including the four new ones).

- [ ] **Step 5: Commit**

```bash
git add src/config.rs
git commit -m "feat(config): add [chrome] schema (style/title/padding/bar/text)"
```

---

### Task 2: Renderer text helpers

**Files:**
- Modify: `src/raster.rs` (add two public methods to `impl Renderer`; add a `#[cfg(test)] mod tests`)

**Interfaces:**
- Consumes: existing private `Renderer::blit_glyph`, fields `regular`/`bold`.
- Produces (on `Renderer`):
  - `pub fn text_extents(&self, text: &str, px: f32) -> (f32, f32, f32)` → `(width, ascent, line_height)` at `px` (monospace advance × char count).
  - `pub fn draw_text(&self, img: &mut RgbaImage, x: f32, baseline: f32, text: &str, px: f32, color: Rgb, bold: bool)` — blit a string left-to-right from pixel `(x, baseline)`, over existing opaque pixels.

- [ ] **Step 1: Write the failing tests**

Add at the very end of `src/raster.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn draw_text_marks_pixels() {
        let r = Renderer::new(20.0);
        let mut img = RgbaImage::from_pixel(200, 40, Rgba([255, 255, 255, 255]));
        r.draw_text(&mut img, 2.0, 28.0, "Hello", 18.0, (0, 0, 0), false);
        assert!(
            img.pixels().any(|p| p[0] < 200),
            "text should darken some pixels"
        );
    }

    #[test]
    fn text_extents_scale_with_length() {
        let r = Renderer::new(20.0);
        let (w1, _, _) = r.text_extents("M", 18.0);
        let (w2, _, _) = r.text_extents("MM", 18.0);
        assert!(w1 > 0.0);
        assert!((w2 - 2.0 * w1).abs() < 0.01);
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `CARGO_BUILD_JOBS=4 cargo test --lib raster::tests`
Expected: FAIL — `draw_text` / `text_extents` do not exist.

- [ ] **Step 3: Add the methods**

In `src/raster.rs`, inside `impl Renderer`, right after the `cell_size` method, add:

```rust
    /// `(width, ascent, line_height)` of `text` at `px`, in this font's metrics.
    /// Monospace: width is `char_count · advance`.
    pub fn text_extents(&self, text: &str, px: f32) -> (f32, f32, f32) {
        let s = self.regular.as_scaled(PxScale::from(px));
        let adv = s.h_advance(self.regular.glyph_id('M'));
        (
            text.chars().count() as f32 * adv,
            s.ascent(),
            s.ascent() - s.descent(),
        )
    }

    /// Blit `text` starting at pixel `(x, baseline)`, each glyph at `px` scale in
    /// `color`, advancing by the monospace advance. Blends over existing pixels.
    pub fn draw_text(
        &self,
        img: &mut RgbaImage,
        x: f32,
        baseline: f32,
        text: &str,
        px: f32,
        color: Rgb,
        bold: bool,
    ) {
        let font = if bold { &self.bold } else { &self.regular };
        let adv = font.as_scaled(PxScale::from(px)).h_advance(font.glyph_id('M'));
        let mut cx = x;
        for ch in text.chars() {
            self.blit_glyph(img, font, ch, cx, baseline, PxScale::from(px), color);
            cx += adv;
        }
    }
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `CARGO_BUILD_JOBS=4 cargo test --lib raster::tests`
Expected: PASS (both new tests).

- [ ] **Step 5: Commit**

```bash
git add src/raster.rs
git commit -m "feat(raster): public draw_text / text_extents helpers"
```

---

### Task 3: The `chrome` module

**Files:**
- Create: `src/chrome.rs`
- Modify: `src/lib.rs` (add `pub mod chrome;` next to the other `pub mod` lines)

**Interfaces:**
- Consumes: `ChromeConfig`, `ChromeStyle` (Task 1); `Renderer::text_extents`, `Renderer::draw_text`, `Renderer::cell_size` (Task 2); `color::parse`; `color::Rgb`.
- Produces (on `Chrome`):
  - `pub fn disabled() -> Chrome` — no-op chrome.
  - `pub fn from_config(cfg: &ChromeConfig, cell_h: u32, term_bg: Rgb) -> anyhow::Result<Chrome>`.
  - `pub fn is_active(&self) -> bool` — true when matting changes the image.
  - `pub fn matte(&self, r: &Renderer, content: &RgbaImage) -> RgbaImage`.

- [ ] **Step 1: Register the module**

In `src/lib.rs`, add alongside the existing `pub mod` declarations (e.g. right after `pub mod color;`):

```rust
pub mod chrome;
```

- [ ] **Step 2: Write the failing tests**

Create `src/chrome.rs` containing ONLY the test module for now (the rest is added in Step 4):

```rust
//! Wrap a rendered cell image in optional window chrome: terminal-bg padding
//! plus a macOS/Linux-style title bar, with corner rounding. Applied uniformly
//! to every frame (captured or card) so all frames keep identical dimensions.

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{ChromeConfig, ChromeStyle};
    use crate::raster::Renderer;
    use image::{Rgba, RgbaImage};

    fn content(c: (u8, u8, u8)) -> RgbaImage {
        RgbaImage::from_pixel(200, 30, Rgba([c.0, c.1, c.2, 255]))
    }
    fn cfg(style: ChromeStyle, padding: u32) -> ChromeConfig {
        ChromeConfig {
            style,
            title: "hi".into(),
            padding,
            bar: "#2b2b2b".into(),
            text: "#d0d0d0".into(),
        }
    }

    #[test]
    fn none_pads_with_term_bg_opaque() {
        let r = Renderer::new(20.0);
        let cell_h = r.cell_size().1;
        let ch = Chrome::from_config(&cfg(ChromeStyle::None, 6), cell_h, (0, 0, 0)).unwrap();
        let out = ch.matte(&r, &content((10, 20, 30)));
        assert_eq!(out.dimensions(), (200 + 12, 30 + 12));
        assert_eq!(out.get_pixel(0, 0)[3], 255); // opaque
        assert_eq!(&out.get_pixel(0, 0).0[..3], &[0, 0, 0]); // term-bg padding
        assert_eq!(&out.get_pixel(6, 6).0[..3], &[10, 20, 30]); // content at offset
    }

    #[test]
    fn disabled_is_identity_size() {
        let r = Renderer::new(20.0);
        let ch = Chrome::disabled();
        assert!(!ch.is_active());
        let out = ch.matte(&r, &content((1, 2, 3)));
        assert_eq!(out.dimensions(), (200, 30));
    }

    #[test]
    fn macos_has_bar_dots_and_rounded_corners() {
        let r = Renderer::new(20.0);
        let cell_h = r.cell_size().1;
        let ch = Chrome::from_config(&cfg(ChromeStyle::Macos, 8), cell_h, (0, 0, 0)).unwrap();
        let out = ch.matte(&r, &content((5, 5, 5)));
        let bar_h = (1.55 * cell_h as f32).round() as u32;
        assert_eq!(out.height(), bar_h + 30 + 16);
        assert_eq!(out.get_pixel(0, 0)[3], 0); // top-left corner transparent
        assert_eq!(out.get_pixel(0, out.height() - 1)[3], 0); // bottom-left too (macOS)
        assert!(out.pixels().any(|p| p.0 == [0x2b, 0x2b, 0x2b, 255])); // bar color
        assert!(out.pixels().any(|p| p.0 == [255, 95, 86, 255])); // TL red dot
    }

    #[test]
    fn linux_rounds_top_only_and_draws_close() {
        let r = Renderer::new(20.0);
        let cell_h = r.cell_size().1;
        let ch = Chrome::from_config(&cfg(ChromeStyle::Linux, 8), cell_h, (0, 0, 0)).unwrap();
        let out = ch.matte(&r, &content((5, 5, 5)));
        assert_eq!(out.get_pixel(0, 0)[3], 0); // top rounded
        assert_eq!(out.get_pixel(0, out.height() - 1)[3], 255); // bottom square
        let bar_h = (1.55 * cell_h as f32).round() as u32;
        let mut found = false;
        for y in 0..bar_h {
            for x in out.width().saturating_sub(bar_h)..out.width() {
                let p = out.get_pixel(x, y);
                if p.0[..3] != [0x2b, 0x2b, 0x2b] && p[3] == 255 {
                    found = true;
                }
            }
        }
        assert!(found, "close glyph drawn top-right");
    }
}
```

- [ ] **Step 3: Run tests to verify they fail**

Run: `CARGO_BUILD_JOBS=4 cargo test --lib chrome`
Expected: FAIL — `Chrome` is undefined (compile error).

- [ ] **Step 4: Implement the module**

Insert the implementation at the top of `src/chrome.rs`, immediately after the `//!` doc comment and before the `#[cfg(test)]` block:

```rust
use anyhow::{Context, Result};
use image::{Rgba, RgbaImage};

use crate::color::{self, Rgb};
use crate::config::{ChromeConfig, ChromeStyle};
use crate::raster::Renderer;

// Fixed macOS traffic-light colors.
const TL_RED: Rgb = (255, 95, 86);
const TL_YELLOW: Rgb = (255, 189, 46);
const TL_GREEN: Rgb = (39, 201, 63);

/// Resolved chrome: colors parsed once, metrics derived from the cell height.
pub struct Chrome {
    style: ChromeStyle,
    title: String,
    padding: u32,
    bar: Rgb,
    text: Rgb,
    term_bg: Rgb,
    bar_h: u32, // 0 when style == None
    corner: u32,
    dot_d: u32,
    dot_gap: u32,
    inset: u32,
    title_px: f32,
}

impl Chrome {
    /// A no-op chrome (no bar, no padding) — matte returns the input unchanged.
    pub fn disabled() -> Self {
        Chrome {
            style: ChromeStyle::None,
            title: String::new(),
            padding: 0,
            bar: (0, 0, 0),
            text: (0, 0, 0),
            term_bg: (0, 0, 0),
            bar_h: 0,
            corner: 0,
            dot_d: 0,
            dot_gap: 0,
            inset: 0,
            title_px: 0.0,
        }
    }

    /// Build from config. `cell_h` sizes all chrome metrics; `term_bg` fills the
    /// padding (the terminal background).
    pub fn from_config(cfg: &ChromeConfig, cell_h: u32, term_bg: Rgb) -> Result<Self> {
        let bar = color::parse(&cfg.bar)
            .map_err(anyhow::Error::msg)
            .context("chrome `bar`")?;
        let text = color::parse(&cfg.text)
            .map_err(anyhow::Error::msg)
            .context("chrome `text`")?;
        let bar_h = if cfg.style == ChromeStyle::None {
            0
        } else {
            (1.55 * cell_h as f32).round() as u32
        };
        Ok(Chrome {
            style: cfg.style,
            title: cfg.title.clone(),
            padding: cfg.padding,
            bar,
            text,
            term_bg,
            bar_h,
            corner: (0.30 * bar_h as f32).round() as u32,
            dot_d: (0.42 * bar_h as f32).round() as u32,
            dot_gap: (0.20 * bar_h as f32).round() as u32,
            inset: (0.55 * bar_h as f32).round() as u32,
            title_px: 0.52 * bar_h as f32,
        })
    }

    /// True when matting changes the image (else callers can skip it).
    pub fn is_active(&self) -> bool {
        self.bar_h > 0 || self.padding > 0
    }

    /// Wrap `content` in the chrome, returning the final image (larger, and for
    /// chrome styles alpha-rounded at the corners).
    pub fn matte(&self, r: &Renderer, content: &RgbaImage) -> RgbaImage {
        let (cw, ch) = content.dimensions();
        let out_w = cw + 2 * self.padding;
        let out_h = self.bar_h + ch + 2 * self.padding;
        let mut img = RgbaImage::from_pixel(out_w, out_h, rgba(self.term_bg, 255));

        if self.bar_h > 0 {
            fill_rect(&mut img, 0, 0, out_w, self.bar_h, rgba(self.bar, 255));
            self.draw_controls(r, &mut img, out_w);
        }
        overlay(&mut img, content, self.padding, self.bar_h + self.padding);
        self.round(&mut img);
        img
    }

    fn draw_controls(&self, r: &Renderer, img: &mut RgbaImage, out_w: u32) {
        let cy = self.bar_h as f32 / 2.0;
        match self.style {
            ChromeStyle::Macos => {
                let radius = self.dot_d as f32 / 2.0;
                let mut cx = self.inset as f32 + radius;
                for c in [TL_RED, TL_YELLOW, TL_GREEN] {
                    fill_disc(img, cx, cy, radius, c);
                    cx += self.dot_d as f32 + self.dot_gap as f32;
                }
                if !self.title.is_empty() {
                    let (tw, asc, lh) = r.text_extents(&self.title, self.title_px);
                    let x = (out_w as f32 - tw) / 2.0;
                    let baseline = (self.bar_h as f32 - lh) / 2.0 + asc;
                    r.draw_text(img, x, baseline, &self.title, self.title_px, self.text, false);
                }
            }
            ChromeStyle::Linux => {
                if !self.title.is_empty() {
                    let (_tw, asc, lh) = r.text_extents(&self.title, self.title_px);
                    let baseline = (self.bar_h as f32 - lh) / 2.0 + asc;
                    r.draw_text(
                        img,
                        self.inset as f32,
                        baseline,
                        &self.title,
                        self.title_px,
                        self.text,
                        false,
                    );
                }
                // single close "✕", right-aligned
                let half = 0.31 * self.bar_h as f32; // ≈ 0.62·bar_h glyph box
                let cx = out_w as f32 - self.inset as f32 - half;
                let t = (0.06 * self.bar_h as f32).max(1.5);
                stroke_x(img, cx, cy, half, t, self.text);
            }
            ChromeStyle::None => {}
        }
    }

    fn round(&self, img: &mut RgbaImage) {
        if self.corner == 0 {
            return;
        }
        match self.style {
            ChromeStyle::Macos => round_corners(img, self.corner, false),
            ChromeStyle::Linux => round_corners(img, self.corner, true),
            ChromeStyle::None => {}
        }
    }
}

// ---- pixel helpers ---------------------------------------------------------

fn rgba(c: Rgb, a: u8) -> Rgba<u8> {
    Rgba([c.0, c.1, c.2, a])
}

fn fill_rect(img: &mut RgbaImage, x0: u32, y0: u32, x1: u32, y1: u32, px: Rgba<u8>) {
    for y in y0..y1.min(img.height()) {
        for x in x0..x1.min(img.width()) {
            img.put_pixel(x, y, px);
        }
    }
}

/// Copy `src` onto `dst` with its top-left at `(ox, oy)` (opaque copy).
fn overlay(dst: &mut RgbaImage, src: &RgbaImage, ox: u32, oy: u32) {
    for (x, y, p) in src.enumerate_pixels() {
        dst.put_pixel(ox + x, oy + y, *p);
    }
}

/// Blend color `c` at coverage `cov` (0..=1) over the existing pixel, keeping
/// the existing alpha.
fn blend_over(img: &mut RgbaImage, x: u32, y: u32, c: Rgb, cov: f32) {
    let cur = img.get_pixel(x, y);
    let mix = |f: u8, b: u8| (f as f32 * cov + b as f32 * (1.0 - cov)).round().clamp(0.0, 255.0) as u8;
    img.put_pixel(
        x,
        y,
        Rgba([mix(c.0, cur[0]), mix(c.1, cur[1]), mix(c.2, cur[2]), cur[3]]),
    );
}

/// Anti-aliased filled disc, blended over the (opaque) background.
fn fill_disc(img: &mut RgbaImage, cx: f32, cy: f32, r: f32, c: Rgb) {
    let (w, h) = (img.width() as i32, img.height() as i32);
    let x0 = (cx - r - 1.0).floor().max(0.0) as i32;
    let x1 = ((cx + r + 1.0).ceil() as i32).min(w);
    let y0 = (cy - r - 1.0).floor().max(0.0) as i32;
    let y1 = ((cy + r + 1.0).ceil() as i32).min(h);
    for y in y0..y1 {
        for x in x0..x1 {
            let d = ((x as f32 + 0.5 - cx).powi(2) + (y as f32 + 0.5 - cy).powi(2)).sqrt();
            let cov = (r + 0.5 - d).clamp(0.0, 1.0);
            if cov > 0.0 {
                blend_over(img, x as u32, y as u32, c, cov);
            }
        }
    }
}

/// Draw an "✕" as two diagonal strokes centred at `(cx, cy)`, arm half-length
/// `h`, line half-width `t`.
fn stroke_x(img: &mut RgbaImage, cx: f32, cy: f32, h: f32, t: f32, c: Rgb) {
    seg(img, cx - h, cy - h, cx + h, cy + h, t, c);
    seg(img, cx - h, cy + h, cx + h, cy - h, t, c);
}

/// Anti-aliased line segment of half-width `t`, blended over the background.
fn seg(img: &mut RgbaImage, ax: f32, ay: f32, bx: f32, by: f32, t: f32, c: Rgb) {
    let x0 = (ax.min(bx) - t - 1.0).floor().max(0.0) as i32;
    let x1 = ((ax.max(bx) + t + 1.0).ceil() as i32).min(img.width() as i32);
    let y0 = (ay.min(by) - t - 1.0).floor().max(0.0) as i32;
    let y1 = ((ay.max(by) + t + 1.0).ceil() as i32).min(img.height() as i32);
    let (dx, dy) = (bx - ax, by - ay);
    let len2 = dx * dx + dy * dy;
    for y in y0..y1 {
        for x in x0..x1 {
            let (px, py) = (x as f32 + 0.5, y as f32 + 0.5);
            let s = if len2 > 0.0 {
                (((px - ax) * dx + (py - ay) * dy) / len2).clamp(0.0, 1.0)
            } else {
                0.0
            };
            let (qx, qy) = (ax + s * dx, ay + s * dy);
            let d = ((px - qx).powi(2) + (py - qy).powi(2)).sqrt();
            let cov = (t + 0.5 - d).clamp(0.0, 1.0);
            if cov > 0.0 {
                blend_over(img, x as u32, y as u32, c, cov);
            }
        }
    }
}

/// Set alpha to 0 outside a rounded rectangle of radius `r`. `top_only` rounds
/// just the top two corners (Linux); otherwise all four (macOS).
fn round_corners(img: &mut RgbaImage, r: u32, top_only: bool) {
    let (w, h) = (img.width(), img.height());
    let rf = r as f32;
    // (circle centre x, circle centre y, corner box origin x, corner box origin y)
    let mut corners = vec![(rf, rf, 0u32, 0u32), (w as f32 - rf, rf, w - r, 0u32)];
    if !top_only {
        corners.push((rf, h as f32 - rf, 0, h - r));
        corners.push((w as f32 - rf, h as f32 - rf, w - r, h - r));
    }
    for (ccx, ccy, bx, by) in corners {
        for y in by..(by + r).min(h) {
            for x in bx..(bx + r).min(w) {
                let d = ((x as f32 + 0.5 - ccx).powi(2) + (y as f32 + 0.5 - ccy).powi(2)).sqrt();
                let cov = (rf - d + 0.5).clamp(0.0, 1.0); // 1 inside, 0 outside the arc
                let p = img.get_pixel(x, y);
                let a = (p[3] as f32 * cov).round() as u8;
                img.put_pixel(x, y, Rgba([p[0], p[1], p[2], a]));
            }
        }
    }
}
```

- [ ] **Step 5: Run tests to verify they pass**

Run: `CARGO_BUILD_JOBS=4 cargo test --lib chrome`
Expected: PASS (all four chrome tests).

- [ ] **Step 6: Lint**

Run: `CARGO_BUILD_JOBS=4 cargo clippy --lib -- -D warnings`
Expected: no warnings.

- [ ] **Step 7: Commit**

```bash
git add src/chrome.rs src/lib.rs
git commit -m "feat(chrome): matte cell frames with padding + macos/linux window chrome"
```

---

### Task 4: Wire chrome into encode & record, confirm WebP alpha

**Files:**
- Modify: `src/lib.rs` (`encode`: build `Chrome`, matte each frame before dump/push)
- Modify: `src/record.rs` (`Recorder`: `chrome` field; matte in `push` and `push_card`)
- Modify: `src/encode.rs` (add an alpha-preservation test)

**Interfaces:**
- Consumes: `Chrome::{from_config, disabled, is_active, matte}` (Task 3); `Renderer::cell_size`.
- Produces: framed images flow through the existing `encode_webp` unchanged.

- [ ] **Step 1: Write the failing WebP-alpha test**

Add a `#[cfg(test)] mod tests` block at the end of `src/encode.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use image::{Rgba, RgbaImage};

    #[test]
    fn preserves_alpha_through_lossless() {
        let mut img = RgbaImage::from_pixel(8, 8, Rgba([10, 20, 30, 255]));
        img.put_pixel(0, 0, Rgba([0, 0, 0, 0])); // transparent corner
        let frames = vec![
            Frame { image: img.clone(), hold_cs: 10 },
            Frame { image: img, hold_cs: 10 },
        ];
        let bytes = encode_webp(&frames).unwrap();

        let decoded = webp::AnimDecoder::new(&bytes)
            .decode()
            .expect("decode animated webp");
        let f0 = decoded.get_frame(0).expect("frame 0");
        let (w, h) = (f0.width() as usize, f0.height() as usize);
        let data = f0.get_image();
        assert_eq!(data.len(), w * h * 4, "decoded as RGBA (alpha channel present)");
        assert_eq!(data[3], 0, "top-left pixel stayed transparent");
    }
}
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `CARGO_BUILD_JOBS=4 cargo test --lib encode::tests::preserves_alpha_through_lossless`
Expected: FAIL to compile (no `tests` module yet), then once compiling it will PASS if alpha survives.

> **If this test fails on the assertions** (decoded length is `w*h*3`, or `data[3] != 0`): libwebp lossless is dropping alpha in this build. Fall back per the spec — in `Chrome`, make `round_corners` a no-op (opaque square corners) for the chrome styles, drop the transparency, and note it in the module doc and README. Features (a)+(b) still land; only rounded transparency is sacrificed. Then adjust the Task 3 macOS/Linux corner assertions to expect `alpha == 255`.

- [ ] **Step 3: Wire chrome into `encode` (`src/lib.rs`)**

In `src/lib.rs`, add the import near the other `use crate::` lines:

```rust
use crate::chrome::Chrome;
```

In `pub fn encode`, replace this block:

```rust
    let renderer = Renderer::new(cfg.font_px);
    let min_cs = crate::config::min_hold_cs(cfg.max_fps);
```

with:

```rust
    let renderer = Renderer::new(cfg.font_px);
    let cell_h = renderer.cell_size().1;
    let chrome = match &cfg.chrome {
        Some(c) => Chrome::from_config(c, cell_h, (0, 0, 0)).context("chrome config")?,
        None => Chrome::disabled(),
    };
    let min_cs = crate::config::min_hold_cs(cfg.max_fps);
```

Then, inside the frame loop, immediately after the `let image = match spec.source()? { … };` statement and BEFORE the `if let Some(d) = dump_png` block, insert:

```rust
        let image = if chrome.is_active() {
            chrome.matte(&renderer, &image)
        } else {
            image
        };
```

- [ ] **Step 4: Wire chrome into `record` (`src/record.rs`)**

Add imports near the top of `src/record.rs` (with the other `use crate::` lines):

```rust
use crate::chrome::Chrome;
```

Add a field to `struct Recorder<'a>` (after `renderer: Renderer,`):

```rust
    chrome: Chrome,
```

In `Recorder::spawn`, replace the start of the `Ok(Recorder { … })` construction so the renderer and chrome are built first. Change:

```rust
        Ok(Recorder {
            renderer: Renderer::new(cfg.font_px),
            idle,
```

to:

```rust
        let renderer = Renderer::new(cfg.font_px);
        let cell_h = renderer.cell_size().1;
        let chrome = match &cfg.chrome {
            Some(c) => Chrome::from_config(c, cell_h, (0, 0, 0)).context("chrome config")?,
            None => Chrome::disabled(),
        };
        Ok(Recorder {
            renderer,
            chrome,
            idle,
```

In `fn push`, replace the final push:

```rust
        self.frames.push(Frame {
            image: img,
            hold_cs: hold_cs.max(self.min_cs),
        });
```

with:

```rust
        let image = if self.chrome.is_active() {
            self.chrome.matte(&self.renderer, &img)
        } else {
            img
        };
        self.frames.push(Frame {
            image,
            hold_cs: hold_cs.max(self.min_cs),
        });
```

In `fn push_card`, replace:

```rust
        self.frames.push(Frame {
            image: img,
            hold_cs: hold_cs.max(self.min_cs),
        });
```

with:

```rust
        let image = if self.chrome.is_active() {
            self.chrome.matte(&self.renderer, &img)
        } else {
            img
        };
        self.frames.push(Frame {
            image,
            hold_cs: hold_cs.max(self.min_cs),
        });
```

- [ ] **Step 5: Run the full test suite**

Run: `CARGO_BUILD_JOBS=4 cargo test`
Expected: PASS (all tests, including `preserves_alpha_through_lossless`).

- [ ] **Step 6: Manual visual verification**

Create `/tmp/chrome-demo.toml`:

```toml
cols = 40
rows = 3
font_px = 22
out = "/tmp/chrome-demo.webp"

[chrome]
style = "macos"
title = "hello.sh"
padding = 14

[[frame]]
card = { lines = ["$ echo hi", "hi", "$"], fg = "#d3d6db", bg = "#000000", border = false }
hold_cs = 100
```

Build and run, dumping the PNG so alpha is inspectable:

```bash
CARGO_BUILD_JOBS=4 cargo build --release
BIN="$(cargo metadata --format-version 1 | jq -r .target_directory)/release/ansidrama"
"$BIN" encode /tmp/chrome-demo.toml --dump-png /tmp/chrome-frames
```

Verify: the command prints an `OK:` line whose dimensions equal `40·cell_w + 28` × `bar_h + 3·cell_h + 28`; open `/tmp/chrome-frames/frame00.png` and confirm the macOS bar, three dots, centered title, terminal-bg padding, and transparent rounded corners. Repeat with `style = "linux"` and confirm the close button top-right and square bottom corners.

- [ ] **Step 7: Commit**

```bash
git add src/lib.rs src/record.rs src/encode.rs
git commit -m "feat: apply chrome matte in encode & record; assert webp alpha"
```

---

### Task 5: Documentation

**Files:**
- Modify: `README.md` (document the `[chrome]` table)
- Modify: `man/ansidrama.1` (add chrome fields to the CONFIGURATION section)

**Interfaces:** none (docs only).

- [ ] **Step 1: README**

In `README.md`, add a short subsection after the `encode`/`record` command descriptions (before or after the config examples, matching the surrounding style):

```markdown
### Window chrome & padding (optional)

Dress a recording as a window and give the cells breathing room. Add a
`[chrome]` table to either an `encode` or a `record` config:

```toml
[chrome]
style   = "macos"     # "macos" | "linux" | "none"   (default: none)
title   = "hello.sh"  # shown in the title bar
padding = 14          # px of terminal-bg inset around the cells
# bar   = "#2b2b2b"   # optional title-bar color
# text  = "#d0d0d0"   # optional title-text color
```

- **macos** — three traffic-light dots top-left, title centered, rounded
  corners (transparent outside).
- **linux** — a single close button top-right, title left-aligned, rounded
  top corners.
- **none** — no bar; `padding` only.

All chrome sizes scale from `font_px`. Padding works with `style = "none"` too.
```

(Note: the outer ```` ``` ```` fences above are literal README content.)

- [ ] **Step 2: Man page**

In `man/ansidrama.1`, extend the two config field lists. After the `encode` field list line ending `max_fps ", " out .`, and after the `record` field list line ending `... quit_keys ", " cursor .` (or wherever that list ends), add a sentence introducing the shared chrome table. Insert into the `.SH CONFIGURATION` section:

```roff
.PP
An optional
.B [chrome]
table (in either config) draws window chrome and padding around the screen:
.BR style " (" none ", " macos ", " linux "), " title ", " padding ", " bar ", " text .
.BR style " " macos " gives macOS traffic-light dots and a centered title;"
.BR linux " gives a single close button and a left title;"
.BR padding " insets the cells with the terminal background (works with " none ")."
```

- [ ] **Step 3: Verify the man page renders**

Run: `man --warnings -l man/ansidrama.1 >/dev/null`
Expected: no warnings printed.

- [ ] **Step 4: Commit**

```bash
git add README.md man/ansidrama.1
git commit -m "docs: document [chrome] window-chrome & padding config"
```

---

## Self-Review

**Spec coverage:**
- (a) Terminal frame, macOS + Linux styles → Task 1 (config), Task 3 (`draw_controls`, `round`), Task 4 (wiring). ✓
- (b) Padding, uniform, terminal-bg filled → Task 3 (`matte` none path), config `padding`. ✓
- Applied to captured frames *and* cards, identical dimensions → Task 4 wires both `encode` (cards + files) and `record` (`push` + `push_card`). ✓
- Mouse pointer stays correct → pointer stamped pre-matte on content image (unchanged in `push`); no `Renderer` geometry change. ✓
- Corner rounding + transparency owned by the chrome style → Task 3 `round`/`round_corners` (`top_only` per style). ✓
- WebP alpha risk confirmed early with a documented fallback → Task 4 Step 1–2. ✓
- Backward compatible (no `[chrome]` ⇒ no change) → `Chrome::disabled()` + `is_active()` skip; `chrome: Option<…>`. ✓
- Config surface on both `EncodeConfig` and `RecordConfig`, `deny_unknown_fields` → Task 1. ✓
- Docs (README + man) → Task 5. ✓

**Placeholder scan:** No TBD/TODO; every code step shows complete code.

**Type consistency:** `Chrome::{disabled, from_config, is_active, matte}`, `Renderer::{text_extents, draw_text, cell_size}`, `ChromeConfig`/`ChromeStyle` field names, and `Rgb = (u8,u8,u8)` are used identically across tasks. `from_config` signature `(&ChromeConfig, u32, Rgb) -> Result<Chrome>` matches all call sites (Tasks 3 tests, 4 encode, 4 record).
