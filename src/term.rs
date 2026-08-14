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
    /// When the last input was written, and whether the child has produced any
    /// output since. Together they let `settle` tell a child that has *finished*
    /// answering from one that has not *started*.
    last_send: Instant,
    awaiting_reply: bool,
    eof: bool,
    /// Bumped on every `parser.process`. Lets a reader skip grid conversion
    /// when nothing has arrived since it last looked.
    generation: u64,
    /// Diagnostics: bytes read, and reads performed, since the last input was
    /// sent. A settle that returns with `reads_since_send == 0` captured a
    /// screen the child had not yet answered onto.
    bytes_since_send: u64,
    reads_since_send: u32,
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
    // Debug aid: `ANSIDRAMA_DUMP_PTY=<path>` tees every byte the child writes,
    // so a suspect repaint can be replayed through the parser offline.
    let mut dump =
        std::env::var_os("ANSIDRAMA_DUMP_PTY").and_then(|p| std::fs::File::create(p).ok());
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
                if let Some(f) = dump.as_mut() {
                    let _ = f.write_all(&buf[..n]);
                }
                let mut s = lock.lock().unwrap();
                s.parser.process(&buf[..n]);
                s.generation += 1;
                s.last_activity = Instant::now();
                s.awaiting_reply = false;
                s.bytes_since_send += n as u64;
                s.reads_since_send += 1;
                cvar.notify_all();
            }
        }
    }
}

