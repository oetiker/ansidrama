//! Continuous grid sampling: a thread that snapshots the screen, and the
//! pending/committed rule that decides which snapshots are real states.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use anyhow::{bail, Result};

use crate::grid::Cell;
use crate::pattern::Pattern;
use crate::term::ParserHandle;

/// One screen the app actually held, and when it first appeared.
pub struct State {
    pub grid: Vec<Vec<Cell>>,
    pub caret: Option<(u32, u32)>,
    pub t: Instant,
}

/// Applies the persistence rule at capture time: a newly seen screen is
/// *pending* until it has survived `persist`, and only then is it *committed*.
/// A screen that is superseded sooner never enters the log at all — which is
/// what drops torn mid-repaint frames, and what bounds memory.
pub struct StateAccumulator {
    persist: Duration,
    committed: Vec<State>,
    pending: Option<State>,
    last_change: Instant,
    bytes: usize,
}

impl StateAccumulator {
    pub fn new(persist: Duration) -> StateAccumulator {
        StateAccumulator {
            persist,
            committed: Vec::new(),
            pending: None,
            last_change: Instant::now(),
            bytes: 0,
        }
    }

    pub fn observe(&mut self, grid: Vec<Vec<Cell>>, caret: Option<(u32, u32)>, now: Instant) {
        let newest = self.pending.as_ref().or_else(|| self.committed.last());
        // A "screen" is grid *and* caret together — a plain space keystroke
        // overwrites a blank cell with a blank cell, leaving the grid
        // byte-identical while only the caret moves. Comparing the grid alone
        // would call that "unchanged" and let the caret go stale in the
        // committed state, drawing the cursor one cell behind where the app
        // actually put it.
        let unchanged = newest.is_some_and(|s| s.grid == grid && s.caret == caret);
        if !unchanged {
            // A genuinely new screen. Whatever was pending never persisted.
            self.pending = Some(State { grid, caret, t: now });
            self.last_change = now;
            return;
        }
        // Unchanged. Promote the pending state once it has earned its place.
        self.tick(now);
    }

    /// Promote the pending state if it has now survived `persist`. This is the
    /// no-change tick: an idle screen must not cost a grid clone.
    pub fn tick(&mut self, now: Instant) {
        if let Some(p) = &self.pending {
            if now.duration_since(p.t) >= self.persist {
                let p = self.pending.take().expect("checked above");
                self.bytes += state_bytes(&p);
                self.committed.push(p);
            }
        }
    }

    pub fn force_commit(&mut self, _now: Instant) -> Option<usize> {
        let p = self.pending.take()?;
        self.bytes += state_bytes(&p);
        self.committed.push(p);
        Some(self.committed.len() - 1)
    }

    pub fn committed(&self) -> &[State] {
        &self.committed
    }

    /// The newest screen, pending or committed — what "the screen right now" means.
    pub fn newest(&self) -> Option<&State> {
        self.pending.as_ref().or_else(|| self.committed.last())
    }

    /// The index of the state a capture is settling on: the state it just
    /// force-committed, or — if the sampler thread already promoted it (the
    /// normal case whenever `stable >= persist`) — the last committed state.
    /// `Sampler::start` observes one state synchronously before any capture can
    /// run, so `committed()` being empty here would be that invariant broken,
    /// not a reachable outcome — made explicit rather than laundered into `0`.
    pub fn settled_index(&mut self, now: Instant) -> usize {
        match self.force_commit(now) {
            Some(idx) => idx,
            None => self.committed.len().checked_sub(1).unwrap_or_else(|| {
                unreachable!(
                    "Sampler::start observes one state before any wait() can run; \
                     committed() must not be empty here"
                )
            }),
        }
    }

    pub fn last_change(&self) -> Instant {
        self.last_change
    }
    pub fn bytes(&self) -> usize {
        self.bytes
    }
}

/// Approximate resident size of one state, for the `max_capture_mb` bound.
fn state_bytes(s: &State) -> usize {
    s.grid
        .iter()
        .map(|r| r.len() * std::mem::size_of::<Cell>() + std::mem::size_of::<Vec<Cell>>())
        .sum()
}

