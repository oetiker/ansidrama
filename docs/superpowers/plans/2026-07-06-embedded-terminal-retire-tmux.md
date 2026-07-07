# Retire tmux — Embedded PTY+VT Terminal Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the `tmux` shell-outs in `record` with an in-process terminal (PTY + VT parser), so `ansidrama` becomes a zero-runtime-dependency static binary.

**Architecture:** A new `term.rs` spawns the child on a PTY (`rustix-openpty`), a reader thread feeds output to a `vt100` parser, and `record` reads the screen grid + caret directly instead of scraping tmux. A new `keys.rs` maps tmux-style key names to bytes. `encode` and its `.ansi`/`parse_grid` path are untouched.

**Tech Stack:** Rust 2021, `vt100` 0.16 (VT parser), `rustix` 1 + `rustix-openpty` 0.2 (PTY + spawn syscalls), `anyhow`.

## Global Constraints

- **Pure-Rust deps only.** New crates: `vt100` (the **`Junyi-99/vt100-rust` `deck` fork** — a git dep, API-identical to crates.io `vt100` 0.16.2 but carrying VS16 emoji-double-width and wide-char-clear fixes that matter for `screen_to_grid`), `rustix = { version = "1", features = ["process", "termios", "stdio"] }`, `rustix-openpty = "0.2"`. No `*-sys`/vendored-C in the tree.
- **vt100 is a git dependency**, pinned by `Cargo.lock` (commit `4bca1b1e` at time of writing) so builds are reproducible. **Caveat for sub-project 2:** crates.io publishing forbids git deps — if we ever `cargo publish`, the fixes must first be upstreamed to `doy/vt100-rust` or the fork published under its own crate name. crates.io publish is already out-of-scope here; the release binaries/deb/rpm build from source in CI, where a git dep is fine.
- **`record` is unix-only** — new PTY code is under `#[cfg(unix)]` where it touches process/fd syscalls; `encode` stays fully portable.
- **Preserve the `record` config surface** — every field in `RecordConfig`/`Scene` keeps its meaning and syntax. Key names stay **tmux-compatible** (`Down`, `Enter`, `C-c`, `F10`, `S-Tab`, `M-x`, …).
- **Cap build/test parallelism to 4 cores** — prefix cargo commands with `CARGO_BUILD_JOBS=4` (this machine is shared).
- **Verified external APIs** (already compile-checked — use exactly these):
  - `rustix_openpty::openpty(termios: Option<&Termios>, winsize: Option<&Winsize>) -> io::Result<Pty>`, `Pty { controller: OwnedFd, user: OwnedFd }`.
  - `rustix::termios::Winsize { ws_row: u16, ws_col: u16, ws_xpixel: u16, ws_ypixel: u16 }`.
  - `rustix::process::setsid() -> io::Result<Pid>`; `rustix::process::ioctl_tiocsctty<Fd: AsFd>(fd) -> io::Result<()>` (note: `process` module, not `termios`).
  - `vt100::Parser::new(rows: u16, cols: u16, scrollback_len: usize)`, `.process(&[u8])`, `.screen() -> &Screen`.
  - `Screen::cell(row: u16, col: u16) -> Option<&Cell>`, `Screen::cursor_position() -> (u16, u16)` (row, col), `Screen::hide_cursor() -> bool`.
  - `Cell::contents() -> &str`, `Cell::is_wide_continuation() -> bool`, `Cell::fgcolor()/bgcolor() -> vt100::Color`, `Cell::bold() -> bool`, `Cell::inverse() -> bool`.
  - `enum vt100::Color { Default, Idx(u8), Rgb(u8, u8, u8) }`.
- **TDD, frequent commits.** Work on branch `feat/embedded-terminal` (already checked out).

## File Structure

- `Cargo.toml` (modify) — add the three deps.
- `src/color.rs` (modify) — add `index_to_rgb` (shared xterm-256 palette) + `vt_color` (vt100::Color → Rgb).
- `src/grid.rs` (modify) — delegate its palette helpers to `color::index_to_rgb` (DRY).
- `src/keys.rs` (create) — `key_bytes(name) -> Result<Vec<u8>>`, tmux-compatible.
- `src/term.rs` (create) — `screen_to_grid`, `screen_caret` (pure), then `Term` (PTY + reader + settle/send).
- `src/record.rs` (modify) — rewrite to drive `Term`; delete all tmux code.
- `src/lib.rs` (modify) — register `keys` + `term` modules; update tmux-mentioning doc.
- `src/config.rs` (modify) — one doc-comment tweak (tmux → terminal).
- `tests/record_smoke.rs` (create) — end-to-end `record` → non-empty WebP.
- `README.md` (modify) — remove the tmux runtime-dependency claims.

---

### Task 1: Colour resolution for vt100 + shared 256 palette

**Files:**
- Modify: `Cargo.toml`
- Modify: `src/color.rs`
- Modify: `src/grid.rs:44-73` (palette helpers)
- Test: `src/color.rs` (inline `#[cfg(test)]`)

