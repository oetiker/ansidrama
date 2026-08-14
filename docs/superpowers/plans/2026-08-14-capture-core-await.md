# Continuous Capture and Declared Scene Completion — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace `record`'s PTY-quiet heuristic with continuous grid sampling plus script-declared scene completion (`await`), so a screen the app reached is never lost and a declared expectation that fails aborts the run.

**Architecture:** Three phases replace one loop. `record.rs` drives (send input, wait, mark). `sampler.rs` samples the grid on a thread into a state log with a pending/committed split. `assemble.rs` turns the state log plus input marks into frames — a pure function — and only surviving frames are rasterised. `term::settle()` is deleted.

**Tech Stack:** Rust 2021, `vt100` (Junyi-99 `deck` fork), `regex-lite` (new), `image`, `webp`, `anyhow`, `serde`/`toml`, `rustix`.

**Spec:** `docs/superpowers/specs/2026-08-14-capture-core-await-design.md` — read it before Task 1. The plan argues from the spec; where they disagree, the spec wins.

## Global Constraints

- **Shared machine: cap parallelism to 4 cores.** Prefix every cargo invocation with `CARGO_BUILD_JOBS=4`.
- Cargo target dir is redirected to `/home/oetiker/scratch/cargo-target`; the release binary is **not** under `./target/`.
- `record` is unix-only (`#[cfg(unix)]` on the `record`/`term`/`keys` modules). New modules `sampler` and `pattern` follow the same gating: `sampler` is unix-only, `pattern` is portable (pure grid matching, no PTY).
- Every task must leave `CARGO_BUILD_JOBS=4 cargo test` green and `cargo clippy --all-targets` warning-free. Additive first, deletions last — see Task 7.
- Config defaults, verbatim from the spec: `sample_ms = 10`, `change_ms = 150`, `stable_ms = 40`, `persist_ms = 40`, `wait_cap_ms = 3000`, `await_ms = 5000`, `realtime = false`, `startup_ms = 900`, `max_capture_mb = 256`.
- **Removed** config keys: `settle_ms`, `react_ms`. `RecordConfig` uses `#[serde(deny_unknown_fields)]`, so old configs fail loudly at parse — that is the intended migration path, not a regression.
- Comments, identifiers and docs in English.

---

## File Structure

| File | Responsibility |
|---|---|
| `src/pattern.rs` (new) | `Pattern` — compiled regex plus optional row scoping; grid→text. No PTY, portable, pure. |
| `src/sampler.rs` (new) | `StateAccumulator` (pure pending/committed logic) and `Sampler` (thread + `wait`). Unix-only. |
| `src/assemble.rs` (new) | `assemble()` — pure `(state times, marks) → Vec<FrameSpec>`. Portable, no clock, no rendering. |
| `src/term.rs` (modify) | Add `generation`, add `ParserHandle`; **delete `settle()` in Task 7**. |
| `src/record.rs` (modify) | Drive only: send, wait, mark. Rendering moves to assembly. Writes the manifest. |
| `src/config.rs` (modify) | New timing keys; `Scene::await`/`animated`; patterns compiled at load. |
| `src/grid.rs` (modify) | `Cell` gains `PartialEq` (grid comparison needs it). |
| `src/lib.rs` (modify) | Register `pattern`, `sampler`, `assemble`. |
| `Cargo.toml` (modify) | Add `regex-lite`. |

`raster.rs`, `encode.rs`, `chrome.rs`, `frame.rs`, `mouse.rs`, `keys.rs`, `color.rs`, `cursor.rs` are untouched.

**Why `regex-lite` and not `regex`:** this project prefers lean, pure-Rust dependencies. `regex-lite` (0.1.9, MIT/Apache) is the upstream rust-lang crate built for binary size, with no `aho-corasick`/`memchr`/SIMD tree. Screen matching is nowhere near hot enough to need the full engine.

---

## Task 1: `Pattern` — regex with optional row scoping

**Files:**
- Create: `src/pattern.rs`
- Modify: `Cargo.toml`, `src/lib.rs`

**Interfaces:**
- Consumes: `crate::grid::Cell`
- Produces:
  - `pub fn screen_text(grid: &[Vec<Cell>]) -> String` — rows joined by `\n`, trailing spaces per row trimmed
  - `pub struct Pattern`
  - `pub fn Pattern::new(find: &str, row: Option<i32>) -> anyhow::Result<Pattern>`
  - `pub fn Pattern::matches(&self, grid: &[Vec<Cell>]) -> bool`
  - `pub fn Pattern::row(&self) -> Option<i32>`

- [ ] **Step 1: Add the dependency and register the module**

```bash
CARGO_BUILD_JOBS=4 cargo add regex-lite@0.1
```

In `src/lib.rs`, beside the other `pub mod` lines (alphabetical, and **not** `#[cfg(unix)]` — this module is portable):

```rust
pub mod pattern;
```

- [ ] **Step 2: Write the failing tests**

Create `src/pattern.rs` with only the test module and stub signatures:

```rust
//! Screen-content predicates: a regex, optionally scoped to one row.

use anyhow::{Context, Result};
use regex_lite::Regex;

use crate::grid::Cell;

/// The grid as text: one line per row, trailing blanks trimmed.
pub fn screen_text(_grid: &[Vec<Cell>]) -> String {
    unimplemented!()
}

/// A compiled screen predicate.
pub struct Pattern {
    re: Regex,
    row: Option<i32>,
}

impl Pattern {
    pub fn new(_find: &str, _row: Option<i32>) -> Result<Pattern> {
        unimplemented!()
    }
    pub fn matches(&self, _grid: &[Vec<Cell>]) -> bool {
        unimplemented!()
    }
    pub fn row(&self) -> Option<i32> {
        self.row
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cell(ch: char) -> Cell {
        Cell { ch, fg: (0, 0, 0), bg: (0, 0, 0), bold: false }
    }

    /// Build a grid from lines, padded to `cols`.
    fn grid(lines: &[&str], cols: usize) -> Vec<Vec<Cell>> {
        lines
            .iter()
            .map(|l| {
                let mut row: Vec<Cell> = l.chars().map(cell).collect();
                row.resize(cols, cell(' '));
                row
            })
            .collect()
    }

    #[test]
    fn matches_anywhere_on_screen() {
        let g = grid(&["hello world", "theme: light"], 20);
        assert!(Pattern::new("theme: light", None).unwrap().matches(&g));
        assert!(!Pattern::new("theme: dark", None).unwrap().matches(&g));
    }

    #[test]
    fn row_scoping_restricts_the_match() {
        let g = grid(&["theme: light", "nothing here"], 20);
        // present, but on row 0 — a row-1 scoped pattern must not see it
        assert!(Pattern::new("theme: light", Some(0)).unwrap().matches(&g));
        assert!(!Pattern::new("theme: light", Some(1)).unwrap().matches(&g));
    }

    #[test]
    fn negative_row_counts_from_the_bottom() {
        let g = grid(&["a", "b", "theme: light"], 20);
        assert!(Pattern::new("theme: light", Some(-1)).unwrap().matches(&g));
        assert!(!Pattern::new("theme: light", Some(-2)).unwrap().matches(&g));
    }

    #[test]
    fn out_of_range_row_never_matches() {
        let g = grid(&["theme: light"], 20);
        assert!(!Pattern::new("theme: light", Some(9)).unwrap().matches(&g));
        assert!(!Pattern::new("theme: light", Some(-9)).unwrap().matches(&g));
    }

    #[test]
    fn a_pattern_does_not_match_across_a_row_boundary() {
        // `.` must not cross the newline that separates rows.
        let g = grid(&["abc", "def"], 3);
        assert!(!Pattern::new("abc.def", None).unwrap().matches(&g));
    }

    #[test]
    fn trailing_blanks_are_trimmed_so_end_anchors_work() {
        let g = grid(&["done"], 40);
        assert!(Pattern::new("done$", Some(0)).unwrap().matches(&g));
    }

    #[test]
    fn a_bad_regex_is_an_error_not_a_panic() {
        assert!(Pattern::new("unclosed(", None).is_err());
    }
}
```

- [ ] **Step 3: Run the tests to verify they fail**

Run: `CARGO_BUILD_JOBS=4 cargo test --lib pattern::`
Expected: FAIL — panics with `not implemented`.

- [ ] **Step 4: Implement**

Replace the two `unimplemented!()` bodies:

```rust
pub fn screen_text(grid: &[Vec<Cell>]) -> String {
    let mut out = String::new();
    for (i, row) in grid.iter().enumerate() {
        if i > 0 {
            out.push('\n');
        }
        let line: String = row.iter().map(|c| c.ch).collect();
        out.push_str(line.trim_end());
    }
    out
}

impl Pattern {
    pub fn new(find: &str, row: Option<i32>) -> Result<Pattern> {
        let re = Regex::new(find)
            .with_context(|| format!("compile await pattern {find:?}"))?;
        Ok(Pattern { re, row })
    }

    pub fn matches(&self, grid: &[Vec<Cell>]) -> bool {
        match self.row {
            None => self.re.is_match(&screen_text(grid)),
            Some(r) => {
                let Some(idx) = resolve_row(r, grid.len()) else {
                    return false;
                };
                let line: String = grid[idx].iter().map(|c| c.ch).collect();
                self.re.is_match(line.trim_end())
            }
        }
    }
}

/// Resolve a possibly-negative row index against a screen of `rows` rows.
/// `-1` is the last row. Out of range yields `None`.
fn resolve_row(row: i32, rows: usize) -> Option<usize> {
    let rows = rows as i32;
    let idx = if row < 0 { rows + row } else { row };
    (idx >= 0 && idx < rows).then_some(idx as usize)
}
```

- [ ] **Step 5: Run the tests to verify they pass**

Run: `CARGO_BUILD_JOBS=4 cargo test --lib pattern:: && CARGO_BUILD_JOBS=4 cargo clippy --all-targets`
Expected: 7 passed, no clippy warnings.

- [ ] **Step 6: Commit**

```bash
git add Cargo.toml Cargo.lock src/lib.rs src/pattern.rs
git commit -m "feat(pattern): screen predicates with optional row scoping"
```

---

## Task 2: `generation` counter and `ParserHandle`

Additive only — `settle()` stays until Task 7 so the tree keeps compiling.

**Files:**
- Modify: `src/term.rs`, `src/grid.rs`

**Interfaces:**
- Consumes: nothing new
- Produces:
  - `pub struct ParserHandle` (`Clone`)
  - `pub fn ParserHandle::generation(&self) -> u64`
  - `pub fn ParserHandle::snapshot(&self) -> (Vec<Vec<Cell>>, Option<(u32, u32)>, u64)` — grid, caret and generation read under **one** lock acquisition
  - `pub fn ParserHandle::is_eof(&self) -> bool`
  - `pub fn Term::handle(&self) -> ParserHandle`
  - `Cell` now derives `PartialEq`

- [ ] **Step 1: Give `Cell` equality**

In `src/grid.rs:12`:

```rust
#[derive(Clone, Copy, PartialEq)]
pub struct Cell {
```

- [ ] **Step 2: Write the failing test**

Append to the `pty_tests` module in `src/term.rs`:

```rust
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
```

- [ ] **Step 3: Run it to verify it fails**

Run: `CARGO_BUILD_JOBS=4 cargo test --lib term::pty_tests::generation_advances_only_on_output`
Expected: FAIL — no method `handle` on `Term`.

- [ ] **Step 4: Implement**

In `struct Shared`, add the field (beside `eof`):

```rust
    /// Bumped on every `parser.process`. Lets a reader skip grid conversion
    /// when nothing has arrived since it last looked.
    generation: u64,
```

Initialise it in `Term::spawn`'s `Shared { .. }` literal with `generation: 0,`.

In `read_loop`, in the `Ok(n)` arm, beside `s.last_activity = Instant::now();`:

```rust
                s.generation += 1;
```

Add the handle type after the `Term` struct:

```rust
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
```

And on `impl Term`:

```rust
    pub fn handle(&self) -> ParserHandle {
        ParserHandle {
            shared: Arc::clone(&self.shared),
            rows: self.rows,
            cols: self.cols,
        }
    }
```

- [ ] **Step 5: Run the tests to verify they pass**

Run: `CARGO_BUILD_JOBS=4 cargo test && CARGO_BUILD_JOBS=4 cargo clippy --all-targets`
Expected: all pass (45 lib tests), no warnings.