/// What ended a wait, and which state the capture should use.
#[derive(Debug)]
pub struct WaitOutcome {
    pub state: usize,
    /// The bound was reached rather than the screen settling. Not an error —
    /// a deliberately animated screen never settles — but D reports it.
    pub hit_cap: bool,
}

/// Samples the screen on its own thread so capture is never blocked by
/// rendering, and nothing the app draws between inputs is missed.
pub struct Sampler {
    acc: Arc<(Mutex<StateAccumulator>, Condvar)>,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
    over_budget: Arc<AtomicBool>,
    max_bytes: usize,
    start: Instant,
}

impl Sampler {
    pub fn start(
        handle: ParserHandle,
        sample: Duration,
        persist: Duration,
        max_bytes: usize,
    ) -> Sampler {
        let acc = Arc::new((Mutex::new(StateAccumulator::new(persist)), Condvar::new()));
        let stop = Arc::new(AtomicBool::new(false));
        let over_budget = Arc::new(AtomicBool::new(false));

        // Materialize one state synchronously, before the thread starts and
        // before `start` returns. Without this, the sampler's own first
        // observation — turning "nothing recorded yet" into a blank grid —
        // races the caller's first `wait()`: if that race is lost, `wait()`
        // sees its own startup bookkeeping as "the screen moved" and can
        // settle on a screen the child has not drawn onto yet. Doing it here
        // means `last_change` is already settled by the time any `wait()`
        // call can capture its `base_change`.
        let (grid, caret, mut last_gen) = handle.snapshot();
        {
            let (lock, _) = &*acc;
            lock.lock().unwrap().observe(grid, caret, Instant::now());
        }

        let thread = {
            let (acc, stop, over) = (Arc::clone(&acc), Arc::clone(&stop), Arc::clone(&over_budget));
            std::thread::spawn(move || {
                // The stop flag is only checked here, at the top of the loop,
                // after the previous iteration's `sleep(sample)` — so a
                // `stop()`/`Drop` can block for up to one `sample` interval.
                // Invisible at test-sized samples; worth knowing at a large
                // configured `sample_ms`.
                while !stop.load(Ordering::Relaxed) {
                    let gen = handle.generation();
                    // Something arrived: pay for the conversion, but do it
                    // *before* taking the accumulator lock, and take the
                    // timestamp together with the snapshot so it describes
                    // the screen actually read. Nothing arrived: a pending
                    // state may still have earned promotion, but that costs
                    // one guarded read, never a grid clone.
                    let sampled = if gen != last_gen { Some(handle.snapshot()) } else { None };
                    let now = Instant::now();
                    let (lock, cvar) = &*acc;
                    let bytes = {
                        let mut a = lock.lock().unwrap();
                        match sampled {
                            Some((grid, caret, new_gen)) => {
                                last_gen = new_gen;
                                a.observe(grid, caret, now);
                            }
                            None => a.tick(now),
                        }
                        a.bytes()
                    };
                    if bytes > max_bytes {
                        over.store(true, Ordering::Relaxed);
                        cvar.notify_all();
                        return;
                    }
                    cvar.notify_all();
                    std::thread::sleep(sample);
                }
            })
        };
        Sampler {
            acc,
            stop,
            thread: Some(thread),
            over_budget,
            max_bytes,
            start: Instant::now(),
        }
    }

