//! Translate tmux-style key names to the bytes a terminal application reads.
//! This is the one piece of tmux behaviour we own after retiring the tmux
//! dependency: `send-keys Down` becomes `key_bytes("Down") == b"\x1b[B"`.

use anyhow::{bail, Result};

/// Named key → bytes. Returns `None` for names handled elsewhere (modifiers,
/// single characters, raw escapes).
fn named(name: &str) -> Option<Vec<u8>> {
    let b: &[u8] = match name {
        "Enter" | "Return" => b"\r",
        "Tab" => b"\t",
        "BTab" => b"\x1b[Z",
        "Escape" | "Esc" => b"\x1b",
        "Space" => b" ",
        "BSpace" | "Backspace" => b"\x7f",
        "Up" => b"\x1b[A",
        "Down" => b"\x1b[B",
        "Right" => b"\x1b[C",
        "Left" => b"\x1b[D",
        "Home" => b"\x1b[H",
        "End" => b"\x1b[F",
        "PageUp" | "PPage" => b"\x1b[5~",
        "PageDown" | "NPage" => b"\x1b[6~",
        "Insert" | "IC" => b"\x1b[2~",
        "Delete" | "DC" => b"\x1b[3~",
        "F1" => b"\x1bOP",
        "F2" => b"\x1bOQ",
        "F3" => b"\x1bOR",
        "F4" => b"\x1bOS",
        "F5" => b"\x1b[15~",
        "F6" => b"\x1b[17~",
        "F7" => b"\x1b[18~",
        "F8" => b"\x1b[19~",
        "F9" => b"\x1b[20~",
        "F10" => b"\x1b[21~",
        "F11" => b"\x1b[23~",
        "F12" => b"\x1b[24~",
        _ => return None,
    };
    Some(b.to_vec())
}

/// `C-<x>` → control byte (letters and `@ [ \ ] ^ _`, plus `C-Space` = NUL).
fn ctrl(rest: &str) -> Result<Vec<u8>> {
    if rest.chars().count() == 1 {
        let c = rest.chars().next().unwrap();
        if c == ' ' {
            return Ok(vec![0x00]);
        }
        let up = c.to_ascii_uppercase() as u8;
        if (b'@'..=b'_').contains(&up) {
            return Ok(vec![up & 0x1f]);
        }
    }
    bail!("unknown control key C-{rest}")
}

/// `S-<x>` → shifted key (`S-Tab` special-cased; letters → uppercase).
fn shift(rest: &str) -> Result<Vec<u8>> {
    if rest.eq_ignore_ascii_case("Tab") {
        return Ok(b"\x1b[Z".to_vec());
    }
    if rest.chars().count() == 1 {
        let c = rest.chars().next().unwrap();
        return Ok(c.to_ascii_uppercase().to_string().into_bytes());
    }
    bail!("unknown shifted key S-{rest}")
}

/// Translate a tmux-style key name to the bytes an app reads. Handles named
/// keys, `C-`/`M-`/`S-` modifiers, single literal characters, and raw escape
/// sequences (anything already starting with ESC passes through unchanged).
pub fn key_bytes(name: &str) -> Result<Vec<u8>> {
    if name.starts_with('\x1b') {
        return Ok(name.as_bytes().to_vec());
    }
    if let Some(rest) = name.strip_prefix("C-") {
        return ctrl(rest);
    }
    if let Some(rest) = name.strip_prefix("M-") {
        let mut v = vec![0x1b];
        v.extend(key_bytes(rest)?);
        return Ok(v);
    }
    if let Some(rest) = name.strip_prefix("S-") {
        return shift(rest);
    }
    if let Some(bytes) = named(name) {
        return Ok(bytes);
    }
    // Exactly one character → its literal UTF-8 bytes.
    let mut it = name.chars();
    if let (Some(c), None) = (it.next(), it.next()) {
        return Ok(c.to_string().into_bytes());
    }
    bail!("unknown key name {name:?}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn named_keys() {
        assert_eq!(key_bytes("Enter").unwrap(), b"\r");
        assert_eq!(key_bytes("Tab").unwrap(), b"\t");
        assert_eq!(key_bytes("Escape").unwrap(), b"\x1b");
        assert_eq!(key_bytes("BSpace").unwrap(), b"\x7f");
        assert_eq!(key_bytes("Up").unwrap(), b"\x1b[A");
        assert_eq!(key_bytes("Down").unwrap(), b"\x1b[B");
        assert_eq!(key_bytes("Right").unwrap(), b"\x1b[C");
        assert_eq!(key_bytes("Left").unwrap(), b"\x1b[D");
        assert_eq!(key_bytes("Home").unwrap(), b"\x1b[H");
        assert_eq!(key_bytes("End").unwrap(), b"\x1b[F");
        assert_eq!(key_bytes("PageUp").unwrap(), b"\x1b[5~");
        assert_eq!(key_bytes("PageDown").unwrap(), b"\x1b[6~");
        assert_eq!(key_bytes("Delete").unwrap(), b"\x1b[3~");
        assert_eq!(key_bytes("F1").unwrap(), b"\x1bOP");
        assert_eq!(key_bytes("F10").unwrap(), b"\x1b[21~");
        assert_eq!(key_bytes("F12").unwrap(), b"\x1b[24~");
        assert_eq!(key_bytes("BTab").unwrap(), b"\x1b[Z");
    }

    #[test]
    fn modifiers() {
        assert_eq!(key_bytes("C-c").unwrap(), vec![0x03]);
        assert_eq!(key_bytes("C-a").unwrap(), vec![0x01]);
        assert_eq!(key_bytes("S-Tab").unwrap(), b"\x1b[Z");
        assert_eq!(key_bytes("M-x").unwrap(), b"\x1bx");
    }

    #[test]
    fn literal_and_raw() {
        assert_eq!(key_bytes("q").unwrap(), b"q");
        // Raw escape sequence passes straight through (the escape hatch).
        assert_eq!(key_bytes("\x1b[<0;1;1M").unwrap(), b"\x1b[<0;1;1M");
    }

    #[test]
    fn unknown_errors() {
        assert!(key_bytes("Nope").is_err());
    }
}
