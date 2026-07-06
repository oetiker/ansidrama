//! Overlay a mouse-pointer arrow onto a rendered frame, so scenes that click,
//! drag or scroll show where the pointer is. `ansidrama` generates the mouse
//! events, so it knows the pointer cell exactly.

use image::{Rgba, RgbaImage};

/// Classic arrow pointer, hotspot (tip) at the top-left. `O` = black outline,
/// `W` = white fill, space = transparent. Stamped 1:1 (native pixels) so it stays
/// crisp; ~12×19px reads well against an 18px-cell terminal.
const ARROW: &[&str] = &[
    "O",
    "OO",
    "OWO",
    "OWWO",
    "OWWWO",
    "OWWWWO",
    "OWWWWWO",
    "OWWWWWWO",
    "OWWWWWWWO",
    "OWWWWWWWWO",
    "OWWWWWWWWWO",
    "OWWWWWOOOOO",
    "OWWOWWO",
    "OWO OWWO",
    "OO  OWWO",
    "O    OWWO",
    "      OWWO",
    "       OWWO",
    "        OO",
];

/// Draw a text caret — a slim vertical beam at the left of the cell `(x0, y0)`,
/// full cell height, in `color`. Marks the app's insertion point while typing.
pub fn caret(img: &mut RgbaImage, x0: i32, y0: i32, cell_w: u32, cell_h: u32, color: (u8, u8, u8)) {
    let (w, h) = (img.width() as i32, img.height() as i32);
    let bar = (cell_w / 4).max(2) as i32;
    let c = Rgba([color.0, color.1, color.2, 255]);
    for y in y0..(y0 + cell_h as i32) {
        for x in x0..(x0 + bar) {
            if x >= 0 && y >= 0 && x < w && y < h {
                img.put_pixel(x as u32, y as u32, c);
            }
        }
    }
}

/// Stamp the pointer with its tip at pixel `(px, py)`, clipped to the image.
pub fn stamp(img: &mut RgbaImage, px: i32, py: i32) {
    let (w, h) = (img.width() as i32, img.height() as i32);
    let black = Rgba([0, 0, 0, 255]);
    let white = Rgba([255, 255, 255, 255]);
    for (dy, row) in ARROW.iter().enumerate() {
        for (dx, ch) in row.chars().enumerate() {
            let color = match ch {
                'O' => black,
                'W' => white,
                _ => continue,
            };
            let (x, y) = (px + dx as i32, py + dy as i32);
            if x >= 0 && y >= 0 && x < w && y < h {
                img.put_pixel(x as u32, y as u32, color);
            }
        }
    }
}
