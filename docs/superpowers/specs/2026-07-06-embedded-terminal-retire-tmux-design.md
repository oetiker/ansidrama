# Design: retire tmux — embed the terminal in `record`

Date: 2026-07-06
Status: approved (pending spec review)
Sub-project 1 of 2 (this must land before the release-trimmings sub-project, so
packages / README / man page / demo all describe a dependency-free tool).

## Overview

`ansidrama record` currently shells out to `tmux` for every step of its work: it
uses tmux as a headless terminal emulator with a scriptable scrape API. This
design replaces that with an **in-process terminal** — spawn the child on a PTY,
feed its output to a pure-Rust VT parser that maintains a screen grid + cursor,
and inject input by writing bytes to the PTY master.

Result: `ansidrama` becomes a **single static binary with zero runtime
dependencies**. The README's "one optional runtime dependency: tmux" caveat, the
deb/rpm `Recommends: tmux`, and the tmux truecolor hack all disappear, and
capture becomes more deterministic.

Only `record`'s capture path changes. `encode` and its `.ansi` /
`grid::parse_grid` path are untouched.

## Goals

- Remove the `tmux` runtime dependency entirely from `record`.
- Preserve the existing `record` config surface — user scripts keep working
  verbatim, including tmux-style key names (`Down`, `Enter`, `C-c`, `F10`, …).
- Native truecolor (24-bit) capture, no 256-colour quantisation, no
  `terminal-overrides` hack.
- Lean, pure-Rust dependency stack (see Dependencies).
- Test-driven: the capture/parse path is unit-testable without a real PTY.

## Non-goals

- Windows support for `record` (needs a real PTY; already unix-only in practice).
- Inline images (sixel/kitty graphics) — out of scope; ansidrama rasterizes a
  cell grid, and the current tmux path does not capture images either.
- Any change to `encode`, title cards, rasterization, or the WebP output.
- The release trimmings (CI, deb/rpm, man page, demo, release workflow) — that is
  sub-project 2, specced separately after this lands.

## Current state (what tmux does today)

From `src/record.rs`, tmux provides exactly these services:

| tmux usage | purpose |
|---|---|
| `new-session -d -s … -x cols -y rows -e K=V … bash -lc "<launch>"` | spawn child in a fixed-size headless terminal, with env |
| `capture-pane -e -p -N` | scrape the coloured cell grid |
| `display-message -p '#{cursor_flag} #{cursor_x} #{cursor_y}'` | query caret position + visibility |
| `send-keys -t … <name>` | inject a named key |
| `send-keys -t … -l <bytes>` | inject literal bytes (typed text, mouse SGR reports, raw escapes) |
| `set-option -ga terminal-overrides ,*:RGB` | force truecolor to survive capture |
| `kill-session`, `wait-for` | teardown / launch sync |

Every one of these has an in-process equivalent.

## Architecture

### New module: `src/term.rs`

Owns the PTY, the child process, and the VT parser. Public surface:

```rust
pub struct Term { /* pty master, child, parser, reader thread handle, … */ }

impl Term {
    /// openpty (rustix), spawn `bash -lc <launch>` via std::process::Command
    /// with a pre_exec hook (setsid + TIOCSCTTY + slave→stdio), env applied,
    /// COLORTERM defaulted to "truecolor". Starts a reader thread draining the
    /// PTY master into the parser.
    pub fn spawn(cols: u16, rows: u16, launch: &str, env: &BTreeMap<String,String>) -> Result<Term>;

    /// Block until the PTY has been idle for `settle` (no new bytes), or until a
    /// hard cap elapses, then return. Drain-until-idle replaces sleep-then-scrape.
    pub fn settle(&mut self, idle: Duration, cap: Duration);

    /// Current screen as ansidrama cells (colours resolved to RGB, wide cells
    /// handled).
    pub fn grid(&self) -> Vec<Vec<Cell>>;

    /// (x, y, visible) caret from the parser's cursor state; 0-based.
    pub fn caret(&self) -> Option<(u32, u32)>;

    /// Write raw bytes to the PTY master (typed text, mouse SGR, escapes).
    pub fn send_bytes(&mut self, bytes: &[u8]) -> Result<()>;

    /// Translate a tmux-style key name to bytes and send it.
    pub fn send_key(&mut self, name: &str) -> Result<()>;
}
```

