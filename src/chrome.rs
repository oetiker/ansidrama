//! Wrap a rendered cell image in optional window chrome: terminal-bg padding
//! plus a macOS/Linux-style title bar, with corner rounding. Applied uniformly
//! to every frame (captured or card) so all frames keep identical dimensions.

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
                    r.draw_text(
                        img,
                        x,
                        baseline,
                        &self.title,
                        self.title_px,
                        self.text,
                        false,
                    );
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
    let mix = |f: u8, b: u8| {
        (f as f32 * cov + b as f32 * (1.0 - cov))
            .round()
            .clamp(0.0, 255.0) as u8
    };
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
