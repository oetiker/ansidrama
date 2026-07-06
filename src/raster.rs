//! Rasterize a grid of styled cells ([`crate::grid::Cell`]) to an RGBA image,
//! using a bundled JetBrains Mono font. The cell box is sized from the font's own
//! advance/line metrics so box-drawing glyphs (┌─┐│└┘═║…) tile seamlessly.
//! Box-drawing and block/shade glyphs are hand-painted so they reach the exact
//! integer cell edges (font outlines leave a ~1px anti-aliased seam otherwise).

use ab_glyph::{Font, FontRef, PxScale, ScaleFont};
use image::{Rgba, RgbaImage};

use crate::color::Rgb;
use crate::grid::Cell;

const FONT_REGULAR: &[u8] = include_bytes!("../assets/JetBrainsMono-Regular.ttf");
const FONT_BOLD: &[u8] = include_bytes!("../assets/JetBrainsMono-Bold.ttf");

/// Fixed cell metrics + the loaded fonts, derived once and reused per frame.
pub struct Renderer {
    regular: FontRef<'static>,
    bold: FontRef<'static>,
    cell_w: u32,
    cell_h: u32,
    /// Per-glyph scale, stretched so the advance maps to exactly `cell_w` and the
    /// `ascent - descent` span maps to exactly `cell_h`. This sub-pixel correction
    /// is what makes box-drawing glyphs reach the integer cell edges and tile
    /// seamlessly.
    scale: PxScale,
    ascent: f32,
}

impl Renderer {
    /// Build a renderer at `px` font size. Larger `px` ⇒ larger cells ⇒ higher
    /// output resolution. 18px is small-but-crisp; 28–32px reads well in a README.
    pub fn new(px: f32) -> Self {
        let px = px.max(6.0);
        let regular = FontRef::try_from_slice(FONT_REGULAR).expect("regular font parses");
        let bold = FontRef::try_from_slice(FONT_BOLD).expect("bold font parses");
        let scaled = regular.as_scaled(PxScale::from(px));
        let adv = scaled.h_advance(regular.glyph_id('M')); // monospace: one advance
        let asc = scaled.ascent();
        let line = asc - scaled.descent(); // descent is negative
        let cell_w = adv.round().max(1.0) as u32;
        let cell_h = line.round().max(1.0) as u32; // no line_gap — box-drawing must fill the cell
        let scale = PxScale {
            x: px * cell_w as f32 / adv,
            y: px * cell_h as f32 / line,
        };
        let ascent = asc * cell_h as f32 / line;
        Renderer {
            regular,
            bold,
            cell_w,
            cell_h,
            scale,
            ascent,
        }
    }

    /// Pixel size of a `cols × rows` frame in this renderer's cell metrics.
    pub fn frame_size(&self, cols: u32, rows: u32) -> (u32, u32) {
        (self.cell_w * cols, self.cell_h * rows)
    }

    /// Pixel size of one cell.
    pub fn cell_size(&self) -> (u32, u32) {
        (self.cell_w, self.cell_h)
    }

    /// Top-left pixel of a 1-based terminal cell `(col, row)`.
    pub fn cell_origin(&self, col: u32, row: u32) -> (i32, i32) {
        (
            (col.saturating_sub(1) * self.cell_w) as i32,
            (row.saturating_sub(1) * self.cell_h) as i32,
        )
    }