    pub fn states(&self) -> MutexGuard<'_, StateAccumulator> {
        self.acc.0.lock().unwrap()
    }

    pub fn stop(mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }

    /// Two phases: up to `change` for the screen to move at all, then until the
    /// newest state has held for `stable` and matches `want`.
    ///
    /// Phase 1 is essential. Before the app reacts, the pre-input screen has
    /// been unchanged for a long time, so stability alone is satisfied the
    /// instant an input is sent. Measuring the grace on *grid changes* rather
    /// than PTY bytes is what stops the previous input's still-draining output
    /// from ending this wait *when that output does not change the screen*
    /// (a color reset, a redundant redraw of what's already there, and so
    /// on) — bytes alone would satisfy it, a grid comparison will not.
    ///
    /// It cannot stop output that genuinely repaints the screen: leftover
    /// text from a previous input can settle and hold for `stable` before
    /// the real reply arrives, and nothing about timing can tell the two
    /// apart from here. That is what `want` is for — pin the pattern the
    /// real reply must match, and a merely-plausible intermediate screen
    /// cannot satisfy it.
    ///
    /// Phase 1 is paid even when `want` is set — deliberately. A pattern that
    /// already matches a screen from *before* this call (one the app has not
    /// yet reacted onto) must not end the wait instantly: that is a stale
    /// match, the same shape of bug as capturing a screen the app never
    /// drew. Skipping the grace whenever `want` matches would reintroduce it.
    /// The bounded `change` cost is the price of ruling that out.
    pub fn wait(
        &self,
        want: Option<&Pattern>,
        change: Duration,
        stable: Duration,
        timeout: Duration,
    ) -> Result<WaitOutcome> {
        let start = Instant::now();
        let (lock, cvar) = &*self.acc;
        let mut a = lock.lock().unwrap();
        let base_change = a.last_change();
        loop {
            if self.over_budget.load(Ordering::Relaxed) {
                let states = a.committed().len();
                drop(a);
                trace("budget", start.elapsed(), false, want, states);
                let elapsed = self.start.elapsed();
                bail!(
                    "recording exceeded max_capture_mb = {} after {} ({} states)\n\
                     raise max_capture_mb, or raise persist_ms to commit fewer states",
                    self.max_bytes / (1024 * 1024),
                    format_elapsed(elapsed),
                    states,
                );
            }
            let now = Instant::now();
            let elapsed = now.duration_since(start);
            let moved = a.last_change() > base_change;
            let held = now.duration_since(a.last_change()) >= stable;
            let grace_left = change.saturating_sub(elapsed);
            let matched = match (want, a.newest()) {
                (None, _) => true,
                (Some(_), None) => false,
                (Some(p), Some(s)) => p.matches(&s.grid),
            };

            // Phase 1 is satisfied by any change, or by the grace expiring.
            let phase1 = moved || grace_left.is_zero();
            if phase1 && held && matched {
                // Which half of phase 1 let this through is the interesting
                // bit: `grace` means the screen never moved at all.
                let idx = a.settled_index(now);
                let states = a.committed().len();
                drop(a);
                trace(if moved { "moved" } else { "grace" }, elapsed, moved, want, states);
                return Ok(WaitOutcome { state: idx, hit_cap: false });
            }
            if elapsed >= timeout {
                if let Some(p) = want {
                    let states = a.committed().len();
                    let seen = a
                        .newest()
                        .map(|s| crate::pattern::screen_text(&s.grid))
                        .unwrap_or_default();
                    drop(a);
                    trace("nomatch", elapsed, moved, want, states);
                    bail!(
                        "await pattern {:?} never matched within {}ms\n--- last screen ---\n{seen}",
                        p.source(),
                        timeout.as_millis()
                    );
                }
                let idx = a.settled_index(now);
                let states = a.committed().len();
                drop(a);
                trace("cap", elapsed, moved, want, states);
                return Ok(WaitOutcome { state: idx, hit_cap: true });
            }
            let nap = Duration::from_millis(2)
                .min(timeout.saturating_sub(elapsed))
                .max(Duration::from_millis(1));
            a = cvar.wait_timeout(a, nap).unwrap().0;
        }
    }
}