- [ ] **Step 6: Commit**

```bash
git add src/term.rs src/grid.rs
git commit -m "feat(term): generation counter and a read-only ParserHandle"
```

---

## Task 3: `StateAccumulator` — the pending/committed rule

Pure logic with an injected clock, so the interesting rules are tested without a PTY or a thread.

**Files:**
- Create: `src/sampler.rs`
- Modify: `src/lib.rs`

**Interfaces:**
- Consumes: `crate::grid::Cell`
- Produces:
  - `pub struct State { pub grid: Vec<Vec<Cell>>, pub caret: Option<(u32, u32)>, pub t: Instant }`
  - `pub struct StateAccumulator`
  - `pub fn StateAccumulator::new(persist: Duration) -> StateAccumulator`
  - `pub fn StateAccumulator::observe(&mut self, grid: Vec<Vec<Cell>>, caret: Option<(u32, u32)>, now: Instant)`
  - `pub fn StateAccumulator::force_commit(&mut self, now: Instant) -> Option<usize>` — commit the pending state regardless of `persist`, returning its committed index
  - `pub fn StateAccumulator::committed(&self) -> &[State]`
  - `pub fn StateAccumulator::last_change(&self) -> Instant`
  - `pub fn StateAccumulator::bytes(&self) -> usize`

`observe` must be called on every tick, with or without a change: it is what promotes a pending state once it has survived `persist`.

- [ ] **Step 1: Register the module**

In `src/lib.rs`, beside the other unix-gated modules:

```rust
#[cfg(unix)]
pub mod sampler;
```

- [ ] **Step 2: Write the failing tests**

Create `src/sampler.rs`:

```rust
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
    pub fn new(_persist: Duration) -> StateAccumulator {
        unimplemented!()
    }
    pub fn observe(&mut self, _grid: Vec<Vec<Cell>>, _caret: Option<(u32, u32)>, _now: Instant) {
        unimplemented!()
    }
    pub fn force_commit(&mut self, _now: Instant) -> Option<usize> {
        unimplemented!()
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
}
```

- [ ] **Step 3: Run them to verify they fail**

Run: `CARGO_BUILD_JOBS=4 cargo test --lib sampler::acc_tests`
Expected: FAIL — `not implemented`.

- [ ] **Step 4: Implement**

```rust
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
}

/// Approximate resident size of one state, for the `max_capture_mb` bound.
fn state_bytes(s: &State) -> usize {
    s.grid
        .iter()
        .map(|r| r.len() * std::mem::size_of::<Cell>() + std::mem::size_of::<Vec<Cell>>())
        .sum()
}
```

Note `force_commit` takes `now` for symmetry with `observe` and for future diagnostics; it is unused today, so name it `_now`.

- [ ] **Step 5: Run them to verify they pass**

Run: `CARGO_BUILD_JOBS=4 cargo test --lib sampler:: && CARGO_BUILD_JOBS=4 cargo clippy --all-targets`
Expected: 6 passed, no warnings.

- [ ] **Step 6: Commit**

```bash
git add src/lib.rs src/sampler.rs
git commit -m "feat(sampler): pending/committed state rule"
```

---

## Task 4: The `Sampler` thread and `wait()`

**Files:**
- Modify: `src/sampler.rs`

**Interfaces:**
- Consumes: `ParserHandle` (Task 2), `Pattern` (Task 1), `StateAccumulator` (Task 3)
- Produces:
  - `pub struct Sampler`
  - `pub fn Sampler::start(handle: ParserHandle, sample: Duration, persist: Duration, max_bytes: usize) -> Sampler`
  - `pub struct WaitOutcome { pub state: usize, pub hit_cap: bool }`
  - `pub fn Sampler::wait(&self, want: Option<&Pattern>, change: Duration, stable: Duration, timeout: Duration) -> Result<WaitOutcome>`
  - `pub fn Sampler::states(&self) -> MutexGuard<'_, StateAccumulator>`
  - `pub fn Sampler::stop(self)`

`wait` implements the two phases from the spec: up to `change` for a grid change after the call began, then until the newest state has held for `stable` and matches `want`. It returns the index of the settled state, force-committing it so assembly always has a frame for the input.

- [ ] **Step 1: Write the failing tests**

Append to `src/sampler.rs`:

```rust
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

    /// The disarm bug: output still draining from the *previous* input must not
    /// satisfy this input's grace. Measured on grid changes, it cannot.
    #[test]
    fn is_not_disarmed_by_the_previous_inputs_output() {
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
        let out = s
            .wait(None, Duration::from_millis(2000), Duration::from_millis(40), Duration::from_secs(5))
            .unwrap();

        let acc = s.states();
        let text = crate::pattern::screen_text(&acc.committed()[out.state].grid);
        assert!(text.contains("LATE"), "previous input's output ended the wait: {text:?}");
    }

    /// An `await` that matches returns promptly and does not spend the timeout.
    #[test]
    fn await_returns_on_match() {
        let env = BTreeMap::new();
        let term = Term::spawn(20, 3, "printf 'HELLO'; sleep 3", &env).unwrap();
        let s = sampler_for(&term);
        let p = Pattern::new("HELLO", Some(0)).unwrap();
        let started = Instant::now();
        let out = s
            .wait(Some(&p), Duration::from_millis(500), Duration::from_millis(40), Duration::from_secs(5))
            .unwrap();
        assert!(!out.hit_cap);
        assert!(started.elapsed() < Duration::from_secs(2), "spent too long: {:?}", started.elapsed());
    }

    /// An `await` that never matches is an error naming the pattern — never a
    /// silently wrong frame.
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
    }

    /// An input that draws nothing costs the grace and then proceeds, flagged.
    #[test]
    fn an_input_that_draws_nothing_hits_the_cap() {
        let env = BTreeMap::new();
        let term = Term::spawn(20, 3, "printf 'IDLE'; sleep 3", &env).unwrap();
        let s = sampler_for(&term);
        s.wait(None, Duration::from_millis(300), Duration::from_millis(40), Duration::from_secs(2))
            .unwrap();
        // Second wait: nothing changes at all.
        let out = s
            .wait(None, Duration::from_millis(150), Duration::from_millis(40), Duration::from_millis(600))
            .unwrap();
        assert!(!out.hit_cap, "grace expiring then a stable screen is a normal return");
    }
}
```