**Interfaces:**
- Produces: `color::index_to_rgb(i: u8) -> Rgb`, `color::vt_color(c: vt100::Color, default: Rgb) -> Rgb`.

- [ ] **Step 1: Add the `vt100` dependency (deck fork)**

Run: `CARGO_BUILD_JOBS=4 cargo add vt100 --git https://github.com/Junyi-99/vt100-rust --branch deck`

This writes to `Cargo.toml`:

```toml
vt100 = { git = "https://github.com/Junyi-99/vt100-rust", branch = "deck" }
```

and pins the exact commit in `Cargo.lock`. (API-identical to crates.io `vt100` 0.16.2, plus emoji/wide-char fixes — see Global Constraints.)

- [ ] **Step 2: Write the failing tests**

Append to the `#[cfg(test)] mod tests` block in `src/color.rs`:

```rust
    #[test]
    fn index_cube_endpoints() {
        assert_eq!(index_to_rgb(9), (255, 85, 85)); // bright red (low 16)
        assert_eq!(index_to_rgb(16), (0, 0, 0)); // cube start
        assert_eq!(index_to_rgb(231), (255, 255, 255)); // cube end
        assert_eq!(index_to_rgb(232), (8, 8, 8)); // greyscale ramp start
    }

    #[test]
    fn vt_color_maps_each_variant() {
        assert_eq!(vt_color(vt100::Color::Default, (1, 2, 3)), (1, 2, 3));
        assert_eq!(vt_color(vt100::Color::Rgb(9, 8, 7), (0, 0, 0)), (9, 8, 7));
        assert_eq!(vt_color(vt100::Color::Idx(9), (0, 0, 0)), (255, 85, 85));
    }
```

- [ ] **Step 3: Run tests to verify they fail**

Run: `CARGO_BUILD_JOBS=4 cargo test --lib color:: 2>&1 | tail -20`
Expected: FAIL — `cannot find function 'index_to_rgb'` / `vt_color`.

- [ ] **Step 4: Implement `index_to_rgb` and `vt_color`**

In `src/color.rs`, add after the `Rgb` type alias (before `parse`):

```rust
/// Classic VGA/xterm 16-colour palette (SGR 30–37/90–97 and the low 16 of the
/// 256-colour cube).
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

/// xterm 256-colour index → RGB (16 base + 6×6×6 cube + greyscale ramp).
pub fn index_to_rgb(i: u8) -> Rgb {
    match i {
        0..=15 => PALETTE16[(i & 0x0f) as usize],
        16..=231 => {
            let i = i - 16;
            let steps = [0u8, 95, 135, 175, 215, 255];
            (
                steps[(i / 36) as usize],
                steps[((i / 6) % 6) as usize],
                steps[(i % 6) as usize],
            )
        }
        232..=255 => {
            let v = 8 + 10 * (i - 232);
            (v, v, v)
        }
    }
}

/// Resolve a `vt100` cell colour to RGB; `Default` falls back to `default`.
pub fn vt_color(c: vt100::Color, default: Rgb) -> Rgb {
    match c {
        vt100::Color::Default => default,
        vt100::Color::Idx(i) => index_to_rgb(i),
        vt100::Color::Rgb(r, g, b) => (r, g, b),
    }
}
```

- [ ] **Step 5: Delegate `grid.rs` palette helpers (DRY)**

In `src/grid.rs`, delete the `const PALETTE16: [Rgb; 16] = [...]` block (lines ~25-42) and replace the two helper fns `palette16` and `xterm256` (lines ~50-73) with:

```rust
fn palette16(i: u8) -> Col {
    let (r, g, b) = crate::color::index_to_rgb(i & 0x0f);
    Col::Rgb(r, g, b)
}

/// xterm 256-colour index → RGB.
fn xterm256(i: u8) -> Col {
    let (r, g, b) = crate::color::index_to_rgb(i);
    Col::Rgb(r, g, b)
}
```

- [ ] **Step 6: Run tests to verify they pass**

Run: `CARGO_BUILD_JOBS=4 cargo test --lib 2>&1 | tail -20`
Expected: PASS — all `color::` and `grid::` tests green.

- [ ] **Step 7: Commit**

```bash
git add Cargo.toml Cargo.lock src/color.rs src/grid.rs
git commit -m "feat(color): vt100 colour resolution + shared xterm-256 palette

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

### Task 2: tmux-compatible key-name table

**Files:**
- Create: `src/keys.rs`
- Modify: `src/lib.rs:16-24` (module list)
- Test: `src/keys.rs` (inline `#[cfg(test)]`)

**Interfaces:**
- Produces: `keys::key_bytes(name: &str) -> anyhow::Result<Vec<u8>>`.

- [ ] **Step 1: Register the module**

In `src/lib.rs`, add to the `pub mod` list (keep alphabetical-ish with the others):

```rust
pub mod keys;
```

- [ ] **Step 2: Write the failing tests**

Create `src/keys.rs` with only the test module first:

