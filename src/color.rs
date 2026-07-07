//! Colour type and parser for config-supplied colours (title cards).

/// An 8-bit-per-channel RGB colour.
pub type Rgb = (u8, u8, u8);

/// Classic VGA/xterm 16-colour palette (SGR 30–37/90–97 and the low 16 of the
/// 256-colour cube).
const PALETTE16: [Rgb; 16] = [
    (0, 0, 0),
    (170, 0, 0),
    (0, 170, 0),
    (170, 85, 0),
    (0, 0, 170),
    (170, 0, 170),
    (0, 170, 170),
    (170, 170, 170),
    (85, 85, 85),
    (255, 85, 85),
    (85, 255, 85),
    (255, 255, 85),
    (85, 85, 255),
    (255, 85, 255),
    (85, 255, 255),
    (255, 255, 255),
];

/// xterm 256-colour index → RGB (16 base + 6×6×6 cube + greyscale ramp).
pub fn index_to_rgb(i: u8) -> Rgb {
    match i {
        0..=15 => PALETTE16[(i & 0x0f) as usize],
        16..=231 => {
            let i = i - 16;
            let steps = [0u8, 95, 135, 175, 215, 255];
            (
                steps[(i / 36) as usize],
                steps[((i / 6) % 6) as usize],
                steps[(i % 6) as usize],
            )
        }
        232..=255 => {
            let v = 8 + 10 * (i - 232);
            (v, v, v)
        }
    }
}

/// Resolve a `vt100` cell colour to RGB; `Default` falls back to `default`.
pub fn vt_color(c: vt100::Color, default: Rgb) -> Rgb {
    match c {
        vt100::Color::Default => default,
        vt100::Color::Idx(i) => index_to_rgb(i),
        vt100::Color::Rgb(r, g, b) => (r, g, b),
    }
}

/// Parse a colour string: `#rrggbb` / `#rgb` hex, or a small set of names.
/// Returns an error string on anything unrecognised.
pub fn parse(s: &str) -> Result<Rgb, String> {
    let s = s.trim();
    if let Some(hex) = s.strip_prefix('#') {
        return parse_hex(hex);
    }
    match s.to_ascii_lowercase().as_str() {
        "black" => Ok((0, 0, 0)),
        "white" => Ok((255, 255, 255)),
        "red" => Ok((220, 38, 38)),
        "green" => Ok((22, 163, 74)),
        "blue" => Ok((37, 99, 235)),
        "yellow" => Ok((254, 249, 195)),
        "grey" | "gray" => Ok((148, 163, 184)),
        _ => Err(format!(
            "unrecognised colour {s:?} (use #rrggbb or a basic name)"
        )),
    }
}

fn parse_hex(hex: &str) -> Result<Rgb, String> {
    let bad = || format!("bad hex colour #{hex}");
    match hex.len() {
        6 => {
            let r = u8::from_str_radix(&hex[0..2], 16).map_err(|_| bad())?;
            let g = u8::from_str_radix(&hex[2..4], 16).map_err(|_| bad())?;
            let b = u8::from_str_radix(&hex[4..6], 16).map_err(|_| bad())?;
            Ok((r, g, b))
        }
        3 => {
            let d = |i: usize| {
                let v = u8::from_str_radix(&hex[i..i + 1], 16).map_err(|_| bad())?;
                Ok::<u8, String>(v * 17) // 0xf → 0xff
            };
            Ok((d(0)?, d(1)?, d(2)?))
        }
        _ => Err(bad()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex6() {
        assert_eq!(parse("#fef9c3").unwrap(), (0xfe, 0xf9, 0xc3));
    }

    #[test]
    fn hex3_expands() {
        assert_eq!(parse("#fff").unwrap(), (255, 255, 255));
    }

    #[test]
    fn names() {
        assert_eq!(parse("black").unwrap(), (0, 0, 0));
        assert_eq!(parse("White").unwrap(), (255, 255, 255));
    }

    #[test]
    fn bad() {
        assert!(parse("nope").is_err());
        assert!(parse("#12").is_err());
    }

    #[test]
    fn index_cube_endpoints() {
        assert_eq!(index_to_rgb(9), (255, 85, 85)); // bright red (low 16)
        assert_eq!(index_to_rgb(16), (0, 0, 0)); // cube start
        assert_eq!(index_to_rgb(231), (255, 255, 255)); // cube end
        assert_eq!(index_to_rgb(232), (8, 8, 8)); // greyscale ramp start
    }

    #[test]
    fn vt_color_maps_each_variant() {
        assert_eq!(vt_color(vt100::Color::Default, (1, 2, 3)), (1, 2, 3));
        assert_eq!(vt_color(vt100::Color::Rgb(9, 8, 7), (0, 0, 0)), (9, 8, 7));
        assert_eq!(vt_color(vt100::Color::Idx(9), (0, 0, 0)), (255, 85, 85));
    }
}
