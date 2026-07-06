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
