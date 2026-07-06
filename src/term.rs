//! In-process terminal for `record`: spawn the child on a PTY, feed its output
//! to a `vt100` parser, and read the screen grid + caret directly — no tmux.
//! The screen→grid conversion is kept as free functions so it is unit-testable
//! without a real PTY.

use crate::color::vt_color;
use crate::grid::Cell;

use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::os::fd::AsFd;
use std::os::unix::process::CommandExt;
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Condvar, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};

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

/// Parser state shared between the reader thread and the caller.
struct Shared {
    parser: vt100::Parser,
    last_activity: Instant,
    eof: bool,
}

/// An embedded terminal: a child on a PTY, plus a `vt100` parser fed by a
/// background reader thread. Dropping it reaps the child and joins the reader.
pub struct Term {
    shared: Arc<(Mutex<Shared>, Condvar)>,
    master: std::fs::File,
    child: Child,
    reader: Option<JoinHandle<()>>,
    cols: u16,
    rows: u16,
}

/// Read the PTY master until EOF, feeding bytes to the parser and stamping
/// `last_activity` so `settle` can detect quiescence.
fn read_loop(mut master: std::fs::File, shared: Arc<(Mutex<Shared>, Condvar)>) {
    let mut buf = [0u8; 8192];
    loop {
        match master.read(&mut buf) {
            Ok(0) | Err(_) => {
                let (lock, cvar) = &*shared;
                let mut s = lock.lock().unwrap();
                s.eof = true;
                cvar.notify_all();
                return;
            }
            Ok(n) => {
                let (lock, cvar) = &*shared;
                let mut s = lock.lock().unwrap();
                s.parser.process(&buf[..n]);
                s.last_activity = Instant::now();
                cvar.notify_all();
            }
        }
    }
}

impl Term {
    pub fn spawn(
        cols: u16,
        rows: u16,
        launch: &str,
        env: &BTreeMap<String, String>,
    ) -> Result<Term> {
        let ws = rustix::termios::Winsize {
            ws_row: rows,
            ws_col: cols,
            ws_xpixel: 0,
            ws_ypixel: 0,
        };
        let pty = rustix_openpty::openpty(None, Some(&ws)).context("openpty")?;
        let controller = pty.controller; // master
        let user = pty.user; // slave (controlling terminal for the child)

        // The child's stdio are three dups of the slave.
        let stdin = user.try_clone().context("dup pty slave")?;
        let stdout = user.try_clone().context("dup pty slave")?;
        let stderr = user.try_clone().context("dup pty slave")?;

        let mut cmd = Command::new("bash");
        cmd.args(["-lc", launch]);
        cmd.env("TERM", "xterm-256color");
        cmd.env("COLORTERM", "truecolor");
        for (k, v) in env {
            cmd.env(k, v);
        }
        cmd.stdin(Stdio::from(stdin));
        cmd.stdout(Stdio::from(stdout));
        cmd.stderr(Stdio::from(stderr));

        // After fork, put the child in its own session and make the pty its
        // controlling terminal. `user` (CLOEXEC) is closed at exec in the child.
        let ctty = user;
        unsafe {
            cmd.pre_exec(move || {
                rustix::process::setsid().map_err(std::io::Error::from)?;
                rustix::process::ioctl_tiocsctty(ctty.as_fd()).map_err(std::io::Error::from)?;
                Ok(())
            });
        }
        let child = cmd.spawn().context("spawn bash")?;
        drop(cmd); // drops the pre_exec closure → closes the parent's slave copy

        let master = std::fs::File::from(controller);
        let reader_master = master.try_clone().context("dup pty master")?;

        let shared = Arc::new((
            Mutex::new(Shared {
                parser: vt100::Parser::new(rows, cols, 0),
                last_activity: Instant::now(),
                eof: false,
            }),
            Condvar::new(),
        ));
        let reader = {
            let shared = Arc::clone(&shared);
            std::thread::spawn(move || read_loop(reader_master, shared))
        };

        Ok(Term {
            shared,
            master,
            child,
            reader: Some(reader),
            cols,
            rows,
        })
    }

