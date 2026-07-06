//! The rendered-screen model: a grid of styled [`Cell`]s.
//!
//! Two producers feed the rasterizer:
//! - [`parse_grid`] turns `tmux capture-pane -e -p -N` output (ANSI/SGR) into a
//!   grid — one captured terminal frame.
//! - [`card`] synthesizes a "silent-movie" title card — a solid panel with
//!   centered text — with no terminal involved.

use crate::color::Rgb;

/// One rendered screen cell: a character with its resolved RGB colours.
#[derive(Clone, Copy)]
pub struct Cell {
    pub ch: char,
    pub fg: Rgb,
    pub bg: Rgb,
    pub bold: bool,
}

impl Cell {
    fn blank(bg: Rgb) -> Self {
        Cell {
            ch: ' ',
            fg: bg,
            bg,
            bold: false,
        }
    }
}

// --- ANSI capture → grid ----------------------------------------------------

/// Classic VGA/xterm 16-colour palette, used to resolve SGR 30–37/40–47 and the
/// low 16 of the 256-colour cube. Truecolour apps emit `38;2;r;g;b` directly and
/// never touch this; it only backs the rare 16/256-indexed cell.
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

#[derive(Clone, Copy, PartialEq)]
enum Col {
    Default,
    Rgb(u8, u8, u8),
}

fn palette16(i: u8) -> Col {
    let (r, g, b) = PALETTE16[(i & 0x0f) as usize];
    Col::Rgb(r, g, b)
}

/// xterm 256-colour index → RGB.
fn xterm256(i: u8) -> Col {
    match i {
        0..=15 => palette16(i),
        16..=231 => {
            let i = i - 16;
            let steps = [0u8, 95, 135, 175, 215, 255];
            Col::Rgb(
                steps[(i / 36) as usize],
                steps[((i / 6) % 6) as usize],
                steps[(i % 6) as usize],
            )
        }
        232..=255 => {
            let v = 8 + 10 * (i - 232);
            Col::Rgb(v, v, v)
        }
    }
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

/// Build a `cols × rows` "silent-movie" title card: a solid `bg` panel with the
/// given `lines` of text centered (both axes), drawn in `fg`, optionally inside a
/// double-line frame (the classic intertitle border). Longer-than-width lines are
/// clipped from the right. Rendered by the same rasterizer as a captured frame,
/// so cards drop straight into a sequence.
pub fn card(
    cols: u32,
    rows: u32,
    lines: &[String],
    fg: Rgb,
    bg: Rgb,
    bold: bool,
    border: bool,
) -> Vec<Vec<Cell>> {
    let (cols_u, rows_u) = (cols as usize, rows as usize);
    let mut grid: Vec<Vec<Cell>> = (0..rows_u).map(|_| vec![Cell::blank(bg); cols_u]).collect();
    let mut put = |x: usize, y: usize, ch: char| {
        if x < cols_u && y < rows_u {
            grid[y][x] = Cell { ch, fg, bg, bold };
        }
    };

    // Double-line frame, inset from the edge. The horizontal inset is larger than
    // the vertical one so the visual margin is roughly even (cells are ~2× taller
    // than wide). Only drawn when the card is big enough to hold it.
    if border && cols_u >= 7 && rows_u >= 5 {
        let (mx, my) = (2usize, 1usize);
        let (x0, x1) = (mx, cols_u - 1 - mx);
        let (y0, y1) = (my, rows_u - 1 - my);
        put(x0, y0, '╔');
        put(x1, y0, '╗');
        put(x0, y1, '╚');
        put(x1, y1, '╝');
        for x in (x0 + 1)..x1 {
            put(x, y0, '═');
            put(x, y1, '═');
        }
        for y in (y0 + 1)..y1 {
            put(x0, y, '║');
            put(x1, y, '║');
        }
    }

    // Center the block of text lines (both axes) within the whole card.
    let n = lines.len();
    let top = rows_u.saturating_sub(n) / 2;
    for (i, line) in lines.iter().enumerate() {
        let y = top + i;
        let chars: Vec<char> = line.chars().collect();
        let left = cols_u.saturating_sub(chars.len()) / 2;
        for (j, &ch) in chars.iter().enumerate() {
            put(left + j, y, ch);
        }
    }
    grid
}

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

    #[test]
    fn card_centers_text() {
        // Borderless (card too small for a frame anyway).
        let g = card(
            10,
            3,
            &["hi".to_string()],
            (255, 255, 255),
            (0, 0, 0),
            false,
            false,
        );
        assert_eq!(g.len(), 3);
        assert_eq!(g[0].len(), 10);
        // Single line → vertically centered on row 1; "hi" (width 2) centered → cols 4,5.
        assert_eq!(g[1][4].ch, 'h');
        assert_eq!(g[1][5].ch, 'i');
        assert_eq!(g[1][0].ch, ' ');
        assert_eq!(g[1][0].bg, (0, 0, 0));
    }

    #[test]
    fn card_draws_double_frame() {
        let g = card(
            20,
            7,
            &["ok".to_string()],
            (255, 255, 255),
            (0, 0, 0),
            false,
            true,
        );
        // Frame inset by (mx=2, my=1): corners at (2,1),(17,1),(2,5),(17,5).
        assert_eq!(g[1][2].ch, '╔');
        assert_eq!(g[1][17].ch, '╗');
        assert_eq!(g[5][2].ch, '╚');
        assert_eq!(g[5][17].ch, '╝');
        assert_eq!(g[1][9].ch, '═');
        assert_eq!(g[3][2].ch, '║');
    }
}