```rust
//! Translate tmux-style key names to the bytes a terminal application reads.
//! This is the one piece of tmux behaviour we own after retiring the tmux
//! dependency: `send-keys Down` becomes `key_bytes("Down") == b"\x1b[B"`.

use anyhow::{bail, Result};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn named_keys() {
        assert_eq!(key_bytes("Enter").unwrap(), b"\r");
        assert_eq!(key_bytes("Tab").unwrap(), b"\t");
        assert_eq!(key_bytes("Escape").unwrap(), b"\x1b");
        assert_eq!(key_bytes("BSpace").unwrap(), b"\x7f");
        assert_eq!(key_bytes("Up").unwrap(), b"\x1b[A");
        assert_eq!(key_bytes("Down").unwrap(), b"\x1b[B");
        assert_eq!(key_bytes("Right").unwrap(), b"\x1b[C");
        assert_eq!(key_bytes("Left").unwrap(), b"\x1b[D");
        assert_eq!(key_bytes("Home").unwrap(), b"\x1b[H");
        assert_eq!(key_bytes("End").unwrap(), b"\x1b[F");
        assert_eq!(key_bytes("PageUp").unwrap(), b"\x1b[5~");
        assert_eq!(key_bytes("PageDown").unwrap(), b"\x1b[6~");
        assert_eq!(key_bytes("Delete").unwrap(), b"\x1b[3~");
        assert_eq!(key_bytes("F1").unwrap(), b"\x1bOP");
        assert_eq!(key_bytes("F10").unwrap(), b"\x1b[21~");
        assert_eq!(key_bytes("F12").unwrap(), b"\x1b[24~");
        assert_eq!(key_bytes("BTab").unwrap(), b"\x1b[Z");
    }

    #[test]
    fn modifiers() {
        assert_eq!(key_bytes("C-c").unwrap(), vec![0x03]);
        assert_eq!(key_bytes("C-a").unwrap(), vec![0x01]);
        assert_eq!(key_bytes("S-Tab").unwrap(), b"\x1b[Z");
        assert_eq!(key_bytes("M-x").unwrap(), b"\x1bx");
    }

    #[test]
    fn literal_and_raw() {
        assert_eq!(key_bytes("q").unwrap(), b"q");
        // Raw escape sequence passes straight through (the escape hatch).
        assert_eq!(key_bytes("\x1b[<0;1;1M").unwrap(), b"\x1b[<0;1;1M");
    }

    #[test]
    fn unknown_errors() {
        assert!(key_bytes("Nope").is_err());
    }
}
```

- [ ] **Step 3: Run tests to verify they fail**

Run: `CARGO_BUILD_JOBS=4 cargo test --lib keys:: 2>&1 | tail -20`
Expected: FAIL — `cannot find function 'key_bytes'`.

- [ ] **Step 4: Implement `key_bytes`**

Insert above the `#[cfg(test)]` block in `src/keys.rs`:

```rust
/// Named key → bytes. Returns `None` for names handled elsewhere (modifiers,
/// single characters, raw escapes).
fn named(name: &str) -> Option<Vec<u8>> {
    let b: &[u8] = match name {
        "Enter" | "Return" => b"\r",
        "Tab" => b"\t",
        "BTab" => b"\x1b[Z",
        "Escape" | "Esc" => b"\x1b",
        "Space" => b" ",
        "BSpace" | "Backspace" => b"\x7f",
        "Up" => b"\x1b[A",
        "Down" => b"\x1b[B",
        "Right" => b"\x1b[C",
        "Left" => b"\x1b[D",
        "Home" => b"\x1b[H",
        "End" => b"\x1b[F",
        "PageUp" | "PPage" => b"\x1b[5~",
        "PageDown" | "NPage" => b"\x1b[6~",
        "Insert" | "IC" => b"\x1b[2~",
        "Delete" | "DC" => b"\x1b[3~",
        "F1" => b"\x1bOP",
        "F2" => b"\x1bOQ",
        "F3" => b"\x1bOR",
        "F4" => b"\x1bOS",
        "F5" => b"\x1b[15~",
        "F6" => b"\x1b[17~",
        "F7" => b"\x1b[18~",
        "F8" => b"\x1b[19~",
        "F9" => b"\x1b[20~",
        "F10" => b"\x1b[21~",
        "F11" => b"\x1b[23~",
        "F12" => b"\x1b[24~",
        _ => return None,
    };
    Some(b.to_vec())
}

/// `C-<x>` → control byte (letters and `@ [ \ ] ^ _`, plus `C-Space` = NUL).
fn ctrl(rest: &str) -> Result<Vec<u8>> {
    if rest.chars().count() == 1 {
        let c = rest.chars().next().unwrap();
        if c == ' ' {
            return Ok(vec![0x00]);
        }
        let up = c.to_ascii_uppercase() as u8;
        if (b'@'..=b'_').contains(&up) {
            return Ok(vec![up & 0x1f]);
        }
    }
    bail!("unknown control key C-{rest}")
}

/// `S-<x>` → shifted key (`S-Tab` special-cased; letters → uppercase).
fn shift(rest: &str) -> Result<Vec<u8>> {
    if rest.eq_ignore_ascii_case("Tab") {
        return Ok(b"\x1b[Z".to_vec());
    }
    if rest.chars().count() == 1 {
        let c = rest.chars().next().unwrap();
        return Ok(c.to_ascii_uppercase().to_string().into_bytes());
    }
    bail!("unknown shifted key S-{rest}")
}

/// Translate a tmux-style key name to the bytes an app reads. Handles named
/// keys, `C-`/`M-`/`S-` modifiers, single literal characters, and raw escape
/// sequences (anything already starting with ESC passes through unchanged).
pub fn key_bytes(name: &str) -> Result<Vec<u8>> {
    if name.starts_with('\x1b') {
        return Ok(name.as_bytes().to_vec());
    }
    if let Some(rest) = name.strip_prefix("C-") {
        return ctrl(rest);
    }
    if let Some(rest) = name.strip_prefix("M-") {
        let mut v = vec![0x1b];
        v.extend(key_bytes(rest)?);
        return Ok(v);
    }
    if let Some(rest) = name.strip_prefix("S-") {
        return shift(rest);
    }
    if let Some(bytes) = named(name) {
        return Ok(bytes);
    }
    // Exactly one character → its literal UTF-8 bytes.
    let mut it = name.chars();
    if let (Some(c), None) = (it.next(), it.next()) {
        return Ok(c.to_string().into_bytes());
    }
    bail!("unknown key name {name:?}")
}
```