    /// Render one frame. `cols`/`rows` fix the image size so every frame in an
    /// animation has identical dimensions even if a captured row is short.
    pub fn render(&self, grid: &[Vec<Cell>], cols: u32, rows: u32) -> RgbaImage {
        let (w, h) = self.frame_size(cols, rows);
        let mut img = RgbaImage::from_pixel(w, h, Rgba([0, 0, 0, 255]));

        for (ry, row) in grid.iter().enumerate().take(rows as usize) {
            for (cx, cell) in row.iter().enumerate().take(cols as usize) {
                let x0 = cx as u32 * self.cell_w;
                let y0 = ry as u32 * self.cell_h;

                for yy in 0..self.cell_h {
                    for xx in 0..self.cell_w {
                        img.put_pixel(
                            x0 + xx,
                            y0 + yy,
                            Rgba([cell.bg.0, cell.bg.1, cell.bg.2, 255]),
                        );
                    }
                }

                if cell.ch == ' ' || cell.ch == '\u{00a0}' {
                    continue;
                }
                if let Some(spec) = box_spec(cell.ch) {
                    self.draw_box(&mut img, x0, y0, spec, cell.fg);
                    continue;
                }
                if self.draw_block(&mut img, x0, y0, cell.ch, cell.fg) {
                    continue;
                }

                let font = if cell.bold { &self.bold } else { &self.regular };
                let glyph = font.glyph_id(cell.ch).with_scale_and_position(
                    self.scale,
                    ab_glyph::point(x0 as f32, y0 as f32 + self.ascent),
                );
                if let Some(outline) = font.outline_glyph(glyph) {
                    let bounds = outline.px_bounds();
                    outline.draw(|gx, gy, coverage| {
                        let px = bounds.min.x as i32 + gx as i32;
                        let py = bounds.min.y as i32 + gy as i32;
                        if px < 0 || py < 0 || px as u32 >= w || py as u32 >= h {
                            return;
                        }
                        let blended = blend(cell.fg, cell.bg, coverage);
                        img.put_pixel(
                            px as u32,
                            py as u32,
                            Rgba([blended.0, blended.1, blended.2, 255]),
                        );
                    });
                }
            }
        }
        img
    }

    /// Render a "silent-movie" title card directly at `w × h`, independent of the
    /// terminal cell size: a solid `bg` panel with a double-line frame and the
    /// `lines` centered (both axes) at `card_px` — so cards can use a larger,
    /// readable font than the small terminal capture. Frames stay `w × h` so they
    /// sit in the same animation.
    #[allow(clippy::too_many_arguments)]
    pub fn render_card(
        &self,
        w: u32,
        h: u32,
        lines: &[String],
        fg: Rgb,
        bg: Rgb,
        bold: bool,
        border: bool,
        card_px: f32,
    ) -> RgbaImage {
        let card_px = card_px.max(6.0);
        let mut img = RgbaImage::from_pixel(w, h, Rgba([bg.0, bg.1, bg.2, 255]));
        let font = if bold { &self.bold } else { &self.regular };
        let scaled = font.as_scaled(PxScale::from(card_px));
        let adv = scaled.h_advance(font.glyph_id('M'));
        let asc = scaled.ascent();
        let line_h = asc - scaled.descent();
        let scale = PxScale::from(card_px);

        if border && w > 8 && h > 8 {
            let inset = (card_px * 0.6).round() as i32;
            let gap = (card_px * 0.14).round().max(2.0) as i32;
            let t = (card_px / 20.0).round().max(1.0) as i32;
            rect_outline(
                &mut img,
                inset,
                inset,
                w as i32 - inset,
                h as i32 - inset,
                t,
                fg,
            );
            rect_outline(
                &mut img,
                inset + gap,
                inset + gap,
                w as i32 - inset - gap,
                h as i32 - inset - gap,
                t,
                fg,
            );
        }

        let block_h = lines.len() as f32 * line_h;
        let top = (h as f32 - block_h) / 2.0;
        for (i, line) in lines.iter().enumerate() {
            let text_w = line.chars().count() as f32 * adv;
            let mut x = (w as f32 - text_w) / 2.0;
            let baseline = top + i as f32 * line_h + asc;
            for ch in line.chars() {
                self.blit_glyph(&mut img, font, ch, x, baseline, scale, fg);
                x += adv;
            }
        }
        img
    }

    /// Draw one glyph, blending `fg` over whatever is already in the image.
    #[allow(clippy::too_many_arguments)]
    fn blit_glyph(
        &self,
        img: &mut RgbaImage,
        font: &FontRef<'static>,
        ch: char,
        x: f32,
        baseline: f32,
        scale: PxScale,
        fg: Rgb,
    ) {
        let glyph = font
            .glyph_id(ch)
            .with_scale_and_position(scale, ab_glyph::point(x, baseline));
        if let Some(outline) = font.outline_glyph(glyph) {
            let b = outline.px_bounds();
            let (iw, ih) = (img.width() as i32, img.height() as i32);
            outline.draw(|gx, gy, coverage| {
                let px = b.min.x as i32 + gx as i32;
                let py = b.min.y as i32 + gy as i32;
                if px < 0 || py < 0 || px >= iw || py >= ih {
                    return;
                }
                let cur = img.get_pixel(px as u32, py as u32);
                let under = (cur[0], cur[1], cur[2]);
                let bl = blend(fg, under, coverage);
                img.put_pixel(px as u32, py as u32, Rgba([bl.0, bl.1, bl.2, 255]));
            });
        }
    }