- [ ] **Step 2: Run them to verify they fail**

Run: `CARGO_BUILD_JOBS=4 cargo test --lib sampler::pty_tests`
Expected: FAIL — no `Sampler::start`.

- [ ] **Step 3: Implement the sampler**

Add to the top of `src/sampler.rs`:

```rust
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::thread::JoinHandle;

use anyhow::{bail, Result};

use crate::pattern::Pattern;
use crate::term::ParserHandle;
```

And the type:

```rust
/// What ended a wait, and which state the capture should use.
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
        let thread = {
            let (acc, stop, over) = (Arc::clone(&acc), Arc::clone(&stop), Arc::clone(&over_budget));
            std::thread::spawn(move || {
                let mut last_gen = u64::MAX;
                while !stop.load(Ordering::Relaxed) {
                    let gen = handle.generation();
                    let now = Instant::now();
                    let (lock, cvar) = &*acc;
                    let mut a = lock.lock().unwrap();
                    if gen != last_gen {
                        // Something arrived: pay for the conversion.
                        last_gen = gen;
                        let (grid, caret, _) = handle.snapshot();
                        a.observe(grid, caret, now);
                    } else {
                        // Nothing arrived, but a pending state may have earned
                        // promotion, so the accumulator still needs a tick.
                        let (grid, caret) = match a.newest() {
                            Some(s) => (s.grid.clone(), s.caret),
                            None => {
                                drop(a);
                                std::thread::sleep(sample);
                                continue;
                            }
                        };
                        a.observe(grid, caret, now);
                    }
                    if a.bytes() > max_bytes {
                        over.store(true, Ordering::Relaxed);
                        cvar.notify_all();
                        return;
                    }
                    cvar.notify_all();
                    drop(a);
                    std::thread::sleep(sample);
                }
            })
        };
        Sampler { acc, stop, thread: Some(thread), over_budget }
    }

    pub fn states(&self) -> MutexGuard<'_, StateAccumulator> {
        self.acc.0.lock().unwrap()
    }

    pub fn over_budget(&self) -> bool {
        self.over_budget.load(Ordering::Relaxed)
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
    /// from ending this wait.
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
                bail!("recording exceeded the capture memory budget; raise max_capture_mb, or raise persist_ms to commit fewer states");
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
                let idx = a.force_commit(now).unwrap_or_else(|| {
                    a.committed().len().saturating_sub(1)
                });
                return Ok(WaitOutcome { state: idx, hit_cap: false });
            }
            if elapsed >= timeout {
                if let Some(p) = want {
                    let seen = a
                        .newest()
                        .map(|s| crate::pattern::screen_text(&s.grid))
                        .unwrap_or_default();
                    bail!(
                        "await pattern {:?} never matched within {}ms\n--- last screen ---\n{seen}",
                        p.source(),
                        timeout.as_millis()
                    );
                }
                let idx = a
                    .force_commit(now)
                    .unwrap_or_else(|| a.committed().len().saturating_sub(1));
                return Ok(WaitOutcome { state: idx, hit_cap: true });
            }
            let nap = Duration::from_millis(2).min(timeout.saturating_sub(elapsed)).max(Duration::from_millis(1));
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
```

- [ ] **Step 4: Add the two supporting accessors**

`Sampler::wait` and the thread need the newest state (pending or committed), and the error needs the pattern text. Add to `impl StateAccumulator`:

```rust
    /// The newest screen, pending or committed — what "the screen right now" means.
    pub fn newest(&self) -> Option<&State> {
        self.pending.as_ref().or_else(|| self.committed.last())
    }
```

And to `impl Pattern` in `src/pattern.rs` (plus store it in `Pattern::new`):

```rust
    /// The pattern as written, for error messages.
    pub fn source(&self) -> &str {
        &self.source
    }
```

Add `source: String` to the `Pattern` struct and set `source: find.to_string()` in `new`.

- [ ] **Step 5: Run them to verify they pass**

Run: `CARGO_BUILD_JOBS=4 cargo test --lib sampler:: && CARGO_BUILD_JOBS=4 cargo clippy --all-targets`
Expected: 6 accumulator + 5 PTY tests pass, no warnings.

- [ ] **Step 6: Commit**

```bash
git add src/sampler.rs src/pattern.rs
git commit -m "feat(sampler): sampling thread and the two-phase wait"
```

---

## Task 5: `assemble()` — frames from states and marks

Pure, portable, no clock and no rendering: this is where the frame taxonomy lives.

**Files:**
- Create: `src/assemble.rs`
- Modify: `src/lib.rs`

**Interfaces:**
- Consumes: nothing from other tasks (deliberately — it takes plain timestamps)
- Produces:
  - `pub enum FrameSource { State(usize), Card(usize), Reuse }`
  - `pub enum FrameKind { InputDriven, AppDriven, Card }`
  - `pub struct FrameSpec { pub source: FrameSource, pub kind: FrameKind, pub hold_cs: u16, pub mouse: Option<(u32, u32)>, pub scene: usize }`
  - `pub enum Mark { Input(InputMark), Card { scene: usize, hold_cs: u16 }, MouseMove { scene: usize, mouse: (u32, u32), hold_cs: u16 } }`
  - `pub struct InputMark { pub t: Instant, pub scene: usize, pub settled: usize, pub authored_cs: u16, pub mouse: Option<(u32, u32)>, pub animated: bool }`
  - `pub fn assemble(state_times: &[Instant], end: Instant, marks: &[Mark], min_cs: u16) -> Vec<FrameSpec>`

- [ ] **Step 1: Register the module**

In `src/lib.rs` (portable, no `cfg`):

```rust
pub mod assemble;
```

- [ ] **Step 2: Write the failing tests**