- [ ] **Step 5: Run tests to verify they pass**

Run: `CARGO_BUILD_JOBS=4 cargo test --lib keys:: 2>&1 | tail -20`
Expected: PASS — 4 tests in `keys::tests`.

- [ ] **Step 6: Commit**

```bash
git add src/keys.rs src/lib.rs
git commit -m "feat(keys): tmux-compatible key-name to bytes table

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

### Task 3: Screen → grid + caret conversion (pure, no PTY)

**Files:**
- Create: `src/term.rs` (conversion functions + tests only for now)
- Modify: `src/lib.rs` (module list)
- Test: `src/term.rs` (inline `#[cfg(test)]`)

**Interfaces:**
- Consumes: `color::vt_color`, `grid::Cell`.
- Produces: `term::screen_to_grid(screen: &vt100::Screen, rows: u16, cols: u16) -> Vec<Vec<Cell>>`, `term::screen_caret(screen: &vt100::Screen) -> Option<(u32, u32)>`.

- [ ] **Step 1: Register the module**

In `src/lib.rs`, add to the `pub mod` list:

```rust
pub mod term;
```

- [ ] **Step 2: Write the failing tests**

Create `src/term.rs`:

```rust
//! In-process terminal for `record`: spawn the child on a PTY, feed its output
//! to a `vt100` parser, and read the screen grid + caret directly — no tmux.
//! The screen→grid conversion is kept as free functions so it is unit-testable
//! without a real PTY.

use crate::color::vt_color;
use crate::grid::Cell;

/// Default fg/bg for cells the app left unstyled (mirrors `grid::parse_grid`).
const DEF_FG: crate::color::Rgb = (170, 170, 170);
const DEF_BG: crate::color::Rgb = (0, 0, 0);

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
```

- [ ] **Step 3: Run tests to verify they fail**

Run: `CARGO_BUILD_JOBS=4 cargo test --lib term:: 2>&1 | tail -20`
Expected: FAIL — `cannot find function 'screen_to_grid'` / `screen_caret`.

- [ ] **Step 4: Implement the conversion functions**

In `src/term.rs`, insert after the `DEF_BG` const (before the test module):

```rust
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
```

- [ ] **Step 5: Run tests to verify they pass**

Run: `CARGO_BUILD_JOBS=4 cargo test --lib term:: 2>&1 | tail -20`
Expected: PASS — 4 tests in `term::tests`.

- [ ] **Step 6: Commit**

```bash
git add src/term.rs src/lib.rs
git commit -m "feat(term): pure screen->grid and caret conversion

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

### Task 4: PTY spawn + reader thread + settle

**Files:**
- Modify: `Cargo.toml` (add `rustix`, `rustix-openpty`)
- Modify: `src/term.rs` (add the `Term` struct)
- Test: `src/term.rs` (inline `#[cfg(all(test, unix))]`)

**Interfaces:**
- Consumes: `keys::key_bytes`, `screen_to_grid`, `screen_caret`.
- Produces:
  - `Term::spawn(cols: u16, rows: u16, launch: &str, env: &std::collections::BTreeMap<String, String>) -> anyhow::Result<Term>`
  - `Term::settle(&mut self, idle: Duration, cap: Duration)`
  - `Term::grid(&self) -> Vec<Vec<Cell>>`
  - `Term::caret(&self) -> Option<(u32, u32)>`
  - `Term::send_bytes(&mut self, bytes: &[u8]) -> anyhow::Result<()>`
  - `Term::send_key(&mut self, name: &str) -> anyhow::Result<()>`

- [ ] **Step 1: Add the PTY dependencies**