Internals:
- **PTY**: `rustix_openpty::openpty` → master/slave `OwnedFd`s. Slave becomes the
  child's controlling terminal; master is read/written by us. Winsize set from
  `cols`/`rows` (`TIOCSWINSZ`).
- **Spawn**: `std::process::Command::new("bash").args(["-lc", launch])`, env
  applied, `pre_exec` closure: `setsid()`, set controlling tty on the slave, dup
  slave to stdin/stdout/stderr, close extra fds. `std` owns the `Child` and
  reaping.
- **Reader thread**: reads the master fd into a buffer, feeds `vt100::Parser`,
  and records a "last byte received" timestamp (behind a `Mutex`/`Condvar`) so
  `settle()` can wait for quiescence. EOF (child exit) ends the thread.
- **Testability**: a `Term::feed(&[u8])` (test-only or internal) drives the
  parser directly with no PTY, so grid/caret assertions need no child process.

### New module: `src/keys.rs`

A **tmux-compatible** key-name → bytes table. Covers the names ansidrama’s
README and existing scripts use, plus the common set:

- Named: `Enter`/`Return`, `Tab`, `BTab`/`S-Tab`, `Escape`, `Space`, `BSpace`,
  `Up`/`Down`/`Left`/`Right`, `Home`/`End`, `PageUp`/`PageDown`, `Insert`,
  `Delete`, `F1`–`F12`.
- Modifiers: `C-<x>` (control), `M-<x>` (alt → ESC prefix). `S-` for the shifted
  variants we define (e.g. `S-Tab`).
- Anything already starting with `\x1b` (raw escape) or non-named literal is sent
  as literal bytes — the existing escape-hatch behaviour.

The supported key set is documented (and later lands in the man page). Unknown
names return an error rather than silently doing nothing.

### Changes to `src/record.rs`

- Delete `tmux()`, `send()`, `send_literal()`, `query_caret()`, the SGR helpers'
  tmux plumbing stays (they just produce bytes now), and the tmux session
  lifecycle in `run()`.
- `Recorder` holds a `Term` instead of talking to a global tmux `SESSION`.
- `capture()` → `term.settle(idle, cap)` then `self.last_grid = term.grid();
  self.caret = term.caret();`.
- `send(token)` → if it starts with `\x1b` or is literal text, `term.send_bytes`;
  if it's a named key, `term.send_key`. Mouse SGR and typed chars already produce
  raw bytes and go through `send_bytes`.
- `run()`: replace the tmux new-session/kill-session/wait-for dance with
  `Term::spawn(...)`, the startup settle, the scene loop, then send `quit_keys`
  and drop the `Term` (which closes the master and reaps the child).

### Changes to `src/color.rs`

Add resolution from `vt100::Color` to ansidrama's `(u8,u8,u8)`:
- `Rgb(r,g,b)` → passthrough.
- `Idx(0..=15)` → the existing basic palette.
- `Idx(16..=255)` → standard xterm 256 cube + greyscale ramp.
- `Default` → the configured default fg/bg.

Wide cells: place the glyph in its cell, skip the trailing continuation cell
(`vt100` marks wide cells) so columns stay aligned.

## Timing / quiescence

`settle_ms` is reinterpreted from "sleep this long before scraping" to "consider
the screen settled after this long with no new PTY bytes". `startup_ms` is the
first-paint settle after spawn. A hard cap (a small multiple of `settle_ms`, or a
fixed ceiling) prevents a chatty app (e.g. a spinner) from blocking capture
forever. `max_fps` still clamps the minimum per-frame hold. No config fields are
added or removed.