    /// Block until the PTY has been idle for `idle`, or `cap` total elapses,
    /// or the child exits.
    pub fn settle(&mut self, idle: Duration, cap: Duration) {
        let (lock, cvar) = &*self.shared;
        let start = Instant::now();
        let mut s = lock.lock().unwrap();
        loop {
            if s.eof {
                return;
            }
            if s.last_activity.elapsed() >= idle {
                return;
            }
            if start.elapsed() >= cap {
                return;
            }
            let wait = (idle - s.last_activity.elapsed())
                .min(cap.saturating_sub(start.elapsed()))
                .max(Duration::from_millis(1));
            s = cvar.wait_timeout(s, wait).unwrap().0;
        }
    }

    pub fn grid(&self) -> Vec<Vec<Cell>> {
        let (lock, _) = &*self.shared;
        let s = lock.lock().unwrap();
        screen_to_grid(s.parser.screen(), self.rows, self.cols)
    }

    pub fn caret(&self) -> Option<(u32, u32)> {
        let (lock, _) = &*self.shared;
        let s = lock.lock().unwrap();
        screen_caret(s.parser.screen())
    }

    pub fn send_bytes(&mut self, bytes: &[u8]) -> Result<()> {
        self.master.write_all(bytes).context("write to pty")?;
        self.master.flush().ok();
        // Count our own write as activity: without this, a `settle()` called
        // right after `send_bytes` can see a `last_activity` timestamp that's
        // already stale from *before* the send (the previous `settle()` only
        // returns once things have been quiet for `idle`), so its very first
        // check would immediately conclude "idle" and return before the
        // child has had any chance to react to what we just sent.
        {
            let (lock, cvar) = &*self.shared;
            let mut s = lock.lock().unwrap();
            s.last_activity = Instant::now();
            cvar.notify_all();
        }
        Ok(())
    }

    pub fn send_key(&mut self, name: &str) -> Result<()> {
        let bytes = crate::keys::key_bytes(name)?;
        self.send_bytes(&bytes)
    }
}

impl Drop for Term {
    fn drop(&mut self) {
        // Kill + reap the child first; that closes the slave, so the reader
        // hits EOF and its thread exits, which we then join.
        let _ = self.child.kill();
        let _ = self.child.wait();
        if let Some(h) = self.reader.take() {
            let _ = h.join();
        }
    }
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

#[cfg(all(test, unix))]
mod pty_tests {
    use super::*;
    use std::collections::BTreeMap;
    use std::time::Duration;

    #[test]
    fn captures_printed_text() {
        let env = BTreeMap::new();
        // Print, then sleep so the child is still alive while we capture.
        let mut term = Term::spawn(20, 5, "printf 'HELLO'; sleep 2", &env).unwrap();
        term.settle(Duration::from_millis(150), Duration::from_millis(2000));
        let grid = term.grid();
        let row0: String = grid[0].iter().map(|c| c.ch).collect();
        assert!(row0.starts_with("HELLO"), "row0 = {row0:?}");
        assert_eq!(grid.len(), 5);
        assert_eq!(grid[0].len(), 20);
    }

    #[test]
    fn send_key_reaches_app() {
        let env = BTreeMap::new();
        // `read -n1 k` echoes the key we send back onto the screen.
        let mut term = Term::spawn(20, 3, "read -n1 k; printf \"got:$k\"; sleep 2", &env).unwrap();
        term.settle(Duration::from_millis(150), Duration::from_millis(1000));
        term.send_key("x").unwrap();
        term.settle(Duration::from_millis(150), Duration::from_millis(2000));
        let row0: String = term.grid()[0].iter().map(|c| c.ch).collect();
        assert!(row0.contains("got:x"), "row0 = {row0:?}");
    }
}