Edit `Cargo.toml`, `[dependencies]`:

```toml
rustix = { version = "1", features = ["process", "termios", "stdio"] }
rustix-openpty = "0.2"
```

- [ ] **Step 2: Write the failing real-PTY test**

Append to `src/term.rs` (a new module, after the existing `#[cfg(test)] mod tests`):

```rust
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
```

- [ ] **Step 3: Run tests to verify they fail**

Run: `CARGO_BUILD_JOBS=4 cargo test --lib term::pty_tests 2>&1 | tail -20`
Expected: FAIL — `cannot find … Term` / `spawn`.

- [ ] **Step 4: Implement `Term`**

In `src/term.rs`, add the imports at the top (below the existing `use` lines):

```rust
use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::os::fd::AsFd;
use std::os::unix::process::CommandExt;
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Condvar, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
```

Then add the `Term` implementation (before the test modules):

```rust
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
```

- [ ] **Step 5: Run tests to verify they pass**

Run: `CARGO_BUILD_JOBS=4 cargo test --lib term::pty_tests 2>&1 | tail -20`
Expected: PASS — `captures_printed_text` and `send_key_reaches_app`.

- [ ] **Step 6: Run the full lib test suite + clippy**

Run: `CARGO_BUILD_JOBS=4 cargo test --lib 2>&1 | tail -15 && CARGO_BUILD_JOBS=4 cargo clippy --lib -- -D warnings 2>&1 | tail -15`
Expected: all tests PASS; clippy clean.

- [ ] **Step 7: Commit**

```bash
git add Cargo.toml Cargo.lock src/term.rs
git commit -m "feat(term): PTY spawn, reader thread, drain-until-idle settle

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

### Task 5: Rewire `record` to `Term`, delete tmux

**Files:**
- Modify: `src/record.rs` (full rewrite)
- Modify: `src/config.rs:152` (doc comment)
- Modify: `src/lib.rs:1-10` (module doc)
- Create: `tests/record_smoke.rs`

**Interfaces:**
- Consumes: `Term` (Task 4), `keys::key_bytes` (via `Term::send_key`).
- Produces: no new public API — `ansidrama::record` keeps its signature.

- [ ] **Step 1: Write the failing end-to-end test**

Create `tests/record_smoke.rs`:

```rust
//! End-to-end: `record` drives a real embedded terminal (no tmux) and writes a
//! non-empty WebP. Uses only bash/coreutils, so it runs anywhere `record` does.
#![cfg(unix)]