impl Drop for Sampler {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

/// Debug aid: `ANSIDRAMA_TRACE=1` writes one line per wait to stderr — the
/// direct successor to 0.2.0's per-`settle` trace, with its fields adapted to
/// the two-phase wait.
///
/// `why` is how the wait ended: `moved` (phase 1 satisfied by a real grid
/// change — the healthy case), `grace` (phase 1 satisfied only by the grace
/// expiring), `cap` (the bound was reached with no `await`), `nomatch` (an
/// `await` that never matched — about to become an error), `budget`
/// (`max_capture_mb`), `dwell` (an animated scene, which runs no wait at all).
///
/// `moved=no` is the successor to the old `reads=0` red flag: this input never
/// changed the screen, so whatever frame it produces shows the screen from
/// *before* it. A run full of `grace`/`moved=no` lines is dead air, which is
/// exactly what this redesign set out to remove — and the only way to tell
/// whether it did.
///
/// Takes `states` (the committed count) rather than `&StateAccumulator`
/// itself: the accumulator's mutex must be released before this runs, since
/// `eprintln!` can block on a slow stderr and the sampler thread would stall
/// for as long as it waited on that same lock.
pub(crate) fn trace(
    why: &str,
    elapsed: Duration,
    moved: bool,
    want: Option<&Pattern>,
    states: usize,
) {
    if std::env::var_os("ANSIDRAMA_TRACE").is_none() {
        return;
    }
    eprintln!(
        "wait {why:>7} after {:>7.1}ms  moved={:<3} states={:<4} await={}",
        elapsed.as_secs_f32() * 1000.0,
        if moved { "yes" } else { "no" },
        states,
        want.map(|p| p.source()).unwrap_or("-"),
    );
}

/// Format a duration as e.g. `4m12s`, or `12s` under a minute, for the
/// memory-budget error message.
fn format_elapsed(d: Duration) -> String {
    let secs = d.as_secs();
    let (m, s) = (secs / 60, secs % 60);
    if m > 0 {
        format!("{m}m{s}s")
    } else {
        format!("{s}s")
    }
}

#[cfg(test)]
mod acc_tests {
    use super::*;

    fn cell(ch: char) -> Cell {
        Cell { ch, fg: (0, 0, 0), bg: (0, 0, 0), bold: false }
    }
    fn g(ch: char) -> Vec<Vec<Cell>> {
        vec![vec![cell(ch); 4]; 2]
    }
    fn ms(n: u64) -> Duration {
        Duration::from_millis(n)
    }

    #[test]
    fn a_state_that_persists_is_committed() {
        let t0 = Instant::now();
        let mut a = StateAccumulator::new(ms(40));
        a.observe(g('a'), None, t0);
        assert_eq!(a.committed().len(), 0, "not yet survived persist");
        a.observe(g('a'), None, t0 + ms(50));
        assert_eq!(a.committed().len(), 1);
        assert_eq!(a.committed()[0].t, t0, "timestamp is when it first appeared");
    }

    #[test]
    fn a_transient_is_dropped() {
        let t0 = Instant::now();
        let mut a = StateAccumulator::new(ms(40));
        a.observe(g('a'), None, t0);
        a.observe(g('b'), None, t0 + ms(5)); // supersedes 'a' before it persisted
        a.observe(g('b'), None, t0 + ms(60));
        let committed: Vec<char> = a.committed().iter().map(|s| s.grid[0][0].ch).collect();
        assert_eq!(committed, vec!['b'], "the torn intermediate must not survive");
    }

    #[test]
    fn flicker_resets_the_change_clock() {
        let t0 = Instant::now();
        let mut a = StateAccumulator::new(ms(40));
        a.observe(g('a'), None, t0);
        a.observe(g('b'), None, t0 + ms(10));
        a.observe(g('a'), None, t0 + ms(20));
        assert_eq!(a.last_change(), t0 + ms(20), "A->B->A is two changes, not zero");
    }

    #[test]
    fn force_commit_promotes_a_pending_state() {
        let t0 = Instant::now();
        let mut a = StateAccumulator::new(ms(200));
        a.observe(g('a'), None, t0);
        assert_eq!(a.committed().len(), 0);
        assert_eq!(a.force_commit(t0 + ms(40)), Some(0));
        assert_eq!(a.committed().len(), 1, "an input's settled state is always kept");
        // Idempotent: nothing pending now.
        assert_eq!(a.force_commit(t0 + ms(41)), None);
    }

    #[test]
    fn an_unchanged_screen_adds_nothing() {
        let t0 = Instant::now();
        let mut a = StateAccumulator::new(ms(40));
        a.observe(g('a'), None, t0);
        a.observe(g('a'), None, t0 + ms(50));
        a.observe(g('a'), None, t0 + ms(500));
        assert_eq!(a.committed().len(), 1);
        assert_eq!(a.last_change(), t0);
    }