Create `src/assemble.rs` with the types, a stubbed `assemble`, and:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    fn ms(n: u64) -> Duration {
        Duration::from_millis(n)
    }

    fn input(t: Instant, settled: usize, authored_cs: u16) -> Mark {
        Mark::Input(InputMark {
            t,
            scene: 0,
            settled,
            authored_cs,
            mouse: None,
            animated: false,
        })
    }

    #[test]
    fn the_settled_state_carries_the_authored_duration() {
        let t0 = Instant::now();
        let times = vec![t0 + ms(10)];
        let marks = vec![input(t0, 0, 28)];
        let f = assemble(&times, t0 + ms(500), &marks, 1);
        assert_eq!(f.len(), 1);
        assert_eq!(f[0].hold_cs, 28, "authored, not measured");
        assert!(matches!(f[0].kind, FrameKind::InputDriven));
    }

    #[test]
    fn a_later_state_is_app_driven_at_measured_time() {
        let t0 = Instant::now();
        // settled at +10ms; the app moves again at +200ms and holds to +600ms
        let times = vec![t0 + ms(10), t0 + ms(200)];
        let marks = vec![input(t0, 0, 28)];
        let f = assemble(&times, t0 + ms(600), &marks, 1);
        assert_eq!(f.len(), 2);
        assert_eq!(f[0].hold_cs, 28);
        assert!(matches!(f[1].kind, FrameKind::AppDriven));
        assert_eq!(f[1].hold_cs, 40, "600ms - 200ms = 400ms = 40cs");
    }

    #[test]
    fn a_state_before_the_settled_one_is_also_app_driven() {
        let t0 = Instant::now();
        // a two-stage redraw: stage 1 at +10ms, the awaited stage 2 at +400ms
        let times = vec![t0 + ms(10), t0 + ms(400)];
        let marks = vec![input(t0, 1, 28)];
        let f = assemble(&times, t0 + ms(900), &marks, 1);
        assert_eq!(f.len(), 2);
        assert!(matches!(f[0].kind, FrameKind::AppDriven));
        assert_eq!(f[0].hold_cs, 39, "400ms - 10ms = 390ms = 39cs");
        assert!(matches!(f[1].kind, FrameKind::InputDriven));
        assert_eq!(f[1].hold_cs, 28);
    }

    #[test]
    fn states_are_scoped_to_the_input_that_owns_them() {
        let t0 = Instant::now();
        let times = vec![t0 + ms(10), t0 + ms(510)];
        let marks = vec![input(t0, 0, 9), input(t0 + ms(500), 1, 28)];
        let f = assemble(&times, t0 + ms(900), &marks, 1);
        assert_eq!(f.len(), 2, "one input-driven frame each, none stolen");
        assert_eq!(f[0].hold_cs, 9);
        assert_eq!(f[1].hold_cs, 28);
    }

    #[test]
    fn an_animated_scene_measures_every_frame() {
        let t0 = Instant::now();
        let times = vec![t0 + ms(10), t0 + ms(110), t0 + ms(210)];
        let marks = vec![Mark::Input(InputMark {
            t: t0,
            scene: 0,
            settled: 0,
            authored_cs: 99,
            mouse: None,
            animated: true,
        })];
        let f = assemble(&times, t0 + ms(310), &marks, 1);
        assert_eq!(f.len(), 3);
        assert!(f.iter().all(|s| matches!(s.kind, FrameKind::AppDriven)));
        assert!(f.iter().all(|s| s.hold_cs == 10), "all measured: {:?}", f.iter().map(|s| s.hold_cs).collect::<Vec<_>>());
    }

    #[test]
    fn mouse_move_frames_reuse_the_screen() {
        let t0 = Instant::now();
        let times = vec![t0 + ms(10)];
        let marks = vec![
            input(t0, 0, 28),
            Mark::MouseMove { scene: 0, mouse: (5, 5), hold_cs: 4 },
        ];
        let f = assemble(&times, t0 + ms(500), &marks, 1);
        assert_eq!(f.len(), 2);
        assert!(matches!(f[1].source, FrameSource::Reuse));
        assert_eq!(f[1].mouse, Some((5, 5)));
        assert_eq!(f[1].hold_cs, 4);
    }

    #[test]
    fn cards_are_emitted_in_order() {
        let t0 = Instant::now();
        let times = vec![t0 + ms(10)];
        let marks = vec![
            Mark::Card { scene: 0, hold_cs: 200 },
            input(t0, 0, 28),
        ];
        let f = assemble(&times, t0 + ms(500), &marks, 1);
        assert!(matches!(f[0].source, FrameSource::Card(0)));
        assert_eq!(f[0].hold_cs, 200);
    }

    #[test]
    fn min_cs_clamps_a_very_short_measured_frame() {
        let t0 = Instant::now();
        let times = vec![t0 + ms(10), t0 + ms(20)];
        let marks = vec![input(t0, 0, 28)];
        // 10ms measured = 1cs, but max_fps may demand 4cs
        let f = assemble(&times, t0 + ms(30), &marks, 4);
        assert_eq!(f[1].hold_cs, 4);
    }
}
```

- [ ] **Step 3: Run them to verify they fail**

Run: `CARGO_BUILD_JOBS=4 cargo test --lib assemble::`
Expected: FAIL — `not implemented`.

- [ ] **Step 4: Implement**

```rust
//! Turn the sampler's state log and the driver's input marks into frames.
//! Pure: no PTY, no clock, no rendering — every rule here is unit-tested.

use std::time::{Duration, Instant};

/// Where a frame's pixels come from.
#[derive(Debug)]
pub enum FrameSource {
    /// A committed state, by index into the state log.
    State(usize),
    /// A synthetic title card, by scene index.
    Card(usize),
    /// Reuse the previous frame's screen — pointer-only motion.
    Reuse,
}

#[derive(Debug)]
pub enum FrameKind {
    /// The settled result of an input: paced by the script.
    InputDriven,
    /// The app moving on its own: paced by the clock.
    AppDriven,
    Card,
}

#[derive(Debug)]
pub struct FrameSpec {
    pub source: FrameSource,
    pub kind: FrameKind,
    pub hold_cs: u16,
    pub mouse: Option<(u32, u32)>,
    pub scene: usize,
}

/// One input the driver sent, and the state the wait settled on.
pub struct InputMark {
    pub t: Instant,
    pub scene: usize,
    pub settled: usize,
    pub authored_cs: u16,
    pub mouse: Option<(u32, u32)>,
    pub animated: bool,
}

pub enum Mark {
    Input(InputMark),
    Card { scene: usize, hold_cs: u16 },
    MouseMove { scene: usize, mouse: (u32, u32), hold_cs: u16 },
}

