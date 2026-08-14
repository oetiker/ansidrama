# Changelog

All notable changes to AnsiDrama will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## Unreleased

### New

- **`record`**: `await` — a scene declares what its finished screen looks like, and the recorder waits for that instead of guessing from timing. Either `await = "text"` (whole-screen match) or `await = { find = "text", row = -1 }` (row-scoped; a negative row counts from the bottom). If the pattern never matches within `await_ms`, the run **aborts** naming the pattern and showing the last screen — never a silently wrong frame. Patterns are compiled at config load, and an `await` that could never be honoured (on a `card` scene, on an `animated` scene, or anywhere under `realtime = true`) is rejected at load rather than silently ignored.
- **`record`**: `animated = true` (per scene) — for a screen that never holds still (spinner, clock, progress bar). Instead of waiting for stability, the scene dwells for each input's own authored hold and records whatever the app drew during it.
- **`record`**: `realtime = true` (global) — play the whole recording back at measured time, as if every scene were `animated`.
- **`record`**: the sampling and pacing keys, all milliseconds — `sample_ms` (10), `change_ms` (150), `stable_ms` (40), `persist_ms` (40), `wait_cap_ms` (3000), `await_ms` (5000) — plus `max_capture_mb` (256), a backstop on accumulated grid memory that aborts with a message rather than degrading the recording.
- **`record`**: `--dump-png dir` also writes `dir/manifest.tsv`, mapping every frame back to the scene and input that produced it (`frame`, `scene`, `input`, `kind`, `hold_cs`).

### Changed

- **Breaking — `record`**: `settle_ms` and `react_ms` are gone. A config that still carries either now **fails to parse**; delete them. The fixed per-input settle window they configured no longer exists.
- **`record`** now samples the terminal grid continuously on its own thread and assembles frames from that log, rather than rasterising one screen per input at the moment the PTY goes quiet. Anything the app draws *between* inputs is captured and played back at its own measured duration.
- **`record`** no longer prints a running `scene N → M frames total` tally, because an app-driven frame makes a scene's frame count unpredictable. It prints marks per scene instead, and `manifest.tsv` replaces the arithmetic that tally supported with a lookup.

### Fixed

- **`record`**: output still draining from the *previous* input could end the current input's wait, capturing the screen from before the app answered. The grace is now measured on real grid changes rather than on PTY bytes, so a redundant repaint that touches no cell no longer disarms it.
- **`record`**: the block cursor could be drawn one cell behind the app after a typed space. A space overwrites a blank cell with a blank cell, leaving the grid byte-identical while only the caret moves; the screen comparison looked at the grid alone and kept the stale caret. Caught by the capture regression gate (`docs/regression-gate.md`).

## 0.2.0 - 2026-08-13

### New

- **`record`**: `react_ms` (default 500) — how long the app is given to *begin* answering an input before a quiet terminal counts as "finished drawing".

### Changed

- **Font fallback chain**, so a glyph the text font lacks is drawn instead of silently dropped. JetBrains Mono (text) → Symbols Nerd Font (Nerd Font icons, the Private Use Area set) → JuliaMono (Unicode symbol blocks: arrows, geometric shapes, dingbats, braille, misc technical). Each fallback is fitted to the same cell box, so the grid stays aligned. Coverage of Geometric Shapes goes 45% → 100%, Misc Symbols 3% → 100%, Dingbats 7% → 100%.
- A codepoint no bundled font has now draws a visible box (tofu) instead of nothing at all, so a font gap in a recording is something a reviewer can see.

### Fixed

- **`record`**: an input the app was slow to answer could be captured as the screen from *before* it — the change then surfaced one scene late, or half-drawn if the answer was split (a status bar naming a theme the screen was not wearing). A quiet PTY means both "finished drawing" and "not started yet", and `settle` read both as finished; it now waits up to `react_ms` for the first byte of the answer.

## 0.1.1 - 2026-07-07

### New

### Changed

### Fixed

## 0.1.0 - 2026-07-07

### New

- **`encode`**: assemble an animated WebP from captured ANSI snapshots and synthetic silent-movie title cards, each held for a configurable duration.
- **`record`**: drive a command inside an embedded terminal (a PTY plus a VT parser — no tmux) and capture one frame per key, per typed character, and per mouse cell-step; friendly `click`/`drag`/`scroll` actions expand to SGR mouse reports.
- Title cards, native truecolor, and deterministic frame output (same script → same bytes).
- Hand-painted box-drawing and block glyphs so `─│═▒█` reach cell edges and tile seamlessly; bundled JetBrains Mono font.
- `.deb` and `.rpm` packages, a man page, and prebuilt static Linux (musl) and macOS binaries.

### Changed

### Fixed