/// Debug aid: `ANSIDRAMA_TRACE=1` writes one line per `settle` to stderr —
/// why the wait ended, how long it took, and how much the child had said since
/// the input was sent. `reads=0` means the capture that follows shows a screen
/// the child never answered onto.
fn trace_settle(why: &str, start: Instant, s: &Shared) {
    if std::env::var_os("ANSIDRAMA_TRACE").is_none() {
        return;
    }
    eprintln!(
        "settle {why:>4} after {:>6.1}ms  since_send={:>6.1}ms  reads={:<3} bytes={}",
        start.elapsed().as_secs_f32() * 1000.0,
        s.last_send.elapsed().as_secs_f32() * 1000.0,
        s.reads_since_send,
        s.bytes_since_send,
    );
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
                last_send: Instant::now(),
                awaiting_reply: false,
                eof: false,
                generation: 0,
                bytes_since_send: 0,
                reads_since_send: 0,
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

    /// Block until the child has answered the last input and the PTY has then
    /// been idle for `idle` — or `cap` total elapses, or the child exits.
    ///
    /// A quiet PTY has two meanings, and they need different answers: the child
    /// has *finished* drawing, or it has not *started*. Idle alone reads both as
    /// "done", so an input the child is slow to answer — a theme switch that
    /// re-renders a whole document, a click tmux takes a moment over — is
    /// captured as the screen from before it, and the change surfaces one frame
    /// late (or, if the answer is split across the window, half-drawn: a status
    /// bar naming a theme the screen is not wearing).
    ///
    /// So while an input is outstanding and unanswered, `idle` cannot end the
    /// wait: the child gets up to `react` to produce its first byte. Once any
    /// byte arrives the ordinary idle rule takes over, which costs a responsive
    /// child nothing. `react` is only spent in full by an input that draws
    /// nothing at all (a no-op key, a mouse release), so keep `cap` above
    /// `react + idle` or it will cut the wait short again.
    pub fn settle(&mut self, react: Duration, idle: Duration, cap: Duration) {
        let (lock, cvar) = &*self.shared;
        let start = Instant::now();
        let mut s = lock.lock().unwrap();
        loop {
            if s.eof {
                trace_settle("eof", start, &s);
                return;
            }
            if start.elapsed() >= cap {
                trace_settle("cap", start, &s);
                return;
            }
            // An unanswered input still inside its react window: keep waiting
            // even though the PTY is quiet — the quiet is the child thinking.
            let react_left = react.saturating_sub(s.last_send.elapsed());
            let awaiting = !react_left.is_zero();
            if !awaiting && s.last_activity.elapsed() >= idle {
                trace_settle("idle", start, &s);
                return;
            }
            let until_idle = idle.saturating_sub(s.last_activity.elapsed());
            let wait = if awaiting {
                react_left.max(until_idle)
            } else {
                until_idle
            }
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
            let now = Instant::now();
            s.last_activity = now;
            s.last_send = now;
            s.awaiting_reply = true;
            s.bytes_since_send = 0;
            s.reads_since_send = 0;
            cvar.notify_all();
        }
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
    shared: Arc<(Mutex<Shared>, Condvar)>,
    rows: u16,
    cols: u16,
}

impl ParserHandle {
    pub fn generation(&self) -> u64 {
        let (lock, _) = &*self.shared;
        lock.lock().unwrap().generation
    }

    /// Grid, caret and generation from a single lock acquisition, so the three
    /// can never disagree about which screen they describe.
    pub fn snapshot(&self) -> (Vec<Vec<Cell>>, Option<(u32, u32)>, u64) {
        let (lock, _) = &*self.shared;
        let s = lock.lock().unwrap();
        (
            screen_to_grid(s.parser.screen(), self.rows, self.cols),
            screen_caret(s.parser.screen()),
            s.generation,
        )
    }

    pub fn is_eof(&self) -> bool {
        let (lock, _) = &*self.shared;
        lock.lock().unwrap().eof
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
    /// passes. Timing-dependent capture is inherently racy under parallel load
    /// (a single fixed `settle` window can return before the child's first
    /// output), so tests wait for the expected content rather than one window.
    fn wait_for_row0(term: &mut Term, needle: &str) -> String {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            term.settle(
                Duration::ZERO,
                Duration::from_millis(100),
                Duration::from_millis(500),
            );
            let row0: String = term.grid()[0].iter().map(|c| c.ch).collect();
            if row0.contains(needle) || Instant::now() >= deadline {
                return row0;
            }
        }
    }

    #[test]
    fn captures_printed_text() {
        let env = BTreeMap::new();
        // Print, then sleep so the child is still alive while we capture.
        let mut term = Term::spawn(20, 5, "printf 'HELLO'; sleep 2", &env).unwrap();
        let row0 = wait_for_row0(&mut term, "HELLO");
        assert!(row0.starts_with("HELLO"), "row0 = {row0:?}");
        let grid = term.grid();
        assert_eq!(grid.len(), 5);
        assert_eq!(grid[0].len(), 20);
    }

    /// A key whose answer starts *later* than `idle` must not be captured early.
    ///
    /// `settle` cannot tell "the app has finished answering" from "the app has
    /// not started answering yet" — both look like a quiet PTY. Without a floor
    /// on the wait for the first reply, the capture returns the pre-keystroke
    /// screen and the change shows up one scene late.
    #[test]
    fn settle_waits_for_a_late_first_reply() {
        let env = BTreeMap::new();
        // Echo off, so the keystroke itself is not the "reply": the app goes
        // quiet for 600ms — twice `idle` — before it prints anything.
        // `READY` is printed *after* echo is off, and waited for: sending the key
        // before the child reaches `stty` would let the tty echo it, and that
        // echo — not the app — would be the reply this test is about.
        let mut term = Term::spawn(
            20,
            3,
            "stty -echo; printf 'READY'; read -n1 k; sleep 0.6; printf 'LATE'; sleep 2",
            &env,
        )
        .unwrap();
        let row0 = wait_for_row0(&mut term, "READY");
        assert!(row0.contains("READY"), "child never started: {row0:?}");
        term.send_key("x").unwrap();
        term.settle(
            Duration::from_millis(2000), // react: wait for the app to start
            Duration::from_millis(100),  // idle
            Duration::from_millis(5000), // cap
        );
        let row0: String = term.grid()[0].iter().map(|c| c.ch).collect();
        assert!(
            row0.contains("LATE"),
            "captured before the app answered: {row0:?}"
        );
    }

    /// Output belonging to the *previous* input must not count as this input's
    /// reply.
    ///
    /// `read_loop` clears `awaiting_reply` on any byte. When the previous input's
    /// repaint is still draining as the next input is sent, those leftover bytes
    /// satisfy the react condition, `idle` takes over, and the quiet gap before
    /// the real answer ends the wait early — the capture shows the previous
    /// input's screen. Raising `react` cannot help: react is not waited out, it
    /// is *satisfied*.
    #[test]
    fn settle_is_not_disarmed_by_the_previous_inputs_output() {
        let env = BTreeMap::new();
        // Key 1 answers late (TRAIL, ~400ms) via a background job, so its output
        // lands *after* key 2 has been sent. Key 2 answers later still (LATE).
        let mut term = Term::spawn(
            20,
            3,
            "stty -echo; printf 'READY'; \
             read -n1 a; (sleep 0.4; printf 'TRAIL') & \
             read -n1 b; sleep 1.2; printf 'LATE'; sleep 3",
            &env,
        )
        .unwrap();
        let row0 = wait_for_row0(&mut term, "READY");
        assert!(row0.contains("READY"), "child never started: {row0:?}");

        // Key 1: return before its trailing output arrives.
        term.send_key("a").unwrap();
        term.settle(
            Duration::ZERO,
            Duration::from_millis(50),
            Duration::from_millis(200),
        );

        // Key 2: its reply is LATE. TRAIL (key 1's) arrives first and must not
        // be mistaken for it.
        term.send_key("b").unwrap();
        term.settle(
            Duration::from_millis(2000), // react
            Duration::from_millis(350),  // idle
            Duration::from_millis(5000), // cap
        );
        let row0: String = term.grid()[0].iter().map(|c| c.ch).collect();
        assert!(
            row0.contains("LATE"),
            "previous input's output disarmed the react window: {row0:?}"
        );
    }

    #[test]
    fn send_key_reaches_app() {
        let env = BTreeMap::new();
        // `read -n1 k` echoes the key we send back onto the screen.
        let mut term = Term::spawn(20, 3, "read -n1 k; printf \"got:$k\"; sleep 2", &env).unwrap();
        term.settle(
            Duration::ZERO,
            Duration::from_millis(100),
            Duration::from_millis(500),
        );
        term.send_key("x").unwrap();
        let row0 = wait_for_row0(&mut term, "got:x");
        assert!(row0.contains("got:x"), "row0 = {row0:?}");
    }

    /// The generation counter must advance only when the child actually writes,
    /// so the sampler can skip grid conversion on an idle screen.
    #[test]
    fn generation_advances_only_on_output() {
        let env = BTreeMap::new();
        let mut term = Term::spawn(20, 3, "printf 'READY'; sleep 2", &env).unwrap();
        let h = term.handle();
        let _ = wait_for_row0(&mut term, "READY");

        let (grid, _caret, g1) = h.snapshot();
        let row0: String = grid[0].iter().map(|c| c.ch).collect();
        assert!(row0.contains("READY"), "row0 = {row0:?}");
        assert!(g1 > 0, "generation should have advanced past 0");

        // Nothing more is written for a while: the counter must hold still.
        std::thread::sleep(Duration::from_millis(200));
        assert_eq!(h.generation(), g1, "generation moved with no output");
    }
}