fn cs(d: Duration, min_cs: u16) -> u16 {
    let raw = (d.as_millis() / 10) as u16;
    raw.max(1).max(min_cs)
}

pub fn assemble(
    state_times: &[Instant],
    end: Instant,
    marks: &[Mark],
    min_cs: u16,
) -> Vec<FrameSpec> {
    // How long state i was on screen: until the next state, or until `end`.
    let measured = |i: usize| -> Duration {
        let next = state_times.get(i + 1).copied().unwrap_or(end);
        next.saturating_duration_since(state_times[i])
    };

    // The next input's timestamp bounds which states this input owns.
    let next_input_t: Vec<Instant> = {
        let ts: Vec<Instant> = marks
            .iter()
            .filter_map(|m| match m {
                Mark::Input(i) => Some(i.t),
                _ => None,
            })
            .collect();
        ts.iter()
            .enumerate()
            .map(|(k, _)| ts.get(k + 1).copied().unwrap_or(end))
            .collect()
    };

    let mut out = Vec::new();
    let mut input_ord = 0usize;
    for m in marks {
        match m {
            Mark::Card { scene, hold_cs } => out.push(FrameSpec {
                source: FrameSource::Card(*scene),
                kind: FrameKind::Card,
                hold_cs: (*hold_cs).max(min_cs),
                mouse: None,
                scene: *scene,
            }),
            Mark::MouseMove { scene, mouse, hold_cs } => out.push(FrameSpec {
                source: FrameSource::Reuse,
                kind: FrameKind::AppDriven,
                hold_cs: (*hold_cs).max(min_cs),
                mouse: Some(*mouse),
                scene: *scene,
            }),
            Mark::Input(i) => {
                let upto = next_input_t[input_ord];
                input_ord += 1;
                for (idx, t) in state_times.iter().enumerate() {
                    if *t < i.t || *t >= upto {
                        continue;
                    }
                    let is_settled = idx == i.settled && !i.animated;
                    out.push(FrameSpec {
                        source: FrameSource::State(idx),
                        kind: if is_settled { FrameKind::InputDriven } else { FrameKind::AppDriven },
                        hold_cs: if is_settled {
                            i.authored_cs.max(min_cs)
                        } else {
                            cs(measured(idx), min_cs)
                        },
                        mouse: i.mouse,
                        scene: i.scene,
                    });
                }
            }
        }
    }
    out
}
```

- [ ] **Step 5: Run them to verify they pass**

Run: `CARGO_BUILD_JOBS=4 cargo test --lib assemble:: && CARGO_BUILD_JOBS=4 cargo clippy --all-targets`
Expected: 8 passed, no warnings.

- [ ] **Step 6: Commit**

```bash
git add src/lib.rs src/assemble.rs
git commit -m "feat(assemble): frames from state log and input marks"
```

---

## Task 6: Config — new timing keys, `await`, `animated`

Additive: `settle_ms` and `react_ms` stay for now so `record.rs` still compiles. Task 7 removes them.

**Files:**
- Modify: `src/config.rs`

**Interfaces:**
- Produces (on `RecordConfig`): `sample_ms`, `change_ms`, `stable_ms`, `persist_ms`, `wait_cap_ms`, `await_ms`, `realtime`, `max_capture_mb` — all `u64` except `realtime: bool`
- Produces (on `Scene`): `pub await_spec: Option<AwaitSpec>` (serde-renamed to `await`), `pub animated: bool`
- Produces: `pub enum AwaitSpec { Text(String), Scoped { find: String, row: Option<i32> } }`
- Produces: `pub fn Scene::pattern(&self, rows: u32) -> Result<Option<Pattern>>`

- [ ] **Step 1: Write the failing tests**

Append to the tests module in `src/config.rs` (create one if absent):

```rust
#[cfg(test)]
mod await_tests {
    use super::*;

    fn cfg(scene: &str) -> RecordConfig {
        let text = format!(
            "launch = 'true'\ncols = 10\nrows = 4\n[[scene]]\n{scene}\n"
        );
        toml::from_str(&text).unwrap()
    }

    #[test]
    fn timing_defaults_match_the_spec() {
        let c = cfg("keys = ['a']");
        assert_eq!(c.sample_ms, 10);
        assert_eq!(c.change_ms, 150);
        assert_eq!(c.stable_ms, 40);
        assert_eq!(c.persist_ms, 40);
        assert_eq!(c.wait_cap_ms, 3000);
        assert_eq!(c.await_ms, 5000);
        assert_eq!(c.max_capture_mb, 256);
        assert!(!c.realtime);
    }

    #[test]
    fn await_accepts_a_bare_string() {
        let c = cfg("keys = ['t']\nawait = 'theme: light'");
        let p = c.scenes[0].pattern(c.rows).unwrap().unwrap();
        assert_eq!(p.row(), None);
    }

    #[test]
    fn await_accepts_a_row_scoped_table() {
        let c = cfg("keys = ['t']\nawait = { find = 'theme: light', row = -1 }");
        let p = c.scenes[0].pattern(c.rows).unwrap().unwrap();
        assert_eq!(p.row(), Some(-1));
    }

    #[test]
    fn a_bad_regex_fails_at_load_not_at_runtime() {
        let c = cfg("keys = ['t']\nawait = 'unclosed('");
        assert!(c.scenes[0].pattern(c.rows).is_err());
    }

    #[test]
    fn a_row_outside_the_screen_is_rejected() {
        let c = cfg("keys = ['t']\nawait = { find = 'x', row = 9 }");
        let err = c.scenes[0].pattern(c.rows).unwrap_err().to_string();
        assert!(err.contains("row"), "error should name the row: {err}");
    }

