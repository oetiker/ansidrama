# Changelog

All notable changes to AnsiDrama will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## Unreleased

### New

### Changed

### Fixed

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
