//! In-process terminal for `record`: spawn the child on a PTY, feed its output
//! to a `vt100` parser, and read the screen grid + caret directly — no tmux.
//! The screen→grid conversion is kept as free functions so it is unit-testable
//! without a real PTY.

use crate::color::vt_color;
use crate::grid::Cell;

/// Default fg/bg for cells the app left unstyled (mirrors `grid::parse_grid`).
const DEF_FG: crate::color::Rgb = (170, 170, 170);
const DEF_BG: crate::color::Rgb = (0, 0, 0);

/// Convert a `vt100` screen into ansidrama's grid. Always emits exactly `rows`
/// rows of `cols` cells. Wide-character continuation cells become spaces so
/// columns stay aligned.
pub fn screen_to_grid(screen: &vt100::Screen, rows: u16, cols: u16) -> Vec<Vec<Cell>> {
    let mut out = Vec::with_capacity(rows as usize);
    for r in 0..rows {
        let mut row = Vec::with_capacity(cols as usize);
        for c in 0..cols {
            let (ch, mut fg, mut bg, bold, inverse) = match screen.cell(r, c) {
                Some(cell) if cell.is_wide_continuation() => (
                    ' ',
                    vt_color(cell.fgcolor(), DEF_FG),
                    vt_color(cell.bgcolor(), DEF_BG),
                    false,
                    cell.inverse(),
                ),
                Some(cell) => (
                    cell.contents().chars().next().unwrap_or(' '),
                    vt_color(cell.fgcolor(), DEF_FG),
                    vt_color(cell.bgcolor(), DEF_BG),
                    cell.bold(),
                    cell.inverse(),
                ),
                None => (' ', DEF_FG, DEF_BG, false, false),
            };
            if inverse {
                std::mem::swap(&mut fg, &mut bg);
            }
            row.push(Cell { ch, fg, bg, bold });
        }
        out.push(row);
    }
    out
}

/// The app's text caret as `(x, y)` 0-based, or `None` when hidden.
pub fn screen_caret(screen: &vt100::Screen) -> Option<(u32, u32)> {
    if screen.hide_cursor() {
        return None;
    }
    let (row, col) = screen.cursor_position();
    Some((col as u32, row as u32))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(rows: u16, cols: u16, bytes: &[u8]) -> vt100::Parser {
        let mut p = vt100::Parser::new(rows, cols, 0);
        p.process(bytes);
        p
    }

    #[test]
    fn truecolor_and_bold() {
        let p = parse(2, 10, b"\x1b[1m\x1b[38;2;10;20;30mX");
        let g = screen_to_grid(p.screen(), 2, 10);
        assert_eq!(g.len(), 2);
        assert_eq!(g[0].len(), 10);
        assert_eq!(g[0][0].ch, 'X');
        assert_eq!(g[0][0].fg, (10, 20, 30));
        assert!(g[0][0].bold);
    }

    #[test]
    fn indexed_color_resolves() {
        let p = parse(1, 4, b"\x1b[38;5;9mR");
        let g = screen_to_grid(p.screen(), 1, 4);
        assert_eq!(g[0][0].fg, (255, 85, 85));
    }

    #[test]
    fn inverse_swaps_fg_bg() {
        let p = parse(1, 4, b"\x1b[38;2;9;9;9m\x1b[48;2;1;1;1m\x1b[7mR");
        let g = screen_to_grid(p.screen(), 1, 4);
        assert_eq!(g[0][0].fg, (1, 1, 1));
        assert_eq!(g[0][0].bg, (9, 9, 9));
    }

    #[test]
    fn caret_position_then_hidden() {
        let p = parse(3, 10, b"ab");
        assert_eq!(screen_caret(p.screen()), Some((2, 0))); // (x=col, y=row)
        let p = parse(3, 10, b"ab\x1b[?25l");
        assert_eq!(screen_caret(p.screen()), None); // cursor hidden
    }
}