    #[test]
    fn animated_defaults_to_false() {
        let c = cfg("keys = ['a']");
        assert!(!c.scenes[0].animated);
    }
}
```

- [ ] **Step 2: Run them to verify they fail**

Run: `CARGO_BUILD_JOBS=4 cargo test --lib config::await_tests`
Expected: FAIL — unknown fields / no method `pattern`.

- [ ] **Step 3: Implement**

Add to `RecordConfig`, beside `startup_ms`:

```rust
    /// Grid snapshot interval.
    #[serde(default = "d_sample")]
    pub sample_ms: u64,
    /// Grace for the app's first grid change after an input. Only spent in full
    /// by an input that draws nothing.
    #[serde(default = "d_change")]
    pub change_ms: u64,
    /// How long the grid must hold still to call a screen settled (pacing).
    #[serde(default = "d_stable")]
    pub stable_ms: u64,
    /// How long a state must persist to earn a frame (assembly).
    #[serde(default = "d_persist")]
    pub persist_ms: u64,
    /// Bound on a wait with no `await`.
    #[serde(default = "d_wait_cap")]
    pub wait_cap_ms: u64,
    /// Default `await` timeout.
    #[serde(default = "d_await")]
    pub await_ms: u64,
    /// Play the whole recording at measured time.
    #[serde(default)]
    pub realtime: bool,
    /// Backstop on accumulated grid memory.
    #[serde(default = "d_max_mb")]
    pub max_capture_mb: u64,
```

with:

```rust
fn d_sample() -> u64 { 10 }
fn d_change() -> u64 { 150 }
fn d_stable() -> u64 { 40 }
fn d_persist() -> u64 { 40 }
fn d_wait_cap() -> u64 { 3000 }
fn d_await() -> u64 { 5000 }
fn d_max_mb() -> u64 { 256 }
```

Add to `Scene`:

```rust
    /// What this scene's finished screen looks like. Declaring it replaces the
    /// timing guess with a fact, and a failure aborts the run.
    #[serde(default, rename = "await")]
    pub await_spec: Option<AwaitSpec>,
    /// Per-scene `await` timeout override.
    #[serde(default)]
    pub await_ms: Option<u64>,
    /// This screen never holds still (spinner, clock, progress bar).
    #[serde(default)]
    pub animated: bool,
```

And the spec type plus resolver:

```rust
/// `await = "text"` or `await = { find = "text", row = -1 }`.
#[derive(Deserialize)]
#[serde(untagged)]
pub enum AwaitSpec {
    Text(String),
    Scoped { find: String, row: Option<i32> },
}

impl Scene {
    /// Compile this scene's `await`, validating the row against the screen.
    /// Called at load so a bad pattern fails in milliseconds, not minutes in.
    pub fn pattern(&self, rows: u32) -> anyhow::Result<Option<crate::pattern::Pattern>> {
        let (find, row) = match &self.await_spec {
            None => return Ok(None),
            Some(AwaitSpec::Text(t)) => (t.as_str(), None),
            Some(AwaitSpec::Scoped { find, row }) => (find.as_str(), *row),
        };
        if let Some(r) = row {
            let rows = rows as i32;
            if r >= rows || r < -rows {
                anyhow::bail!("await row {r} is outside the {rows}-row screen");
            }
        }
        Ok(Some(crate::pattern::Pattern::new(find, row)?))
    }
}
```

- [ ] **Step 4: Run them to verify they pass**

Run: `CARGO_BUILD_JOBS=4 cargo test && CARGO_BUILD_JOBS=4 cargo clippy --all-targets`
Expected: all pass, no warnings.

- [ ] **Step 5: Commit**

```bash
git add src/config.rs
git commit -m "feat(config): sampling timings, await patterns, animated scenes"
```

---

## Task 7: Rewire `record.rs`, delete `settle()`

The one big task: driving separates from rendering, and the old machinery goes.

**Files:**
- Modify: `src/record.rs`, `src/term.rs`, `src/config.rs`

- [ ] **Step 1: Rewrite the recorder's core**

`Recorder` loses `renderer`, `chrome`, `last_grid`, `caret`, `frames` from the capture path and gains:

```rust
struct Recorder<'a> {
    cfg: &'a RecordConfig,
    sampler: Sampler,
    term: Term,
    marks: Vec<Mark>,
    patterns: Vec<Option<Pattern>>, // one per scene, compiled at startup
    last_mouse: Option<(u32, u32)>,
    min_cs: u16,
}
```

`capture()` becomes:

```rust
    /// Wait for this input's result, then mark it on the timeline.
    fn capture(&mut self, scene: usize, at: Instant, authored_cs: u16, want: Option<&Pattern>)
        -> Result<()>
    {
        let animated = self.cfg.realtime || self.cfg.scenes[scene].animated;
        let out = if animated {
            std::thread::sleep(Duration::from_millis(authored_cs as u64 * 10));
            let mut a = self.sampler.states();
            WaitOutcome { state: a.force_commit(Instant::now())
                .unwrap_or_else(|| a.committed().len().saturating_sub(1)), hit_cap: false }
        } else {
            let timeout = match want {
                Some(_) => Duration::from_millis(
                    self.cfg.scenes[scene].await_ms.unwrap_or(self.cfg.await_ms)),
                None => Duration::from_millis(self.cfg.wait_cap_ms),
            };
            self.sampler.wait(
                want,
                Duration::from_millis(self.cfg.change_ms),
                Duration::from_millis(self.cfg.stable_ms),
                timeout,
            ).with_context(|| format!("scene {scene}"))?
        };
        self.marks.push(Mark::Input(InputMark {
            t: at, scene, settled: out.state, authored_cs,
            mouse: self.last_mouse_for_frame, animated,
        }));
        Ok(())
    }
```

`await` is passed only for a scene's **final** send — the last key, last character, the mouse-up for `click`/`drag`, the last wheel sequence for `scroll`. Pass `None` for every earlier send in the scene.

`push()` is deleted; `move_to` pushes `Mark::MouseMove`; `push_card` pushes `Mark::Card`.

- [ ] **Step 2: Render after assembly, in `run()`**

```rust
    let state_times: Vec<Instant> = { let a = rec.sampler.states(); a.committed().iter().map(|s| s.t).collect() };
    let end = Instant::now();
    let specs = assemble(&state_times, end, &rec.marks, rec.min_cs);

    let renderer = Renderer::new(cfg.font_px);
    let chrome = /* as today */;
    let mut frames: Vec<Frame> = Vec::with_capacity(specs.len());
    let mut last_image: Option<RgbaImage> = None;
    let acc = rec.sampler.states();
    for spec in &specs {
        let mut img = match &spec.source {
            FrameSource::State(i) => renderer.render(&acc.committed()[*i].grid, cfg.cols, cfg.rows),
            FrameSource::Card(s) => frame::render_card(&renderer, cfg.cols, cfg.rows,
                cfg.scenes[*s].card.as_ref().expect("card mark implies a card"),
                cfg.card_font_px, cfg.card_subtitle_px)?,
            FrameSource::Reuse => last_image.clone().expect("reuse needs a previous frame"),
        };
        // pointer / caret overlay exactly as `push()` did today
        // ...
        last_image = Some(img.clone());
        frames.push(Frame { image: if chrome.is_active() { chrome.matte(&renderer, &img) } else { img },
                            hold_cs: spec.hold_cs });
    }