#[test]
fn record_produces_webp() {
    let dir = std::env::temp_dir().join(format!("ansidrama-rec-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let toml = dir.join("drama.toml");
    let out = dir.join("out.webp");
    std::fs::write(
        &toml,
        "launch = \"printf 'HELLO WORLD'; sleep 2\"\n\
         cols = 40\n\
         rows = 6\n\
         [[scene]]\n\
         keys = []\n\
         hold_cs = 20\n",
    )
    .unwrap();

    ansidrama::record(&toml, Some(&out), None).unwrap();

    let len = std::fs::metadata(&out).unwrap().len();
    assert!(len > 0, "webp should be non-empty");

    let _ = std::fs::remove_dir_all(&dir); // the test's own scratch dir
}
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `CARGO_BUILD_JOBS=4 cargo test --test record_smoke 2>&1 | tail -20`
Expected: FAIL — currently `record` shells out to tmux (absent / wrong behaviour). It must pass only after the rewrite.

- [ ] **Step 3: Rewrite `src/record.rs`**

Replace the **entire** contents of `src/record.rs` with:

```rust
//! The `record` command: drive a command in an embedded terminal (its own PTY
//! plus a `vt100` parser — no tmux) and turn a scene script into an animation.
//! Each scene expands into many frames — one per key, one per typed character,
//! one per mouse-cursor cell-step — so keyboard and mouse actions play out step
//! by step. Cursor-only moves reuse the last capture; drags re-capture each step
//! so live UI (e.g. a resize preview) is shown.

use std::path::Path;
use std::time::Duration;

use anyhow::{bail, Context, Result};
use image::RgbaImage;

use crate::config::{min_hold_cs, Action, Card, RecordConfig, Scene};
use crate::cursor;
use crate::encode::{encode_webp, total_ms, Frame};
use crate::frame;
use crate::grid::Cell;
use crate::mouse::{Button, Scroll};
use crate::raster::Renderer;
use crate::term::Term;

/// Cells along the straight line from `a` to `b`, one per step, excluding `a` and
/// including `b` (empty if `a == b`).
fn line_cells(a: (u32, u32), b: (u32, u32)) -> Vec<(u32, u32)> {
    let (x0, y0) = (a.0 as i64, a.1 as i64);
    let (x1, y1) = (b.0 as i64, b.1 as i64);
    let steps = (x1 - x0).abs().max((y1 - y0).abs());
    (1..=steps)
        .map(|i| {
            let x = x0 + (x1 - x0) * i / steps;
            let y = y0 + (y1 - y0) * i / steps;
            (x as u32, y as u32)
        })
        .collect()
}

/// Drives the embedded terminal and accumulates frames.
struct Recorder<'a> {
    cfg: &'a RecordConfig,
    renderer: Renderer,
    idle: Duration,
    cap: Duration,
    startup: Duration,
    min_cs: u16,
    last_grid: Vec<Vec<Cell>>,
    last_mouse: Option<(u32, u32)>,
    caret: Option<(u32, u32)>,
    frames: Vec<Frame>,
    term: Term,
}

impl<'a> Recorder<'a> {
    fn spawn(cfg: &'a RecordConfig) -> Result<Recorder<'a>> {
        let term = Term::spawn(cfg.cols as u16, cfg.rows as u16, &cfg.launch, &cfg.env)
            .context("start embedded terminal")?;
        let idle = Duration::from_millis(cfg.settle_ms);
        let cap = idle.saturating_mul(8).max(Duration::from_millis(1500));
        Ok(Recorder {
            renderer: Renderer::new(cfg.font_px),
            idle,
            cap,
            startup: Duration::from_millis(cfg.startup_ms),
            min_cs: min_hold_cs(cfg.max_fps),
            last_grid: vec![Vec::new()],
            last_mouse: None,
            caret: None,
            frames: Vec::new(),
            cfg,
            term,
        })
    }

    /// Wait for the first paint to settle, then seed the current grid.
    fn seed(&mut self) {
        self.term.settle(self.idle, self.startup.max(self.idle));
        self.last_grid = self.term.grid();
        self.caret = self.term.caret();
    }

    /// Let the screen settle after an input, then re-read the grid and caret.
    fn capture(&mut self) {
        self.term.settle(self.idle, self.cap);
        self.last_grid = self.term.grid();
        self.caret = self.term.caret();
    }

    /// Render the current grid and push a frame. A frame with a mouse position
    /// draws the pointer; otherwise (keyboard/typing frames) it draws the app's
    /// text caret if the cursor is visible.
    fn push(&mut self, mouse: Option<(u32, u32)>, hold_cs: u16) {
        let mut img: RgbaImage =
            self.renderer
                .render(&self.last_grid, self.cfg.cols, self.cfg.rows);
        if self.cfg.cursor {
            if let Some((x, y)) = mouse {
                let (px, py) = self.renderer.cell_origin(x, y);
                cursor::stamp(&mut img, px, py);
            } else if let Some((cx, cy)) = self.caret {
                let cell = self
                    .last_grid
                    .get(cy as usize)
                    .and_then(|r| r.get(cx as usize))
                    .copied()
                    .unwrap_or(Cell {
                        ch: ' ',
                        fg: (0, 0, 0),
                        bg: (255, 255, 255),
                        bold: false,
                    });
                self.renderer
                    .draw_block_cursor(&mut img, cx + 1, cy + 1, &cell);
            }
        }
        self.frames.push(Frame {
            image: img,
            hold_cs: hold_cs.max(self.min_cs),
        });
    }

    /// Push a synthetic card frame (does not disturb the captured terminal state).
    fn push_card(&mut self, card: &Card, hold_cs: u16) -> Result<()> {
        let img = frame::render_card(
            &self.renderer,
            self.cfg.cols,
            self.cfg.rows,
            card,
            self.cfg.card_font_px,
            self.cfg.card_subtitle_px,
        )?;
        self.frames.push(Frame {
            image: img,
            hold_cs: hold_cs.max(self.min_cs),
        });
        Ok(())
    }

    /// Animate the pointer moving from its last position to `target` over the
    /// current (unchanged) screen — one frame per cell. Leaves `last_mouse` set.
    fn move_to(&mut self, target: (u32, u32), move_cs: u16) {
        if let Some(from) = self.last_mouse {
            for cell in line_cells(from, target) {
                self.push(Some(cell), move_cs);
            }
        }
        self.last_mouse = Some(target);
    }

    fn process(&mut self, scene: &Scene) -> Result<()> {
        let type_cs = scene.type_cs.unwrap_or(self.cfg.type_cs);
        let move_cs = scene.move_cs.unwrap_or(self.cfg.move_cs);
        let hold_cs = scene.hold_cs;
        match scene.action()? {
            Action::Card(card) => self.push_card(card, hold_cs)?,

            Action::Keys(keys) => {
                if keys.is_empty() {
                    // "Hold the current screen" — capture once.
                    self.capture();
                    self.push(None, hold_cs);
                } else {
                    let last = keys.len() - 1;
                    for (i, k) in keys.iter().enumerate() {
                        self.term.send_key(k)?;
                        self.capture();
                        self.push(None, if i == last { hold_cs } else { type_cs });
                    }
                }
            }

            Action::Text(s) => {
                let chars: Vec<char> = s.chars().collect();
                let last = chars.len().saturating_sub(1);
                for (i, c) in chars.iter().enumerate() {
                    self.term.send_bytes(c.to_string().as_bytes())?;
                    self.capture();
                    self.push(None, if i == last { hold_cs } else { type_cs });
                }
            }

            Action::Click(c) => {
                let at = (c.x, c.y);
                let b = c.button;
                self.move_to(at, move_cs);
                self.term.send_bytes(sgr(b, at, true).as_bytes())?; // press
                self.capture();
                self.push(Some(at), move_cs);
                self.term.send_bytes(sgr(b, at, false).as_bytes())?; // release
                self.capture();
                self.push(Some(at), hold_cs);
            }

            Action::Drag(d) => {
                let from = (d.from[0], d.from[1]);
                let to = (d.to[0], d.to[1]);
                let b = d.button;
                self.move_to(from, move_cs);
                self.term.send_bytes(sgr(b, from, true).as_bytes())?; // press
                self.capture();
                self.push(Some(from), move_cs);
                for cell in line_cells(from, to) {
                    self.term.send_bytes(sgr_motion(b, cell).as_bytes())?; // drag
                    self.capture();
                    self.push(Some(cell), move_cs);
                }
                self.term.send_bytes(sgr(b, to, false).as_bytes())?; // release
                self.capture();
                self.push(Some(to), hold_cs);
                self.last_mouse = Some(to);
            }

            Action::Scroll(s) => {
                let at = (s.x, s.y);
                self.move_to(at, move_cs);
                let seqs = scroll_sequences(s);
                let last = seqs.len().saturating_sub(1);
                for (i, seq) in seqs.iter().enumerate() {
                    self.term.send_bytes(seq.as_bytes())?;
                    self.capture();
                    self.push(Some(at), if i == last { hold_cs } else { move_cs });
                }
            }
        }
        Ok(())
    }
}

/// SGR press/release for `button` at 1-based `(x, y)`.
fn sgr(b: Button, at: (u32, u32), press: bool) -> String {
    let code = match b {
        Button::Left => 0,
        Button::Middle => 1,
        Button::Right => 2,
    };
    let end = if press { 'M' } else { 'm' };
    format!("\x1b[<{code};{};{}{end}", at.0, at.1)
}
/// SGR drag motion (button held → +32) at `(x, y)`.
fn sgr_motion(b: Button, at: (u32, u32)) -> String {
    let code = match b {
        Button::Left => 0,
        Button::Middle => 1,
        Button::Right => 2,
    } + 32;
    format!("\x1b[<{code};{};{}M", at.0, at.1)
}
fn scroll_sequences(s: &Scroll) -> Vec<String> {
    s.sequences()
}

pub fn run(config_path: &Path, out_override: Option<&Path>, dump_png: Option<&Path>) -> Result<()> {
    let text = std::fs::read_to_string(config_path)
        .with_context(|| format!("read config {}", config_path.display()))?;
    let cfg: RecordConfig = toml::from_str(&text).context("parse record config")?;
    if cfg.scenes.is_empty() {
        bail!("config has no [[scene]] entries");
    }
    let base = config_path.parent().unwrap_or(Path::new("."));
    let out_path = resolve_out(out_override, cfg.out.as_deref(), base, config_path)?;

    let mut rec = Recorder::spawn(&cfg)?;
    rec.seed(); // settle the first paint, seed the grid
    for (i, scene) in cfg.scenes.iter().enumerate() {
        rec.process(scene)?;
        eprintln!("  scene {i:02} → {} frames total", rec.frames.len());
    }

    // Optional per-frame PNG dump for inspection.
    if let Some(d) = dump_png {
        std::fs::create_dir_all(d).ok();
        for (i, f) in rec.frames.iter().enumerate() {
            let _ = f.image.save(d.join(format!("frame{i:04}.png")));
        }
    }

    // Quit the app, take the frames, then drop the terminal (reaps the child).
    for k in &cfg.quit_keys {
        let _ = rec.term.send_key(k);
    }
    let frames = std::mem::take(&mut rec.frames);
    drop(rec);

    if frames.is_empty() {
        bail!("no frames captured");
    }
    let webp = encode_webp(&frames)?;
    if let Some(parent) = out_path.parent() {
        std::fs::create_dir_all(parent).ok();
    }
    std::fs::write(&out_path, &webp).with_context(|| format!("write {}", out_path.display()))?;
    let (w, h) = frames[0].image.dimensions();
    eprintln!(
        "OK: wrote {} ({} frames, {w}x{h}px, {:.1}s loop)",
        out_path.display(),
        frames.len(),
        total_ms(&frames) as f32 / 1000.0
    );
    Ok(())
}

/// Resolve the output path: `-o` wins, else the config's `out` (relative to the
/// config dir), else error.
fn resolve_out(
    out_override: Option<&Path>,
    cfg_out: Option<&str>,
    base: &Path,
    config_path: &Path,
) -> Result<std::path::PathBuf> {
    if let Some(o) = out_override {
        return Ok(o.to_path_buf());
    }
    if let Some(o) = cfg_out {
        return Ok(base.join(o));
    }
    bail!(
        "no output path: pass -o <file> or set `out = ...` in {}",
        config_path.display()
    )
}
```

- [ ] **Step 4: Update the two tmux-mentioning doc comments**

In `src/config.rs`, change the `launch` field doc (line ~152):

```rust
    /// Shell command line launched inside the embedded terminal.
    pub launch: String,
```

In `src/lib.rs`, update the module doc (lines ~4-10) — replace the `record` bullet and the "captured … from tmux" phrasing:

```rust
//! - [`record`] — drive a command in an embedded terminal (its own PTY + VT
//!   parser, no tmux) per a scene script, capture each frame, then hand off to
//!   the same encode path.
```

- [ ] **Step 5: Run the end-to-end test + full suite + clippy**

Run: `CARGO_BUILD_JOBS=4 cargo test --test record_smoke 2>&1 | tail -20`
Expected: PASS — `record_produces_webp`.

Run: `CARGO_BUILD_JOBS=4 cargo test 2>&1 | tail -20 && CARGO_BUILD_JOBS=4 cargo clippy --all-targets -- -D warnings 2>&1 | tail -15 && CARGO_BUILD_JOBS=4 cargo fmt --check`
Expected: all tests PASS; clippy clean; fmt clean.

- [ ] **Step 6: Verify tmux is fully gone from the source**

Run: `grep -rni tmux src/ ; echo "exit: $?"`
Expected: only the `grid.rs` doc comment that cites `tmux capture-pane` as **one example** way to produce a `.ansi` file for `encode` may remain (that is legitimate — `encode` is bring-your-own-frames). No `Command::new("tmux")`, no `SESSION`, no tmux in `record.rs`.

- [ ] **Step 7: Commit**

```bash
git add src/record.rs src/config.rs src/lib.rs tests/record_smoke.rs
git commit -m "feat(record): drive an embedded terminal, remove tmux dependency

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

### Task 6: README — remove the tmux runtime-dependency claims

**Files:**
- Modify: `README.md`

**Interfaces:** none (docs only).

- [ ] **Step 1: Update the intro line**

In `README.md`, change the opening paragraph (lines ~3-5): replace
`asciinema. One optional runtime dependency: `tmux`.`
with:
`asciinema, no runtime dependencies — a single static binary.`

- [ ] **Step 2: Update the Install section**

Replace the runtime-requirement line (~line 142-143):

```md
`record` embeds its own terminal (a PTY + VT parser), so it needs **nothing but
the binary** — same as `encode`. The font (JetBrains Mono, OFL) is bundled.
```

- [ ] **Step 3: Update the "How it compares" closing line**

In the "How it compares" paragraph (~line 152), change
`determinism, sharpness and a near-zero toolchain (just `tmux`).`
to:
`determinism, sharpness and a zero-dependency single binary.`

- [ ] **Step 4: Confirm the `encode` tmux example is intentionally kept**

The `encode` section keeps `tmux capture-pane -e -p -N > 001.ansi` as **one**
example of how to produce a `.ansi` snapshot — `encode` is bring-your-own-frames
and does not depend on tmux at runtime. Leave that mention as-is.

Run: `grep -n tmux README.md`
Expected: only the single `encode` capture example line remains.

- [ ] **Step 5: Commit**

```bash
git add README.md
git commit -m "docs(readme): ansidrama record no longer needs tmux

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

## Self-Review

**Spec coverage:**
- Retire tmux from `record` → Tasks 4-5. ✓
- New `term.rs` (spawn/settle/grid/caret/send) → Tasks 3-4. ✓
- tmux-compatible `keys.rs` → Task 2. ✓
- Config compatibility (fields unchanged; `settle_ms` = idle gap, `startup_ms` = first-paint settle) → Task 5 (`Recorder::spawn`/`seed`/`capture`). ✓
- Native truecolor / colour resolution + wide cells → Tasks 1, 3. ✓
- TDD shape (keys unit, parser unit, color unit, real-PTY integration) → Tasks 1-5. ✓
- Unix-only `record` → `#[cfg(unix)]`/`#[cfg(all(test, unix))]` and `tests/record_smoke.rs` `#![cfg(unix)]`. ✓
- Lean pure-Rust deps (vt100 + rustix + rustix-openpty) → Task 1/4 Cargo edits + Global Constraints. ✓
- README/doc truthfulness after removal → Tasks 5-6. ✓
- Quiescence hard cap → `Term::settle`'s `cap` (Task 4), `cap = idle*8` floored at 1500ms (Task 5).

**Placeholder scan:** No TBD/TODO; every code step shows complete code; every test shows real assertions. ✓

**Type consistency:** `Term::spawn(cols, rows, launch, env)` used identically in Task 4 tests and Task 5 `Recorder::spawn`. `screen_to_grid(screen, rows, cols)` / `screen_caret(screen)` defined in Task 3, called in Task 4. `key_bytes(&str) -> Result<Vec<u8>>` defined Task 2, used via `Term::send_key` Task 4. `vt_color`/`index_to_rgb` defined Task 1, used Task 3. `Cell { ch, fg, bg, bold }` matches `grid::Cell`. ✓

## Execution Handoff

(Filled in when handing off — see below.)