    /// Paint a box-drawing line char as exact rectangles. `spec` is the weight of
    /// each arm `[up, right, down, left]` — 0 = none, 1 = single, 2 = double.
    fn draw_box(&self, img: &mut RgbaImage, x0: u32, y0: u32, spec: [u8; 4], fg: Rgb) {
        const T: i32 = 1; // stroke thickness (1px, like a real terminal)
        const OFF: i32 = 2; // double-stroke offset from the centre line
        let [up, right, down, left] = spec;
        let (cw, ch) = (self.cell_w as i32, self.cell_h as i32);
        let (mx, my) = (cw / 2, ch / 2);
        let color = Rgba([fg.0, fg.1, fg.2, 255]);
        let mut rect = |xa: i32, xb: i32, ya: i32, yb: i32| {
            for yy in ya.max(0)..yb.min(ch) {
                for xx in xa.max(0)..xb.min(cw) {
                    img.put_pixel(x0 + xx as u32, y0 + yy as u32, color);
                }
            }
        };
        let hw = left.max(right); // horizontal weight
        let vw = up.max(down); // vertical weight
        let ycs: &[i32] = match hw {
            2 => &[my - OFF, my + OFF],
            1 => &[my],
            _ => &[],
        };
        let xcs: &[i32] = match vw {
            2 => &[mx - OFF, mx + OFF],
            1 => &[mx],
            _ => &[],
        };
        if hw > 0 {
            let xa = if left > 0 {
                0
            } else if !xcs.is_empty() {
                xcs.iter().min().unwrap() - T / 2
            } else {
                mx
            };
            let xb = if right > 0 {
                cw
            } else if !xcs.is_empty() {
                xcs.iter().max().unwrap() + T / 2
            } else {
                mx
            };
            for &yc in ycs {
                rect(xa, xb, yc - T / 2, yc - T / 2 + T);
            }
        }
        if vw > 0 {
            let ya = if up > 0 {
                0
            } else if !ycs.is_empty() {
                ycs.iter().min().unwrap() - T / 2
            } else {
                my
            };
            let yb = if down > 0 {
                ch
            } else if !ycs.is_empty() {
                ycs.iter().max().unwrap() + T / 2
            } else {
                my
            };
            for &xc in xcs {
                rect(xc - T / 2, xc - T / 2 + T, ya, yb);
            }
        }
    }

    /// Paint a block / shade element (U+2580..U+2595) as exact fills so window
    /// shadows, buttons and scrollbars tile seamlessly. Returns false if `ch` is
    /// not a block.
    fn draw_block(&self, img: &mut RgbaImage, x0: u32, y0: u32, ch: char, fg: Rgb) -> bool {
        let (cw, chh) = (self.cell_w, self.cell_h);
        let color = Rgba([fg.0, fg.1, fg.2, 255]);
        let mut solid = |xa: u32, xb: u32, ya: u32, yb: u32| {
            for yy in ya..yb {
                for xx in xa..xb {
                    img.put_pixel(x0 + xx, y0 + yy, color);
                }
            }
        };
        let cp = ch as u32;
        match ch {
            '█' => solid(0, cw, 0, chh),
            '▀' => solid(0, cw, 0, chh / 2),
            '▐' => solid(cw / 2, cw, 0, chh),
            '▔' => solid(0, cw, 0, (chh / 8).max(1)),
            '▕' => solid(cw - (cw / 8).max(1), cw, 0, chh),
            '░' | '▒' | '▓' => {
                for yy in 0..chh {
                    for xx in 0..cw {
                        let (gx, gy) = (x0 + xx, y0 + yy);
                        let on = match ch {
                            '░' => gx % 2 == 0 && gy % 2 == 0,  // ~25%
                            '▒' => (gx + gy) % 2 == 0,          // ~50% checker
                            _ => !(gx % 2 == 1 && gy % 2 == 1), // ▓ ~75%
                        };
                        if on {
                            img.put_pixel(gx, gy, color);
                        }
                    }
                }
            }
            _ if (0x2581..=0x2587).contains(&cp) => {
                let n = cp - 0x2580; // 1..7
                let filled = (chh * n / 8).max(1).min(chh);
                solid(0, cw, chh - filled, chh);
            }
            _ if (0x2589..=0x258F).contains(&cp) => {
                let n = 0x2590 - cp; // 7..1
                let filled = (cw * n / 8).max(1).min(cw);
                solid(0, filled, 0, chh);
            }
            _ => return false,
        }
        true
    }
}

