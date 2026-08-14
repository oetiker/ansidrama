//! Continuous grid sampling: a thread that snapshots the screen, and the
//! pending/committed rule that decides which snapshots are real states.

use std::time::{Duration, Instant};

use crate::grid::Cell;

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
        let newest = self
            .pending
            .as_ref()
            .map(|s| &s.grid)
            .or_else(|| self.committed.last().map(|s| &s.grid));
        if newest != Some(&grid) {
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