```

- [ ] **Step 3: Delete the old machinery**

- `src/term.rs`: delete `pub fn settle(&mut self, ...)` and the `last_send` / `awaiting_reply` fields, plus the two `settle` PTY tests (their scenarios now live in `sampler::pty_tests`, Task 4).
- `src/config.rs`: delete `settle_ms`, `react_ms`, `default_settle`, `default_react`.

- [ ] **Step 4: Run the full suite**

Run: `CARGO_BUILD_JOBS=4 cargo test && CARGO_BUILD_JOBS=4 cargo clippy --all-targets`
Expected: all green. `tests/record_smoke.rs` must still pass — if it sets `settle_ms`, update it.

- [ ] **Step 5: Commit**

```bash
git add src/record.rs src/term.rs src/config.rs tests/
git commit -m "feat(record): drive-capture-assemble, and settle() is gone"
```

---

## Task 8: Frame manifest for `--dump-png`

**Files:** Modify `src/record.rs`

- [ ] **Step 1: Write the failing test**

In `tests/record_smoke.rs`, extend the smoke test to pass a `--dump-png` dir and assert `manifest.tsv` exists with a header and one line per frame:

```rust
let manifest = std::fs::read_to_string(dir.join("manifest.tsv")).expect("manifest written");
let lines: Vec<&str> = manifest.lines().collect();
assert_eq!(lines[0], "frame\tscene\tkind\thold_cs");
assert_eq!(lines.len() - 1, frame_count, "one row per frame");
```

- [ ] **Step 2: Run it to verify it fails**

Run: `CARGO_BUILD_JOBS=4 cargo test --test record_smoke`
Expected: FAIL — no such file.

- [ ] **Step 3: Implement**

Beside the PNG dump loop in `run()`:

```rust
    if let Some(d) = dump_png {
        let mut man = String::from("frame\tscene\tkind\thold_cs\n");
        for (i, spec) in specs.iter().enumerate() {
            let kind = match spec.kind {
                FrameKind::InputDriven => "input-driven",
                FrameKind::AppDriven => "app-driven",
                FrameKind::Card => "card",
            };
            man.push_str(&format!("{i:04}\t{}\t{kind}\t{}\n", spec.scene, spec.hold_cs));
        }
        std::fs::write(d.join("manifest.tsv"), man).ok();
    }
```

- [ ] **Step 4: Run it to verify it passes**

Run: `CARGO_BUILD_JOBS=4 cargo test --test record_smoke`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src/record.rs tests/record_smoke.rs
git commit -m "feat(record): frame manifest beside --dump-png"
```

---

## Task 9: Docs, man page, and the regression gate

**Files:** Modify `README.md`, `man/ansidrama.1`; create `docs/regression-gate.md`

- [ ] **Step 1: Document the new config**

In `README.md`'s record-config section, remove `settle_ms`/`react_ms` and document the nine new keys with the spec's defaults verbatim, plus `await` (both forms) and `animated`. State the migration in one line: *"`settle_ms` and `react_ms` are gone; delete them. Configs carrying them now fail to parse."*

- [ ] **Step 2: Mirror it in the man page**

Add the same keys to `man/ansidrama.1` in the existing option-list style.

- [ ] **Step 3: Write the regression-gate procedure**

Create `docs/regression-gate.md` recording the spec's gate so it is repeatable:

```markdown
# Capture regression gate

Compare a recording against the pre-redesign output with the app-driven path
suppressed, so the frame sequence is comparable index-for-index.

1. Record with 0.2.0 (`git stash` or a checkout of `2e5195f`), `--dump-png out-old/`.
2. Record with HEAD and `persist_ms = 3600000` so no state can ever be
   committed by persistence alone — every frame is then input-driven, exactly
   as 0.2.0 produced. `--dump-png out-new/`.
3. Compare: `for f in out-old/*.png; do cmp -s "$f" "out-new/$(basename $f)" || echo "DIFFERS: $f"; done`

Expect no output. Any difference is either a fixed frame or a regression —
inspect it.

**What this does not cover:** the default configuration, and therefore the
app-driven path. Those rules are covered by the `assemble` unit tests.
```

- [ ] **Step 4: Verify**

Run: `CARGO_BUILD_JOBS=4 cargo test && CARGO_BUILD_JOBS=4 cargo clippy --all-targets`
Expected: green.

- [ ] **Step 5: Commit**

```bash
git add README.md man/ansidrama.1 docs/regression-gate.md
git commit -m "docs: sampling config, await, and the regression gate"
```

---

## Self-Review Notes

**Spec coverage:** sampling (T3/T4), `change_ms` two-phase wait (T4), `await` + row scoping (T1/T4/T6), hard failure with last screen (T4), patterns compiled at load (T6), `animated`/`realtime` (T6/T7), assembly rules (T5), commit-on-settled invariant (T3 `force_commit`, used by T4 `wait`), `max_capture_mb` (T3 `bytes`, T4 `over_budget`), manifest (T8), migration + gate (T9), ported `settle` tests (T4).

**Known gap, deliberately deferred:** the spec's "child exits early → hard error naming remaining scenes" is not its own task. Fold it into Task 7 when wiring `run()`: check `sampler`/`term` EOF after each scene and `bail!` if scenes remain.

**Type consistency:** `Pattern::new(&str, Option<i32>)` and `Pattern::row()` are used identically in T1, T4 and T6. `WaitOutcome { state, hit_cap }` is produced in T4 and consumed in T7. `Mark`/`InputMark`/`FrameSpec` are defined in T5 and consumed in T7/T8. `StateAccumulator::{observe, force_commit, committed, last_change, bytes, newest}` are defined in T3/T4 and consumed in T4/T7.
