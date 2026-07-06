//! Colour type and parser for config-supplied colours (title cards).

/// An 8-bit-per-channel RGB colour.
pub type Rgb = (u8, u8, u8);

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
}