## Config compatibility

`launch`, `cols`, `rows`, `font_px`, `card_font_px`, `env`, `quit_keys`,
`settle_ms`, `startup_ms`, `type_cs`, `move_cs`, `max_fps`, and all scene actions
keep their meaning and syntax. Key names remain tmux-style. The only observable
change is that tmux need not be installed.

## Error handling

- `Term::spawn` fails with context if `openpty`/spawn fails (e.g. no `bash`).
- `send_key` errors on an unknown key name (fail loud, not silent).
- Reader-thread read errors / child crash surface as a capture error rather than
  a hang; `settle()`'s hard cap bounds any wait.

## Testing (TDD)

Written test-first, in this order:

1. **`keys.rs` unit tests** — name → bytes for the full documented set
   (`Down`→`\x1b[B`, `Enter`→`\r`, `C-c`→`\x03`, `S-Tab`→`\x1b[Z`, `F10`, `M-x`,
   …); unknown name → error.
2. **`term.rs` parser unit tests** via `feed(&[u8])` (no PTY): SGR truecolor,
   256-colour index resolution, bold, cursor move + hide/show, erase, alt-screen
   switch, wide (CJK) cell handling → assert `grid()` cells and `caret()`.
3. **`color.rs` unit tests** — `Idx`/`Rgb`/`Default` → expected RGB.
4. **Real-PTY integration tests** (`#[cfg(unix)]`) — spawn `bash -c 'printf …'`
   emitting known ANSI, `settle()`, assert the captured grid. Uses only
   bash/coreutils (present in CI); no external deps.

A short smoke test (`encode` a title card → non-empty `.webp`) is added for the
release sub-project, but `record` gets the coverage above now.

## Platform

`record` is unix-only (needs a PTY); this matches current reality. `encode`
remains fully portable.

## Dependencies

Add: `vt100` (the `Junyi-99/vt100-rust` `deck` fork — a git dep, API-identical to
crates.io `vt100` 0.16.2 but with VS16 emoji-double-width and wide-char-clear
fixes relevant to `screen_to_grid`), `rustix-openpty` (0.2, pulls `rustix`).
Remove: the runtime need for the `tmux` binary. Measured transitive footprint of
the added stack: ~10 crates total, entirely pure Rust, zero `*-sys`/`cc`/
vendored-C — chosen over `portable-pty` (23 crates) and `alacritty_terminal`
(34+, a full emulator) per the lean/pure-Rust preference. `portable-pty` is the
documented fallback if the hand-rolled `pre_exec` spawn proves fiddly.

The vt100 git dep is pinned by `Cargo.lock` (reproducible builds). Caveat: a
future crates.io publish (out of scope here) would require upstreaming the fixes
or publishing the fork under its own name, since crates.io forbids git deps; the
release binaries/deb/rpm build from source, where a git dep is fine.

## Rollout

Single change set; `record`'s public behaviour and config are unchanged, so no
migration for users. Bump is deferred to the release sub-project (targets
`v0.2.0`, the first tagged, dependency-free release).

## Risks

- **Spawn plumbing** (`pre_exec`: setsid + controlling tty + stdio dup) is the
  fiddliest part — mitigated by the real-PTY integration tests and the
  portable-pty fallback.
- **Emulation gaps** — vt100 covers the cell-grid-relevant modern sequences we
  need; if a specific TUI reveals a gap, address it narrowly (or fall back to a
  fuller emulator) rather than broadly.
- **Quiescence tuning** — a persistently chatty app (e.g. a spinner/clock) could
  stall on the idle wait; the hard cap bounds it.

## Out of scope (sub-project 2 — release trimmings)

CI workflow, `Cross.toml`, deb/rpm via nfpm, `man/ansidrama.1`, `CHANGES.md`,
`Makefile`, the self-referential demo webp, README release/install updates, and
the byonk-style `release.yml`. Specced separately once this lands.
