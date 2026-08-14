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
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

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
    eof: bool,
    /// Bumped on every `parser.process`. Lets a reader skip grid conversion
    /// when nothing has arrived since it last looked.
    generation: u64,
}

/// An embedded terminal: a child on a PTY, plus a `vt100` parser fed by a
/// background reader thread. Dropping it reaps the child and joins the reader.
pub struct Term {
    shared: Arc<Mutex<Shared>>,
    master: std::fs::File,
    child: Child,
    reader: Option<JoinHandle<()>>,
    cols: u16,
    rows: u16,
}

/// Read the PTY master until EOF, feeding bytes to the parser and bumping the
/// generation counter so the sampler knows when there is something new to read.
fn read_loop(mut master: std::fs::File, shared: Arc<Mutex<Shared>>) {
    // Debug aid: `ANSIDRAMA_DUMP_PTY=<path>` tees every byte the child writes,
    // so a suspect repaint can be replayed through the parser offline.
    let mut dump =
        std::env::var_os("ANSIDRAMA_DUMP_PTY").and_then(|p| std::fs::File::create(p).ok());
    let mut buf = [0u8; 8192];
    loop {
        match master.read(&mut buf) {
            Ok(0) | Err(_) => {
                shared.lock().unwrap().eof = true;
                return;
            }
            Ok(n) => {
                if let Some(f) = dump.as_mut() {
                    let _ = f.write_all(&buf[..n]);
                }
                let mut s = shared.lock().unwrap();
                s.parser.process(&buf[..n]);
                s.generation += 1;
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

        let shared = Arc::new(Mutex::new(Shared {
            parser: vt100::Parser::new(rows, cols, 0),
            eof: false,
            generation: 0,
        }));
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

    pub fn grid(&self) -> Vec<Vec<Cell>> {
        let s = self.shared.lock().unwrap();
        screen_to_grid(s.parser.screen(), self.rows, self.cols)
    }

    pub fn caret(&self) -> Option<(u32, u32)> {
        let s = self.shared.lock().unwrap();
        screen_caret(s.parser.screen())
    }

    pub fn send_bytes(&mut self, bytes: &[u8]) -> Result<()> {
        self.master.write_all(bytes).context("write to pty")?;
        self.master.flush().ok();
        Ok(())
    }

    pub fn send_key(&mut self, name: &str) -> Result<()> {
        let bytes = crate::keys::key_bytes(name)?;
        self.send_bytes(&bytes)
    }

    pub fn handle(&self) -> ParserHandle {
        ParserHandle {
            shared: Arc::clone(&self.shared),
            rows: self.rows,
            cols: self.cols,
        }
    }
}

/// A cloneable read-only view of the parser, for the sampler thread. It
/// deliberately does not expose the child or the master fd: sampling must never
/// be able to drive the terminal.
#[derive(Clone)]
pub struct ParserHandle {
    shared: Arc<Mutex<Shared>>,
    rows: u16,
    cols: u16,
}

impl ParserHandle {
    pub fn generation(&self) -> u64 {
        self.shared.lock().unwrap().generation
    }

    /// Grid, caret and generation from a single lock acquisition, so the three
    /// can never disagree about which screen they describe.
    pub fn snapshot(&self) -> (Vec<Vec<Cell>>, Option<(u32, u32)>, u64) {
        let s = self.shared.lock().unwrap();
        (
            screen_to_grid(s.parser.screen(), self.rows, self.cols),
            screen_caret(s.parser.screen()),
            s.generation,
        )
    }

    pub fn is_eof(&self) -> bool {
        self.shared.lock().unwrap().eof
    }
}

impl Drop for Term {
    fn drop(&mut self) {
        // The child is its own session/process-group leader (setsid in spawn),
        // so kill the whole group — otherwise a backgrounded grandchild could
        // keep the PTY slave open and block the reader thread's EOF forever.
        // The group is isolated (setsid), so this can never touch our own group.
        if let Some(pid) = rustix::process::Pid::from_raw(self.child.id() as i32) {
            let _ = rustix::process::kill_process_group(pid, rustix::process::Signal::KILL);
        }
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
    use std::time::{Duration, Instant};

    /// Poll the terminal until row 0 contains `needle`, or a generous deadline
    /// passes. Timing-dependent capture is inherently racy under parallel load,
    /// so tests wait for the expected content rather than for a fixed window.
    fn wait_for_row0(term: &Term, needle: &str) -> String {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            let row0: String = term.grid()[0].iter().map(|c| c.ch).collect();
            if row0.contains(needle) || Instant::now() >= deadline {
                return row0;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    #[test]
    fn captures_printed_text() {
        let env = BTreeMap::new();
        // Print, then sleep so the child is still alive while we capture.
        let term = Term::spawn(20, 5, "printf 'HELLO'; sleep 2", &env).unwrap();
        let row0 = wait_for_row0(&term, "HELLO");
        assert!(row0.starts_with("HELLO"), "row0 = {row0:?}");
        let grid = term.grid();
        assert_eq!(grid.len(), 5);
        assert_eq!(grid[0].len(), 20);
    }

    #[test]
    fn send_key_reaches_app() {
        let env = BTreeMap::new();
        // `read -n1 k` echoes the key we send back onto the screen. Wait for
        // the child to announce it has reached the `read` before sending:
        // without that synchronisation the test would depend on the pty
        // buffering our key until bash gets there.
        let mut term = Term::spawn(
            20,
            3,
            "printf 'READY'; read -n1 k; printf \"\\rgot:$k\"; sleep 2",
            &env,
        )
        .unwrap();
        let row0 = wait_for_row0(&term, "READY");
        assert!(row0.contains("READY"), "child never started: {row0:?}");
        term.send_key("x").unwrap();
        let row0 = wait_for_row0(&term, "got:x");
        assert!(row0.contains("got:x"), "row0 = {row0:?}");
    }

    /// The generation counter must advance only when the child actually writes,
    /// so the sampler can skip grid conversion on an idle screen.
    #[test]
    fn generation_advances_only_on_output() {
        let env = BTreeMap::new();
        let term = Term::spawn(20, 3, "printf 'READY'; sleep 2", &env).unwrap();
        let h = term.handle();
        let _ = wait_for_row0(&term, "READY");

        let (grid, _caret, g1) = h.snapshot();
        let row0: String = grid[0].iter().map(|c| c.ch).collect();
        assert!(row0.contains("READY"), "row0 = {row0:?}");
        assert!(g1 > 0, "generation should have advanced past 0");

        // Nothing more is written for a while: the counter must hold still.
        std::thread::sleep(Duration::from_millis(200));
        assert_eq!(h.generation(), g1, "generation moved with no output");
    }
}