    #[test]
    fn bytes_tracks_committed_grids() {
        let t0 = Instant::now();
        let mut a = StateAccumulator::new(ms(40));
        assert_eq!(a.bytes(), 0);
        a.observe(g('a'), None, t0);
        a.observe(g('a'), None, t0 + ms(50));
        assert!(a.bytes() > 0, "a committed state must count toward the memory bound");
    }

    /// A plain space overwrites a blank cell with a blank cell: the grid is
    /// byte-identical before and after, and only the caret moves. If `observe`
    /// compared the grid alone it would call this "unchanged" and never
    /// record the new caret position — this is the regression guard for
    /// exactly that bug (found by the capture regression gate: HEAD drew the
    /// block cursor one cell behind the app after a bare-space keystroke).
    #[test]
    fn a_caret_only_change_is_a_new_state() {
        let t0 = Instant::now();
        let mut a = StateAccumulator::new(ms(40));
        a.observe(g('a'), Some((0, 0)), t0);
        a.observe(g('a'), Some((1, 0)), t0 + ms(10)); // same grid, caret moved
        assert_eq!(
            a.last_change(),
            t0 + ms(10),
            "a caret-only change must still count as a change"
        );
        a.observe(g('a'), Some((1, 0)), t0 + ms(60));
        assert_eq!(a.committed().len(), 1);
        assert_eq!(
            a.committed()[0].caret,
            Some((1, 0)),
            "the committed state must carry the caret's new position, not the stale one"
        );
    }

    #[test]
    fn tick_promotes_a_persisted_state_without_a_grid() {
        let t0 = Instant::now();
        let mut a = StateAccumulator::new(ms(40));
        a.observe(g('a'), None, t0);
        a.tick(t0 + ms(10));
        assert_eq!(a.committed().len(), 0, "not yet survived persist");
        a.tick(t0 + ms(50));
        assert_eq!(a.committed().len(), 1);
        assert_eq!(a.committed()[0].t, t0, "timestamp is when it first appeared");
    }
}

#[cfg(all(test, unix))]
mod pty_tests {
    use super::*;
    use crate::pattern::Pattern;
    use crate::term::Term;
    use std::collections::BTreeMap;

    fn sampler_for(term: &Term) -> Sampler {
        Sampler::start(
            term.handle(),
            Duration::from_millis(5),
            Duration::from_millis(40),
            256 * 1024 * 1024,
        )
    }

    /// The scenario 0.2.0 fixed with `react_ms`: the app goes quiet for longer
    /// than `stable` *before* it answers. The pre-input screen is already
    /// stable, so stability alone would return it.
    #[test]
    fn waits_for_a_late_first_reply() {
        let env = BTreeMap::new();
        let mut term = Term::spawn(
            20,
            3,
            "stty -echo; printf 'READY'; read -n1 k; sleep 0.6; printf 'LATE'; sleep 2",
            &env,
        )
        .unwrap();
        let s = sampler_for(&term);
        let ready = Pattern::new("READY", None).unwrap();
        s.wait(Some(&ready), Duration::ZERO, Duration::from_millis(40), Duration::from_secs(5))
            .unwrap();

        term.send_key("x").unwrap();
        let out = s
            .wait(
                None,
                Duration::from_millis(2000), // change grace
                Duration::from_millis(40),
                Duration::from_secs(5),
            )
            .unwrap();
        let acc = s.states();
        let text = crate::pattern::screen_text(&acc.committed()[out.state].grid);
        assert!(text.contains("LATE"), "captured before the app answered: {text:?}");
    }

