//! Rasterize a grid of styled cells ([`crate::grid::Cell`]) to an RGBA image,
//! using a bundled JetBrains Mono Nerd Font. The cell box is sized from the font's own
//! advance/line metrics so box-drawing glyphs (┌─┐│└┘═║…) tile seamlessly.
//! Box-drawing and block/shade glyphs are hand-painted so they reach the exact
//! integer cell edges (font outlines leave a ~1px anti-aliased seam otherwise).

use ab_glyph::{Font, FontRef, PxScale, ScaleFont};
use image::{Rgba, RgbaImage};

use crate::color::Rgb;
use crate::grid::Cell;

// Three fonts, each doing one job, consulted in order (see `face`).
//
// Text comes from JetBrains Mono. Icons come from Symbols Nerd Font, which is
// the Nerd Fonts icon set — Font Awesome, Devicons, Material and friends — as a
// standalone face: those live in the Private Use Area, where Unicode assigns no
// meaning and the Nerd Fonts codepoint map is the de-facto standard the TUIs
// that draw them target. Everything else Unicode calls a symbol comes from
// JuliaMono, which covers the symbol blocks in full (100% of Arrows, Misc
// Technical, Geometric Shapes, Misc Symbols, Dingbats and Braille) where
// JetBrains Mono has 31-45% and no Braille at all.
//
// Layering rather than patching is deliberate: a Nerd-Font-*patched* JetBrains
// Mono carries every icon twice, once per weight, for 5.1MB and adds nothing
// outside the PUA — the arrangement here is 2MB smaller and covers far more.
// The *Mono* symbol variant constrains icons to one advance, which is what a
// cell grid needs; the proportional one would overhang its neighbour.
const FONT_REGULAR: &[u8] = include_bytes!("../assets/JetBrainsMono-Regular.ttf");
const FONT_BOLD: &[u8] = include_bytes!("../assets/JetBrainsMono-Bold.ttf");
const FONT_ICONS: &[u8] = include_bytes!("../assets/SymbolsNerdFontMono-Regular.ttf");
const FONT_SYMBOLS: &[u8] = include_bytes!("../assets/JuliaMono-Regular.ttf");

/// Fixed cell metrics + the loaded fonts, derived once and reused per frame.
pub struct Renderer {
    regular: FontRef<'static>,
    bold: FontRef<'static>,
    /// Fallback faces, in consultation order. Unlike the primary these carry no
    /// pre-computed scale: they are fitted per glyph (see [`Renderer::draw_fitted`]),
    /// because a fallback's metrics say nothing useful about our cell. A
    /// symbols-only font has no `M` to take an advance from at all — asking for
    /// one returns the em square, and scaling by it squashed every icon to half
    /// width.
    fallbacks: Vec<FontRef<'static>>,
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

        let fallbacks = [FONT_ICONS, FONT_SYMBOLS]
            .iter()
            .map(|bytes| FontRef::try_from_slice(bytes).expect("fallback font parses"))
            .collect();

