//! The rendered-screen model: a grid of styled [`Cell`]s.
//!
//! Two producers feed the rasterizer:
//! - [`parse_grid`] turns `tmux capture-pane -e -p -N` output (ANSI/SGR) into a
//!   grid — one captured terminal frame.
//! - [`card`] synthesizes a "silent-movie" title card — a solid panel with
//!   centered text — with no terminal involved.

use crate::color::Rgb;

/// One rendered screen cell: a character with its resolved RGB colours.
#[derive(Clone, Copy, PartialEq)]
pub struct Cell {
    pub ch: char,
    pub fg: Rgb,
    pub bg: Rgb,
    pub bold: bool,
}

// --- ANSI capture → grid ----------------------------------------------------

#[derive(Clone, Copy, PartialEq)]
enum Col {
    Default,
    Rgb(u8, u8, u8),
}

fn palette16(i: u8) -> Col {
    let (r, g, b) = crate::color::index_to_rgb(i & 0x0f);
    Col::Rgb(r, g, b)
}

/// xterm 256-colour index → RGB.
fn xterm256(i: u8) -> Col {
    let (r, g, b) = crate::color::index_to_rgb(i);
    Col::Rgb(r, g, b)
}

#[derive(Clone, Copy, PartialEq)]
struct Sgr {
    fg: Col,
    bg: Col,
    bold: bool,
    reverse: bool,
}

impl Sgr {
    fn reset() -> Self {
        Sgr {
            fg: Col::Default,
            bg: Col::Default,
            bold: false,
            reverse: false,
        }
    }
}

/// Apply one CSI `…m` parameter list to the running SGR state.
fn apply_sgr(state: &mut Sgr, params: &[i64]) {
    let mut it = params.iter().copied().peekable();
    while let Some(p) = it.next() {
        match p {
            0 => *state = Sgr::reset(),
            1 => state.bold = true,
            22 => state.bold = false,
            7 => state.reverse = true,
            27 => state.reverse = false,
            30..=37 => state.fg = palette16((p - 30) as u8),
            90..=97 => state.fg = palette16((p - 90 + 8) as u8),
            39 => state.fg = Col::Default,
            40..=47 => state.bg = palette16((p - 40) as u8),
            100..=107 => state.bg = palette16((p - 100 + 8) as u8),
            49 => state.bg = Col::Default,
            38 | 48 => {
                let target_fg = p == 38;
                match it.next() {
                    Some(5) => {
                        if let Some(n) = it.next() {
                            let c = xterm256(n as u8);
                            if target_fg {
                                state.fg = c
                            } else {
                                state.bg = c
                            }
                        }
                    }
                    Some(2) => {
                        let r = it.next().unwrap_or(0) as u8;
                        let g = it.next().unwrap_or(0) as u8;
                        let b = it.next().unwrap_or(0) as u8;
                        let c = Col::Rgb(r, g, b);
                        if target_fg {
                            state.fg = c
                        } else {
                            state.bg = c
                        }
                    }
                    _ => {}
                }
            }
            _ => {}
        }
    }
}

fn resolve(c: Col, default_rgb: Rgb) -> Rgb {
    match c {
        Col::Rgb(r, g, b) => (r, g, b),
        Col::Default => default_rgb,
    }
}

/// Parse ANSI/SGR text (from `tmux capture-pane -e -p -N`) into a grid of cells,
/// one inner `Vec` per screen row. `Col::Default` resolves to light-grey on black
/// — a TUI paints real colours, so the defaults only show through where the
/// terminal emitted no SGR (e.g. an unstyled shell prompt).
pub fn parse_grid(input: &str) -> Vec<Vec<Cell>> {
    const DEF_FG: Rgb = (170, 170, 170);
    const DEF_BG: Rgb = (0, 0, 0);
    let mut rows: Vec<Vec<Cell>> = vec![Vec::new()];
    let mut state = Sgr::reset();
    let mut chars = input.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\x1b' {
            if chars.peek() == Some(&'[') {
                chars.next();
                let mut buf = String::new();
                let mut final_byte = None;
                for c in chars.by_ref() {
                    if c.is_ascii_alphabetic() {
                        final_byte = Some(c);
                        break;
                    }
                    buf.push(c);
                }
                if final_byte == Some('m') {
                    let params: Vec<i64> = if buf.is_empty() {
                        vec![0]
                    } else {
                        buf.split(';')
                            .map(|s| s.parse::<i64>().unwrap_or(0))
                            .collect()
                    };
                    apply_sgr(&mut state, &params);
                }
            }
            continue;
        }
        if ch == '\r' {
            continue;
        }
        if ch == '\n' {
            rows.push(Vec::new());
            continue;
        }
        let (mut fg, mut bg) = (state.fg, state.bg);
        if state.reverse {
            std::mem::swap(&mut fg, &mut bg);
        }
        rows.last_mut().unwrap().push(Cell {
            ch,
            fg: resolve(fg, DEF_FG),
            bg: resolve(bg, DEF_BG),
            bold: state.bold,
        });
    }
    if rows.last().is_some_and(|r| r.is_empty()) {
        rows.pop();
    }
    rows
}

// --- synthetic title card ---------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truecolor_bg_and_fg_resolve() {
        let g = parse_grid("\x1b[38;2;10;20;30m\x1b[48;2;1;2;3mX");
        assert_eq!(g[0][0].ch, 'X');
        assert_eq!(g[0][0].fg, (10, 20, 30));
        assert_eq!(g[0][0].bg, (1, 2, 3));
    }

    #[test]
    fn reverse_swaps_fg_bg() {
        let g = parse_grid("\x1b[38;2;9;9;9m\x1b[48;2;1;1;1m\x1b[7mR");
        assert_eq!(g[0][0].fg, (1, 1, 1));
        assert_eq!(g[0][0].bg, (9, 9, 9));
    }

    #[test]
    fn newlines_make_rows() {
        let g = parse_grid("ab\ncd");
        assert_eq!(g.len(), 2);
        assert_eq!(g[0].len(), 2);
        assert_eq!(g[1][1].ch, 'd');
    }
}