    /// The disarm bug: bytes still draining from the *previous* input must not
    /// satisfy this input's grace. The old bug was byte-based — `read_loop`
    /// cleared `awaiting_reply` on *any byte* — so an SGR reset (`\033[0m`) is
    /// the faithful analogue: it is real output, it bumps the generation
    /// counter, but it touches no cell, so the grid comparison sees no
    /// change and `last_change` does not move. If phase 1 were keyed off
    /// bytes or the generation counter instead of grid changes, this test
    /// would fail — the wait would end at ~390ms holding "READY".
    #[test]
    fn is_not_disarmed_by_the_previous_inputs_output() {
        let env = BTreeMap::new();
        let mut term = Term::spawn(
            20,
            3,
            "stty -echo; printf 'READY'; \
             read -n1 a; (sleep 0.4; printf '\\033[0m') & \
             read -n1 b; sleep 1.2; printf 'LATE'; sleep 3",
            &env,
        )
        .unwrap();
        let s = sampler_for(&term);
        let ready = Pattern::new("READY", None).unwrap();
        s.wait(Some(&ready), Duration::ZERO, Duration::from_millis(40), Duration::from_secs(5))
            .unwrap();

        term.send_key("a").unwrap();
        let _ = s.wait(None, Duration::from_millis(50), Duration::from_millis(40), Duration::from_millis(200));
        term.send_key("b").unwrap();
        let out = s
            .wait(None, Duration::from_millis(2000), Duration::from_millis(40), Duration::from_secs(5))
            .unwrap();

        let acc = s.states();
        let text = crate::pattern::screen_text(&acc.committed()[out.state].grid);
        assert!(text.contains("LATE"), "previous input's output ended the wait: {text:?}");
    }

    /// Secondary benefit of `observe` comparing caret alongside grid (the fix
    /// for the regression-gate bug above `a_caret_only_change_is_a_new_state`
    /// in `acc_tests`): a reply that only moves the cursor — a cursor-forward
    /// escape, no glyph touched — now satisfies phase 1's `moved` check like
    /// any other real change, instead of always burning the full `change`
    /// grace as if the app hadn't answered yet.
    #[test]
    fn a_caret_only_reply_ends_phase_one_promptly() {
        let env = BTreeMap::new();
        let mut term = Term::spawn(
            20,
            3,
            "stty -echo; printf 'READY'; read -n1 k; sleep 0.05; printf '\\033[3C'; sleep 3",
            &env,
        )
        .unwrap();
        let s = sampler_for(&term);
        let ready = Pattern::new("READY", None).unwrap();
        s.wait(Some(&ready), Duration::ZERO, Duration::from_millis(40), Duration::from_secs(5))
            .unwrap();

        term.send_key("k").unwrap();
        let started = Instant::now();
        s.wait(None, Duration::from_millis(2000), Duration::from_millis(40), Duration::from_secs(5))
            .unwrap();
        let elapsed = started.elapsed();
        assert!(
            elapsed < Duration::from_millis(500),
            "a caret-only change should end phase 1 promptly instead of burning \
             the full 2000ms grace: {elapsed:?}"
        );
    }

    /// The residual case grid-comparison genuinely cannot decide: the
    /// previous input's leftover output really does repaint the screen
    /// (`TRAIL`), settles, and holds for `stable` long before the real reply
    /// (`LATE`) arrives. A plain `wait(None, ...)` cannot tell these apart —
    /// that is Bug B, not fixable by any timing rule. `await` is the escape
    /// hatch: pinning the expected pattern means a merely-plausible
    /// intermediate screen can never satisfy the wait, so the result is
    /// never a silently wrong frame.
    #[test]
    fn await_survives_a_previous_inputs_repaint() {
        let env = BTreeMap::new();
        let mut term = Term::spawn(
            20,
            3,
            "stty -echo; printf 'READY'; \
             read -n1 a; (sleep 0.4; printf 'TRAIL') & \
             read -n1 b; sleep 1.2; printf 'LATE'; sleep 3",
            &env,
        )
        .unwrap();
        let s = sampler_for(&term);
        let ready = Pattern::new("READY", None).unwrap();
        s.wait(Some(&ready), Duration::ZERO, Duration::from_millis(40), Duration::from_secs(5))
            .unwrap();

        term.send_key("a").unwrap();
        let _ = s.wait(None, Duration::from_millis(50), Duration::from_millis(40), Duration::from_millis(200));
        term.send_key("b").unwrap();
        let late = Pattern::new("LATE", None).unwrap();
        let out = s
            .wait(Some(&late), Duration::from_millis(2000), Duration::from_millis(40), Duration::from_secs(5))
            .unwrap();

        let acc = s.states();
        let text = crate::pattern::screen_text(&acc.committed()[out.state].grid);
        assert!(text.contains("LATE"), "TRAIL was mistaken for the real reply: {text:?}");
    }