        Renderer {
            regular,
            bold,
            fallbacks,
            cell_w,
            cell_h,
            scale,
            ascent,
        }
    }

    /// The first fallback face carrying `ch`, if any.
    fn fallback_for(&self, ch: char) -> Option<&FontRef<'static>> {
        self.fallbacks.iter().find(|f| f.glyph_id(ch).0 != 0)
    }

    /// Draw `ch` from a fallback face, scaled *uniformly* to fill the box at
    /// `(bx, by, bw, bh)` and centred in it. Returns false if it has no outline.
    ///
    /// The primary font is stretched to the cell on purpose — that is what makes
    /// box-drawing tile — but a fallback must not be: its advance-to-line ratio
    /// is its own, and forcing it onto ours distorts every glyph (JuliaMono came
    /// out 9% too tall, the icon font twice as tall as wide). Fitting the glyph's
    /// own outline keeps its proportions and cannot overflow into the neighbour,
    /// which is what a terminal does with a fallback face. Nothing that must tile
    /// arrives here: box-drawing and block elements are hand-painted upstream.
    #[allow(clippy::too_many_arguments)]
    fn draw_fitted(
        &self,
        img: &mut RgbaImage,
        font: &FontRef<'static>,
        ch: char,
        bx: f32,
        by: f32,
        bw: f32,
        bh: f32,
        fg: Rgb,
    ) -> bool {
        let id = font.glyph_id(ch);
        // Measure at a nominal scale, then pick the uniform scale that fits.
        let probe = bh.max(1.0);
        let measured = id.with_scale_and_position(PxScale::from(probe), ab_glyph::point(0.0, 0.0));
        let Some(outline) = font.outline_glyph(measured) else {
            return false;
        };
        let b = outline.px_bounds();
        let (gw, gh) = (b.width().max(0.01), b.height().max(0.01));
        let k = (bw / gw).min(bh / gh);
        let scale = PxScale::from(probe * k);

        let placed = id.with_scale_and_position(scale, ab_glyph::point(0.0, 0.0));
        let Some(outline) = font.outline_glyph(placed) else {
            return false;
        };
        let b = outline.px_bounds();
        // Centre the glyph's own bounds inside the box.
        let ox = bx + (bw - b.width()) / 2.0 - b.min.x;
        let oy = by + (bh - b.height()) / 2.0 - b.min.y;
        let (iw, ih) = (img.width() as i32, img.height() as i32);
        outline.draw(|gx, gy, coverage| {
            let px = (b.min.x + ox) as i32 + gx as i32;
            let py = (b.min.y + oy) as i32 + gy as i32;
            if px < 0 || py < 0 || px >= iw || py >= ih {
                return;
            }
            let cur = img.get_pixel(px as u32, py as u32);
            let bl = blend(fg, (cur[0], cur[1], cur[2]), coverage);
            img.put_pixel(px as u32, py as u32, Rgba([bl.0, bl.1, bl.2, 255]));
        });
        true
    }

    /// Pixel size of a `cols × rows` frame in this renderer's cell metrics.
    pub fn frame_size(&self, cols: u32, rows: u32) -> (u32, u32) {
        (self.cell_w * cols, self.cell_h * rows)
    }

    /// Pixel size of one cell.
    pub fn cell_size(&self) -> (u32, u32) {
        (self.cell_w, self.cell_h)
    }

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
    #[allow(clippy::too_many_arguments)]
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
        let adv = font
            .as_scaled(PxScale::from(px))
            .h_advance(font.glyph_id('M'));
        let mut cx = x;
        for ch in text.chars() {
            self.blit_glyph(img, font, ch, cx, baseline, PxScale::from(px), color);
            cx += adv;
        }
    }

    /// Top-left pixel of a 1-based terminal cell `(col, row)`.
    pub fn cell_origin(&self, col: u32, row: u32) -> (i32, i32) {
        (
            (col.saturating_sub(1) * self.cell_w) as i32,
            (row.saturating_sub(1) * self.cell_h) as i32,
        )
    }

    /// Draw a block cursor over 1-based cell `(col, row)`: a solid rectangle that
    /// contrasts the cell background, with the cell's glyph re-drawn in the cell's
    /// background colour on top — inverse video. So a character under the cursor is
    /// inverted, and a blank insert cell shows a solid block.
    pub fn draw_block_cursor(&self, img: &mut RgbaImage, col: u32, row: u32, cell: &Cell) {
        let (x0, y0) = self.cell_origin(col, row);
        let bg = cell.bg;
        let lum = bg.0 as u32 + bg.1 as u32 + bg.2 as u32;
        let block: Rgb = if lum > 384 {
            (20, 24, 28)
        } else {
            (235, 235, 235)
        };
        let (iw, ih) = (img.width() as i32, img.height() as i32);
        for yy in y0..(y0 + self.cell_h as i32) {
            for xx in x0..(x0 + self.cell_w as i32) {
                if xx >= 0 && yy >= 0 && xx < iw && yy < ih {
                    img.put_pixel(xx as u32, yy as u32, Rgba([block.0, block.1, block.2, 255]));
                }
            }
        }
        let ch = cell.ch;
        if ch == ' ' || ch == '\u{00a0}' || x0 < 0 || y0 < 0 {
            return;
        }
        let (ux, uy) = (x0 as u32, y0 as u32);
        // Re-draw the glyph in the cell's background colour, over the block.
        if let Some(spec) = box_spec(ch) {
            self.draw_box(img, ux, uy, spec, bg);
        } else if !self.draw_block(img, ux, uy, ch, bg) {
            let font = if cell.bold { &self.bold } else { &self.regular };
            self.blit_glyph(
                img,
                font,
                ch,
                x0 as f32,
                y0 as f32 + self.ascent,
                self.scale,
                bg,
            );
        }
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
                if font.glyph_id(cell.ch).0 == 0 {
                    // Not in the text font: a fallback face, fitted to the cell.
                    let drawn = match self.fallback_for(cell.ch) {
                        Some(f) => self.draw_fitted(
                            &mut img,
                            f,
                            cell.ch,
                            x0 as f32,
                            y0 as f32,
                            self.cell_w as f32,
                            self.cell_h as f32,
                            cell.fg,
                        ),
                        None => false,
                    };
                    if !drawn {
                        self.draw_tofu(
                            &mut img,
                            x0 as f32,
                            y0 as f32,
                            self.cell_w as f32,
                            self.cell_h as f32,
                            cell.fg,
                        );
                    }
                    continue;
                }
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
    /// terminal cell size: a solid `bg` panel with a double-line frame and centered
    /// text — the first line (the title) at `title_px`, the rest (subtitles) at the
    /// smaller `subtitle_px`. Frames stay `w × h` so they sit in the same animation.
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
        title_px: f32,
        subtitle_px: f32,
    ) -> RgbaImage {
        let title_px = title_px.max(6.0);
        let subtitle_px = subtitle_px.max(6.0);
        let mut img = RgbaImage::from_pixel(w, h, Rgba([bg.0, bg.1, bg.2, 255]));
        let font = if bold { &self.bold } else { &self.regular };

        if border && w > 8 && h > 8 {
            let inset = (title_px * 0.6).round() as i32;
            let gap = (title_px * 0.14).round().max(2.0) as i32;
            let t = (title_px / 20.0).round().max(1.0) as i32;
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

        // Per-line metrics: line 0 is the title, the rest are subtitles.
        let metrics = |px: f32| {
            let s = font.as_scaled(PxScale::from(px));
            (
                s.h_advance(font.glyph_id('M')),
                s.ascent(),
                s.ascent() - s.descent(),
            )
        };
        let per_line: Vec<(f32, f32, f32, f32)> = lines
            .iter()
            .enumerate()
            .map(|(i, _)| {
                let px = if i == 0 { title_px } else { subtitle_px };
                let (adv, asc, line_h) = metrics(px);
                (px, adv, asc, line_h)
            })
            .collect();

        let block_h: f32 = per_line.iter().map(|m| m.3).sum();
        let mut y = (h as f32 - block_h) / 2.0;
        for (line, &(px, adv, asc, line_h)) in lines.iter().zip(&per_line) {
            let text_w = line.chars().count() as f32 * adv;
            let mut x = (w as f32 - text_w) / 2.0;
            let baseline = y + asc;
            for ch in line.chars() {
                self.blit_glyph(&mut img, font, ch, x, baseline, PxScale::from(px), fg);
                x += adv;
            }
            y += line_h;
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
        // Free-size drawing (title cards, chrome titles): the box a fallback is
        // fitted into is the advance and line height the primary would occupy
        // here, so the glyph sits on the same rhythm as the text around it.
        if font.glyph_id(ch).0 == 0 {
            let s = font.as_scaled(scale);
            let (asc, desc) = (s.ascent(), s.descent());
            let adv = s.h_advance(font.glyph_id('M'));
            let (bx, by, bh) = (x, baseline - asc, asc - desc);
            let drawn = match self.fallback_for(ch) {
                Some(f) => self.draw_fitted(img, f, ch, bx, by, adv, bh, fg),
                None => false,
            };
            if !drawn {
                self.draw_tofu(img, bx, by, adv, bh, fg);
            }
            return;
        }
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

    /// A hollow rectangle standing in for a codepoint the font has no glyph for.
    ///
    /// `.notdef` (glyph id 0) has no outline in this font, so drawing it paints
    /// *nothing*: the terminal shows an icon and the recording shows an empty
    /// cell, with no sign anything is missing. A visible box is the conventional
    /// tofu and makes the gap something a reviewer can see.
    fn draw_tofu(&self, img: &mut RgbaImage, x: f32, top: f32, w: f32, h: f32, color: Rgb) {
        let inset_x = (w * 0.18).round().max(1.0);
        let inset_y = (h * 0.14).round().max(1.0);
        let t = (w / 10.0).round().clamp(1.0, 2.0) as i32;
        rect_outline(
            img,
            (x + inset_x) as i32,
            (top + inset_y) as i32,
            (x + w - inset_x) as i32,
            (top + h - inset_y) as i32,
            t,
            color,
        );
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

    /// A codepoint the font has no glyph for must still leave a mark. It used to
    /// draw nothing, so a missing icon was indistinguishable from a blank cell.
    #[test]
    fn missing_glyph_draws_visible_tofu() {
        let r = Renderer::new(20.0);
        let grid = vec![vec![Cell {
            ch: '\u{1F600}', // no emoji in JetBrains Mono, patched or not
            fg: (255, 255, 255),
            bg: (0, 0, 0),
            bold: false,
        }]];
        let img = r.render(&grid, 1, 1);
        assert!(
            img.pixels().any(|p| p[0] > 200),
            "a missing glyph should paint a visible box"
        );
    }

    fn draws(r: &Renderer, ch: char, bold: bool) -> bool {
        let grid = vec![vec![Cell {
            ch,
            fg: (255, 255, 255),
            bg: (0, 0, 0),
            bold,
        }]];
        r.render(&grid, 1, 1).pixels().any(|p| p[0] > 200)
    }

    /// Each layer of the font chain pulls its weight: icons from Symbols Nerd
    /// Font (Private Use Area), Unicode symbols from JuliaMono, and text and
    /// powerline from JetBrains Mono itself — in both weights.
    #[test]
    fn every_layer_of_the_font_chain_draws() {
        let r = Renderer::new(20.0);
        for ch in [
            '\u{f15c}', '\u{f0f6}', // Nerd Font icons (PUA)
            '\u{25A4}', '\u{2315}', // Unicode symbols mdmost uses
            '\u{2801}', '\u{2603}', // Braille, Misc Symbols
            '\u{e0b0}', 'A', // powerline + text, from the primary
        ] {
            for bold in [false, true] {
                assert!(
                    draws(&r, ch, bold),
                    "U+{:04X} should draw (bold={bold})",
                    ch as u32
                );
            }
        }
    }

    /// Ink bounding box of a single-cell render, as `(w, h)`.
    fn ink_box(img: &RgbaImage) -> (f32, f32) {
        let (mut x0, mut y0, mut x1, mut y1) = (u32::MAX, u32::MAX, 0u32, 0u32);
        for (x, y, p) in img.enumerate_pixels() {
            if p[0] > 100 {
                x0 = x0.min(x);
                y0 = y0.min(y);
                x1 = x1.max(x);
                y1 = y1.max(y);
            }
        }
        ((x1 - x0 + 1) as f32, (y1 - y0 + 1) as f32)
    }

    /// A fallback face must not be stretched onto the primary's cell aspect.
    ///
    /// The cell is far taller than it is wide, so a glyph stretched to fill it
    /// comes out at roughly the cell's aspect instead of its own. U+25A4 is a
    /// square, which makes it a direct probe: it must draw square.
    #[test]
    fn fallback_glyphs_keep_their_proportions() {
        let r = Renderer::new(40.0);
        let (cw, chh) = r.cell_size();
        assert!(chh > cw, "cell is taller than wide, or this proves nothing");
        let grid = vec![vec![Cell {
            ch: '\u{25A4}', // SQUARE WITH HORIZONTAL FILL — from JuliaMono
            fg: (255, 255, 255),
            bg: (0, 0, 0),
            bold: false,
        }]];
        let (w, h) = ink_box(&r.render(&grid, 1, 1));
        assert!(
            (w / h - 1.0).abs() < 0.25,
            "a square glyph drew {w}x{h} — fallback stretched to the cell aspect"
        );
        assert!(
            w <= cw as f32 && h <= chh as f32,
            "glyph {w}x{h} overflows its {cw}x{chh} cell"
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