/// Arm weights `[up, right, down, left]` for a box-drawing line char: 0 = none,
/// 1 = single, 2 = double. `None` ⇒ not a handled box char (fall back to font).
fn box_spec(ch: char) -> Option<[u8; 4]> {
    Some(match ch {
        '─' => [0, 1, 0, 1],
        '│' => [1, 0, 1, 0],
        '┌' => [0, 1, 1, 0],
        '┐' => [0, 0, 1, 1],
        '└' => [1, 1, 0, 0],
        '┘' => [1, 0, 0, 1],
        '├' => [1, 1, 1, 0],
        '┤' => [1, 0, 1, 1],
        '┬' => [0, 1, 1, 1],
        '┴' => [1, 1, 0, 1],
        '┼' => [1, 1, 1, 1],
        '═' => [0, 2, 0, 2],
        '║' => [2, 0, 2, 0],
        '╔' => [0, 2, 2, 0],
        '╗' => [0, 0, 2, 2],
        '╚' => [2, 2, 0, 0],
        '╝' => [2, 0, 0, 2],
        '╠' => [2, 2, 2, 0],
        '╣' => [2, 0, 2, 2],
        '╦' => [0, 2, 2, 2],
        '╩' => [2, 2, 0, 2],
        '╬' => [2, 2, 2, 2],
        '╒' => [0, 2, 1, 0],
        '╓' => [0, 1, 2, 0],
        '╕' => [0, 0, 1, 2],
        '╖' => [0, 0, 2, 1],
        '╘' => [1, 2, 0, 0],
        '╙' => [2, 1, 0, 0],
        '╛' => [1, 0, 0, 2],
        '╜' => [2, 0, 0, 1],
        '╞' => [1, 2, 1, 0],
        '╟' => [2, 1, 2, 0],
        '╡' => [1, 0, 1, 2],
        '╢' => [2, 0, 2, 1],
        '╤' => [0, 2, 1, 2],
        '╥' => [0, 1, 2, 1],
        '╧' => [1, 2, 0, 2],
        '╨' => [2, 1, 0, 1],
        '╪' => [1, 2, 1, 2],
        '╫' => [2, 1, 2, 1],
        _ => return None,
    })
}

/// Draw a `t`-pixel-thick rectangle outline `[x0,x1) × [y0,y1)` in `color`.
fn rect_outline(img: &mut RgbaImage, x0: i32, y0: i32, x1: i32, y1: i32, t: i32, color: Rgb) {
    let c = Rgba([color.0, color.1, color.2, 255]);
    let (w, h) = (img.width() as i32, img.height() as i32);
    let mut put = |x: i32, y: i32| {
        if x >= 0 && y >= 0 && x < w && y < h {
            img.put_pixel(x as u32, y as u32, c);
        }
    };
    for x in x0..x1 {
        for k in 0..t {
            put(x, y0 + k);
            put(x, y1 - 1 - k);
        }
    }
    for y in y0..y1 {
        for k in 0..t {
            put(x0 + k, y);
            put(x1 - 1 - k, y);
        }
    }
}

/// Alpha-blend `fg` over `bg` by `coverage` (0..=1).
fn blend(fg: Rgb, bg: Rgb, a: f32) -> Rgb {
    let mix = |f: u8, b: u8| {
        (f as f32 * a + b as f32 * (1.0 - a))
            .round()
            .clamp(0.0, 255.0) as u8
    };
    (mix(fg.0, bg.0), mix(fg.1, bg.1), mix(fg.2, bg.2))
}