    /// An `await` that matches returns promptly and does not spend the
    /// timeout. Phase 1's `change` grace is paid regardless (by design — see
    /// `wait`'s doc comment), so the expected latency is ~`change` + `stable`
    /// here (~140ms), not just `stable`; a small `change` and a bound well
    /// under the timeout is what makes this discriminate a wrongly-burned
    /// grace or timeout from a normal return.
    #[test]
    fn await_returns_on_match() {
        let env = BTreeMap::new();
        let term = Term::spawn(20, 3, "printf 'HELLO'; sleep 3", &env).unwrap();
        let s = sampler_for(&term);
        let p = Pattern::new("HELLO", Some(0)).unwrap();
        let started = Instant::now();
        let out = s
            .wait(Some(&p), Duration::from_millis(100), Duration::from_millis(40), Duration::from_secs(5))
            .unwrap();
        let elapsed = started.elapsed();
        assert!(!out.hit_cap);
        assert!(
            elapsed < Duration::from_millis(600),
            "spent too long for a 100ms change grace + 40ms stable: {elapsed:?}"
        );
    }

    /// An `await` that never matches is an error naming the pattern *and*
    /// showing the last screen's text — never a silently wrong frame.
    #[test]
    fn await_that_never_matches_is_an_error() {
        let env = BTreeMap::new();
        let term = Term::spawn(20, 3, "printf 'HELLO'; sleep 3", &env).unwrap();
        let s = sampler_for(&term);
        let p = Pattern::new("GOODBYE", None).unwrap();
        let err = s
            .wait(Some(&p), Duration::from_millis(100), Duration::from_millis(40), Duration::from_millis(400))
            .unwrap_err()
            .to_string();
        assert!(err.contains("GOODBYE"), "error should name the pattern: {err}");
        assert!(err.contains("HELLO"), "error should show the last screen's text: {err}");
    }

    /// The over-budget path: the thread's early return, `wait`'s bail, and
    /// Override 3's message. `max_bytes` is deliberately absurdly small (1
    /// byte) so the very first committed state trips it. That renders
    /// `max_capture_mb = 0` via integer division — a test-only artifact of
    /// picking a sub-MB budget to trip the limit fast; a real config value
    /// is always at least 1 whole MB, so the message never needs to guard
    /// against a `0` in practice.
    #[test]
    fn exceeding_the_memory_budget_is_an_error_naming_the_numbers() {
        let env = BTreeMap::new();
        let term = Term::spawn(20, 3, "printf 'HELLO'; sleep 3", &env).unwrap();
        let s = Sampler::start(term.handle(), Duration::from_millis(5), Duration::from_millis(40), 1);
        let err = s
            .wait(None, Duration::from_millis(200), Duration::from_millis(40), Duration::from_secs(5))
            .unwrap_err()
            .to_string();
        assert!(err.contains("max_capture_mb"), "should name the limit: {err}");
        assert!(err.contains("states)"), "should name the state count: {err}");
        assert!(err.contains("raise max_capture_mb"), "should name the two knobs: {err}");
    }

    /// An input that draws nothing costs exactly the grace, then returns
    /// normally — reaching the cap is not an error.
    #[test]
    fn an_input_that_draws_nothing_returns_after_the_grace() {
        let env = BTreeMap::new();
        let term = Term::spawn(20, 3, "printf 'IDLE'; sleep 3", &env).unwrap();
        let s = sampler_for(&term);
        s.wait(None, Duration::from_millis(300), Duration::from_millis(40), Duration::from_secs(2))
            .unwrap();
        // Second wait: nothing changes at all.
        let started = Instant::now();
        let out = s
            .wait(None, Duration::from_millis(150), Duration::from_millis(40), Duration::from_millis(600))
            .unwrap();
        let elapsed = started.elapsed();
        assert!(!out.hit_cap, "grace expiring then a stable screen is a normal return");
        assert!(
            elapsed >= Duration::from_millis(150),
            "must not return before the grace has been paid: {elapsed:?}"
        );
        assert!(
            elapsed < Duration::from_millis(500),
            "must not burn the whole timeout, generous headroom for a loaded machine: {elapsed:?}"
        );
    }
}
