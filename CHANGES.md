# Changelog

All notable changes to AnsiDrama will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## Unreleased

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
