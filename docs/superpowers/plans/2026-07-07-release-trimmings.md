# Release Trimmings Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add everything `ansidrama` needs for a public MIT GitHub release — CI, cross-built binaries, `.deb`/`.rpm` packages, a man page, a changelog, a Makefile, a self-made README demo WebP, and a one-click release workflow — mirroring the sibling project `../byonk` minus its container/docs-site machinery.

**Architecture:** Rust-native packaging: `cargo-deb` + `cargo-generate-rpm` read config from `Cargo.toml` `[package.metadata.*]` and package the pre-built (cross) binary in-place. GitHub Actions provides CI (fmt/clippy/test/build) and a `workflow_dispatch` release that bumps the version, rolls `CHANGES.md`, tags, builds a 4-target matrix (Linux x86_64/aarch64 static-musl via `cross`; macOS x86_64/aarch64 native), and publishes a GitHub Release with tarballs + deb + rpm. The demo is `ansidrama` recording itself.

**Tech Stack:** Rust, GitHub Actions, `cross` (musl), `cargo-deb`, `cargo-generate-rpm`, roff (man page), GNU make.

## Global Constraints

Every task's requirements implicitly include this section.

- **Repo URL (verbatim):** `https://github.com/oetiker/ansidrama` (does not exist yet; user creates it after merge).
- **Maintainer/author string (verbatim):** `Tobias Oetiker <tobi@oetiker.ch>`.
- **License:** MIT (file `LICENSE-MIT`). Bundled JetBrains Mono is OFL (`assets/JetBrainsMono-LICENSE.txt`).
- **Targets:** Linux `x86_64-unknown-linux-musl` + `aarch64-unknown-linux-musl` (static musl, via `cross`); macOS `x86_64-apple-darwin` (runner `macos-13`) + `aarch64-apple-darwin` (runner `macos-14`), native per arch. `.deb` + `.rpm` for the two Linux targets only. **No Windows, no dmg/.app, no container, no docs site, no crates.io.**
- **Tool version floors:** `cargo-deb` **3.7.0**, `cargo-generate-rpm` **0.21.0**.
- **Packaging job placement:** deb/rpm are produced **inside the two linux-musl build legs** (binary already present), not a separate job.
- **Demo inner command:** the trailer runs a **real** `ansidrama record hello.toml` (printf-driven, non-recursive) and shows the real `OK:` line.
- **`out` resolution:** in any config, `out` is resolved **relative to the config file's directory** (`config_path.parent()`), NOT the process cwd. The launched child (`bash -lc <launch>`) inherits ansidrama's cwd (no `current_dir` is set — see `src/term.rs:132`).
- **Machine policy:** shared 128-core host — cap all cargo builds/tests to **4 cores** via `CARGO_BUILD_JOBS=4`. Cargo target dir is redirected to `/home/oetiker/scratch/cargo-target` (find it with `cargo metadata --format-version 1`, key `target_directory`); the release binary is there, not in `./target/`.
- **Language:** English for all code, comments, key names, and technical docs.
- **`record` is unix-only** and embeds its own PTY+VT terminal — no tmux anywhere (no tmux install in CI).

---

## File Structure

Created:
- `man/ansidrama.1` — hand-written roff man page (Task 1).
- `CHANGES.md` — Keep-a-Changelog changelog seeded for the first release (Task 2).
- `Cross.toml` — musl cross images + `RUSTFLAGS` passthrough (Task 3).
- `Makefile` — fmt/lint/test/check/release/package/man/demo/help (Task 5).
- `demo/hello.toml` — inner printf-driven `record` script (Task 6).
- `demo/readme.toml` — outer trailer `record` script (Task 6).
- `docs/demo/ansidrama.webp` — committed generated demo (Task 6).
- `.github/workflows/ci.yml` — CI (Task 7).
- `.github/workflows/release.yml` — release automation (Task 8).

Modified:
- `Cargo.toml` — `repository`/`homepage`, `strip`, `[package.metadata.deb]`, `[package.metadata.generate-rpm]` (Task 2).
- `.gitignore` — ignore the demo byproduct `demo/hello.webp` (Task 6).
- `README.md` — hero WebP, CI badge, expanded Install (Task 9).

---

### Task 1: Man page (`man/ansidrama.1`)

Standalone deliverable; packaging (Task 2) and the tarball (Task 8) reference it. No code dependencies.

**Files:**
- Create: `man/ansidrama.1`

**Interfaces:**
- Produces: an installable roff man page at `man/ansidrama.1`, section 1, referenced by deb/rpm assets and the release tarball.

- [ ] **Step 1: Write the man page**

Create `man/ansidrama.1` with exactly this content. Content is drawn from `src/main.rs` (CLI/options), `src/config.rs` (TOML keys), `src/keys.rs` (key names), and `README.md`.

```roff
.TH ANSIDRAMA 1 "2026-07-07" "ansidrama" "User Commands"
.SH NAME
ansidrama \- assemble or record a terminal session into a crisp animated WebP
.SH SYNOPSIS
.B ansidrama encode
.I config.toml
.RB [ \-o
.IR out.webp ]
.RB [ \-\-dump\-png
.IR dir ]
.br
.B ansidrama record
.I config.toml
.RB [ \-o
.IR out.webp ]
.RB [ \-\-dump\-png
.IR dir ]
.SH DESCRIPTION
.B ansidrama
turns a terminal session into a small, lossless, looping animated WebP.
It renders each frame itself \(em parsing the terminal cell grid and
rasterizing it with a bundled monospace font, hand\-painting box\-drawing and
block glyphs so characters reach the exact cell edges and tile seamlessly.
Every run is deterministic: the same script produces the same bytes. There are
no runtime dependencies \(em a single static binary, no browser, no ffmpeg, no
tmux.
.SH COMMANDS
.TP
.B encode
Assemble a WebP from a list of captured ANSI snapshots and/or synthetic title
cards, each held for a duration.
.TP
.B record
Drive a command inside an embedded terminal (a PTY plus a VT parser) per a
scene script, capturing one frame per key, per typed character, and per mouse
cell\-step.
.B record
is available on unix platforms only.
.SH OPTIONS
.TP
.BR \-o ", " \-\-out " " \fIpath\fR
Output WebP path. Overrides the
.B out
key in the config.
.TP
.BR \-\-dump\-png " " \fIdir\fR
Also write each rendered frame as a PNG into
.I dir
(for debugging).
.SH CONFIGURATION
Configuration is TOML. The
.B out
key is resolved relative to the config file's directory.
.SS "encode config"
Top level:
.BR cols ", " rows ", " font_px ", " card_font_px ", " card_subtitle_px ", "
.BR max_fps ", " out .
Each
.B [[frame]]
has either
.B file
(path to a captured
.I .ansi
snapshot, relative to the config) or
.B card
(a title card), plus
.B hold_cs
(hold in centiseconds).
.SS "record config"
Top level:
.BR launch " (the shell command line), "
.BR cols ", " rows ", " font_px ", " card_font_px ", " card_subtitle_px ", "
.BR max_fps ", " out ", " env ", " startup_ms ", " settle_ms ", " type_cs ", "
.BR move_cs ", " quit_keys ", " cursor .
Each
.B [[scene]]
performs exactly one action \(em
.BR keys ", " text ", " click ", " drag ", " scroll ", or " card " \(em"
plus timing
.RB ( hold_cs ", " type_cs ", " move_cs ).
A scene expands into many frames: one captured per key, per typed character,
and per mouse cell\-step.
.SS "title cards"
A
.B card
is a silent\-movie intertitle:
.BR text " (or " lines "), " fg ", " bg ", " bold ", " border .
Colours are
.IR #rrggbb ", " #rgb ", or a basic name (" black " " white " " red " " green"
.IR blue " " yellow " " grey ")."
.SH "KEY NAMES"
Named keys accepted in a scene's
.B keys
list (tmux\-style):
.B Enter Return Tab BTab Escape Esc Space BSpace Backspace Up Down Right Left
.B Home End PageUp PPage PageDown NPage Insert IC Delete DC F1\-F12 .
Modifiers:
.BR C- " (control), " M- " (meta/alt), " S- " (shift) \(em e.g. " C-c ", " M-x ", " S-Tab .
A single character is sent literally; a value beginning with ESC passes through
as a raw escape sequence.
.SH EXAMPLES
Assemble frames listed in a config into a WebP:
.PP
.RS
.EX
ansidrama encode demo.toml \-o out.webp
.EE
.RE
.PP
Record an app, dumping each frame as a PNG for inspection:
.PP
.RS
.EX
ansidrama record record.toml \-\-dump\-png frames/
.EE
.RE
.SH FILES
The output WebP (from
.BR \-o " or the " out " config key). With " \-\-dump\-png ", one PNG per frame."
.SH AUTHOR
Tobias Oetiker <tobi@oetiker.ch>
.SH LICENSE
MIT. The bundled JetBrains Mono font is under the SIL Open Font License.
.SH "SEE ALSO"
Project home: https://github.com/oetiker/ansidrama
```

- [ ] **Step 2: Preview and lint the man page**

Run: `man ./man/ansidrama.1 | head -40`
Expected: renders with NAME/SYNOPSIS/DESCRIPTION headings, no "can't open" error.

Run: `LC_ALL=C groff -man -Tascii -ww ./man/ansidrama.1 >/dev/null`
Expected: no warnings printed (exit 0). (`mandoc` is absent on this host; `groff` from `man-db` is the linter. If `groff` is also absent, `man --warnings=all ./man/ansidrama.1 >/dev/null 2>&1` is the fallback — expect no output.)

- [ ] **Step 3: Commit**

```bash
git add man/ansidrama.1
git commit -m "docs(man): add hand-written ansidrama.1 man page"
```

---

### Task 2: Cargo.toml packaging metadata + CHANGES.md

Adds release metadata and the changelog the release workflow depends on. No behavior change to the binary.

**Files:**
- Modify: `Cargo.toml`
- Create: `CHANGES.md`

**Interfaces:**
- Consumes: `man/ansidrama.1` (Task 1) — referenced by deb/rpm assets.
- Produces: `[package.metadata.deb]` and `[package.metadata.generate-rpm]` tables consumed by Tasks 4/8; `CHANGES.md` in the exact shape the release perl (Task 8) rewrites; `strip = true` in `[profile.release]` so the cross-built binary is stripped at build time (avoids cross-arch `strip` in packaging).

- [ ] **Step 1: Add repository/homepage + strip to `[package]` / `[profile.release]`**

In `Cargo.toml`, add `repository` and `homepage` to `[package]` (after `categories`):

```toml
repository = "https://github.com/oetiker/ansidrama"
homepage = "https://github.com/oetiker/ansidrama"
```

And add `strip = true` to `[profile.release]` (keep the existing `opt-level = 2`):

```toml
[profile.release]
# Small, self-contained demo binary; box-glyph painting is not hot.
opt-level = 2
# Strip at build time so cross-arch packaging never needs a matching `strip`.
strip = true
```

- [ ] **Step 2: Append the cargo-deb metadata table**

Append to the end of `Cargo.toml`:

```toml
[package.metadata.deb]
maintainer = "Tobias Oetiker <tobi@oetiker.ch>"
copyright = "2026, Tobias Oetiker <tobi@oetiker.ch>"
license-file = ["LICENSE-MIT", "0"]
extended-description = """\
Turn a terminal session into a crisp, tiny, animated WebP. ansidrama assembles \
captured ANSI snapshots and silent-movie title cards (encode), or drives a \
command in an embedded terminal and captures each frame (record). Deterministic, \
lossless, and dependency-free: a single static binary."""
section = "utils"
priority = "optional"
assets = [
    ["target/release/ansidrama", "usr/bin/", "755"],
    ["man/ansidrama.1", "usr/share/man/man1/", "644"],
    ["README.md", "usr/share/doc/ansidrama/README.md", "644"],
    ["LICENSE-MIT", "usr/share/doc/ansidrama/LICENSE-MIT", "644"],
]
```

Note: cargo-deb rewrites the `target/release/` asset prefix to `target/<triple>/release/` when `--target` is passed, so this path is correct for both host and cross packaging.

- [ ] **Step 3: Append the cargo-generate-rpm metadata table**

Append to the end of `Cargo.toml`:

```toml
[package.metadata.generate-rpm]
summary = "Assemble or record a terminal session into a crisp animated WebP"
license = "MIT"
assets = [
    { source = "target/release/ansidrama", dest = "/usr/bin/ansidrama", mode = "755" },
    { source = "man/ansidrama.1", dest = "/usr/share/man/man1/ansidrama.1", mode = "644", doc = true },
    { source = "README.md", dest = "/usr/share/doc/ansidrama/README.md", mode = "644", doc = true },
    { source = "LICENSE-MIT", dest = "/usr/share/doc/ansidrama/LICENSE-MIT", mode = "644", doc = true },
]
```

- [ ] **Step 4: Verify Cargo.toml still parses and builds**

Run: `CARGO_BUILD_JOBS=4 cargo metadata --format-version 1 >/dev/null && echo METADATA_OK`
Expected: `METADATA_OK` (no TOML parse error).

Run: `CARGO_BUILD_JOBS=4 cargo build --release 2>&1 | tail -3`
Expected: `Finished` line; binary rebuilt (now stripped).

- [ ] **Step 5: Create CHANGES.md**

Create `CHANGES.md` with exactly this content. The `## Unreleased` block with `### New` / `### Changed` / `### Fixed` in this order is required by the release workflow's perl rewrite (Task 8).

```markdown
# Changelog

All notable changes to ansidrama will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## Unreleased

### New

- **`encode`**: assemble an animated WebP from captured ANSI snapshots and synthetic silent-movie title cards, each held for a configurable duration.
- **`record`**: drive a command inside an embedded terminal (a PTY plus a VT parser — no tmux) and capture one frame per key, per typed character, and per mouse cell-step; friendly `click`/`drag`/`scroll` actions expand to SGR mouse reports.
- Title cards, native truecolor, and deterministic frame output (same script → same bytes).
- Hand-painted box-drawing and block glyphs so `─│═▒█` reach cell edges and tile seamlessly; bundled JetBrains Mono font.
- `.deb` and `.rpm` packages, a man page, and prebuilt static Linux (musl) and macOS binaries.

### Changed

### Fixed
```

- [ ] **Step 6: Commit**

```bash
git add Cargo.toml CHANGES.md
git commit -m "build: add deb/rpm metadata, repo URLs, release strip, and CHANGES.md"
```

---

### Task 3: Cross.toml + verify the musl build risk

The `webp` crate compiles libwebp via `cc`. Per the spec, confirm a static-musl cross build succeeds **before** the release matrix depends on it. `cross` and `podman` are present locally.

**Files:**
- Create: `Cross.toml`

**Interfaces:**
- Produces: `Cross.toml` used by the release workflow's two musl legs (Task 8).

- [ ] **Step 1: Create Cross.toml**

Create `Cross.toml` (copied from byonk):

```toml
# Cross-compilation configuration for fully static musl binaries.
# See https://github.com/cross-rs/cross

[target.x86_64-unknown-linux-musl]
image = "ghcr.io/cross-rs/x86_64-unknown-linux-musl:main"

[target.aarch64-unknown-linux-musl]
image = "ghcr.io/cross-rs/aarch64-unknown-linux-musl:main"

[build.env]
passthrough = ["RUSTFLAGS"]
```

- [ ] **Step 2: Run the x86_64-musl cross build (the risk check)**

`cross` uses `podman` here via `CROSS_CONTAINER_ENGINE`. This pulls a container image (network + minutes on first run).

Run:
```bash
CROSS_CONTAINER_ENGINE=podman CARGO_BUILD_JOBS=4 \
  RUSTFLAGS="-C target-feature=+crt-static" \
  cross build --release --target x86_64-unknown-linux-musl 2>&1 | tail -15
```
Expected: `Finished \`release\` profile`. libwebp compiled inside the musl container.

- [ ] **Step 3: Confirm the binary is a static musl ELF**

Run:
```bash
BIN="$(cargo metadata --format-version 1 | python3 -c 'import json,sys;print(json.load(sys.stdin)["target_directory"])')/x86_64-unknown-linux-musl/release/ansidrama"
file "$BIN"; ldd "$BIN" 2>&1 || true
```
Expected: `file` reports `ELF 64-bit ... statically linked`; `ldd` prints `not a dynamic executable` (or similar). This proves libwebp cross-compiled statically.

If the build **fails** (libwebp/cc cannot cross to musl): STOP and report. The documented fallback is to switch both Linux targets to `-gnu` (glibc) in `Cross.toml`, this task's commands, and Task 8's matrix; deb/rpm remain valid on glibc. Do not proceed silently.

- [ ] **Step 4: Commit**

```bash
git add Cross.toml
git commit -m "build(cross): add musl Cross.toml; verified static-musl libwebp build"
```

---

### Task 4: Local host packaging (validate the deb/rpm metadata)

Produce `.deb` and `.rpm` on the host from the Task 2 metadata and inspect their contents, confirming the binary + man page + docs land at the right paths. This validates Task 2 before the release workflow relies on it.

**Files:** none created; this task builds and inspects packages.

**Interfaces:**
- Consumes: `[package.metadata.deb]` / `[package.metadata.generate-rpm]` (Task 2), `man/ansidrama.1` (Task 1).

- [ ] **Step 1: Ensure the packaging tools are installed at/above the version floors**

Run:
```bash
cargo deb --version 2>/dev/null || cargo install cargo-deb --version '^3.7'
cargo generate-rpm --version 2>/dev/null || cargo install cargo-generate-rpm --version '^0.21'
```
Expected: both print a version (`cargo-deb` ≥ 3.7.0, `cargo-generate-rpm` ≥ 0.21.0). `cargo-generate-rpm` is not yet installed on this host, so it will build.

- [ ] **Step 2: Build the release binary (host target)**

Run: `CARGO_BUILD_JOBS=4 cargo build --release 2>&1 | tail -2`
Expected: `Finished`. (The man page and binary now both exist for packaging.)

- [ ] **Step 3: Build the .deb and inspect its contents**

Run:
```bash
cargo deb --no-build 2>&1 | tail -2
DEB="$(cargo metadata --format-version 1 | python3 -c 'import json,sys;print(json.load(sys.stdin)["target_directory"])')/debian"
ls "$DEB"/*.deb
dpkg-deb -c "$DEB"/ansidrama_*.deb
```
Expected: a `.deb` is written; `dpkg-deb -c` lists `./usr/bin/ansidrama`, `./usr/share/man/man1/ansidrama.1.gz`, `./usr/share/doc/ansidrama/README.md`, and `./usr/share/doc/ansidrama/LICENSE-MIT`.

- [ ] **Step 4: Build the .rpm**

Run:
```bash
cargo generate-rpm 2>&1 | tail -3
RPM="$(cargo metadata --format-version 1 | python3 -c 'import json,sys;print(json.load(sys.stdin)["target_directory"])')/generate-rpm"
ls "$RPM"/*.rpm
```
Expected: an `.rpm` is written. (The `rpm` CLI is absent on this host; producing the file from the shared asset list validated by the deb inspection in Step 3 is sufficient. If `rpm2cpio` happens to be present, `rpm2cpio "$RPM"/*.rpm | cpio -tv` lists the same paths — best-effort.)

- [ ] **Step 5: No commit**

Packages are build artifacts under the (gitignored) target dir. Nothing to commit. If the deb listing shows a wrong path, fix the Task 2 metadata, `git commit --amend` into Task 2's commit, and re-run.

---

### Task 5: Makefile

Trimmed byonk-style Makefile with the developer targets and the demo/package entry points. Caps parallelism to 4 cores.

**Files:**
- Create: `Makefile`

**Interfaces:**
- Produces: `make demo` (used by Task 6), `make package`, `make man`, `make check`.

- [ ] **Step 1: Write the Makefile**

Create `Makefile`:

```makefile
# ansidrama Makefile — build, check, package, and regenerate the demo.
# Shared host policy: cap cargo to 4 cores.

export CARGO_BUILD_JOBS := 4

# Locate the (possibly redirected) cargo target dir and the release binary.
TARGET_DIR := $(shell cargo metadata --format-version 1 | python3 -c 'import json,sys;print(json.load(sys.stdin)["target_directory"])')
RELEASE_BIN := $(TARGET_DIR)/release/ansidrama

.PHONY: all release fmt lint test check package man demo help

all: check

# --- development ------------------------------------------------------------

fmt:
	cargo fmt

lint:
	cargo clippy --all-targets -- -D warnings

test:
	cargo test

# Format, lint, and test — run before every commit.
check: fmt lint test
	@echo "All checks passed!"

# Release binary (format + lint first).
release: fmt lint
	cargo build --release

# --- packaging (host target) ------------------------------------------------

# Build .deb + .rpm for the host target from the Cargo.toml metadata.
package: release
	cargo deb --no-build
	cargo generate-rpm
	@echo "Packages written under $(TARGET_DIR)/{debian,generate-rpm}/"

# --- documentation ----------------------------------------------------------

# Preview the man page.
man:
	man ./man/ansidrama.1

# --- demo -------------------------------------------------------------------

# Regenerate the committed README demo (docs/demo/ansidrama.webp).
# Runs from demo/ so the inner `cat`/`record`/`ls` see hello.toml, and puts the
# freshly built binary on PATH for the inner `ansidrama record` call.
demo: release
	cd demo && PATH="$(dir $(RELEASE_BIN)):$$PATH" ansidrama record readme.toml
	@echo "Wrote docs/demo/ansidrama.webp"

# --- help -------------------------------------------------------------------

help:
	@echo "ansidrama Makefile"
	@echo ""
	@echo "  make check     Format, lint (clippy -D warnings), and test"
	@echo "  make release   Build the release binary (fmt + lint first)"
	@echo "  make package   Build .deb + .rpm for the host target"
	@echo "  make man       Preview the man page"
	@echo "  make demo      Regenerate docs/demo/ansidrama.webp"
	@echo "  make help      Show this help"
```

- [ ] **Step 2: Verify the core targets work**

Run: `make help`
Expected: the help text above.

Run: `make check 2>&1 | tail -5`
Expected: ends with `All checks passed!` (fmt clean, clippy clean, tests green).

Run: `make man >/dev/null && echo MAN_OK` (non-interactive check that the target resolves; `man` may page — pipe to a pager-less check)
Expected: `MAN_OK` after quitting the pager, or run `MANPAGER=cat make man | head -5`.

- [ ] **Step 3: Commit**

```bash
git add Makefile
git commit -m "build: add Makefile (check/release/package/man/demo)"
```

---

### Task 6: The "Makes itself" demo

`ansidrama` records itself: an outer `record` drives an interactive `bash` through a trailer whose middle beat is a **real** `ansidrama record hello.toml`. Output is committed so GitHub renders it without CI.

**Files:**
- Create: `demo/hello.toml`
- Create: `demo/readme.toml`
- Create: `docs/demo/ansidrama.webp` (generated, committed)
- Modify: `.gitignore`

**Interfaces:**
- Consumes: `make demo` (Task 5), the release binary.
- Produces: `docs/demo/ansidrama.webp` embedded by the README (Task 9).

- [ ] **Step 1: Write the inner script `demo/hello.toml`**

Drives `printf` (non-recursive — it never runs ansidrama). Produces `hello.webp` in `demo/`.

```toml
# Inner demo: a tiny, real `record` run driven by printf (no recursion).
# `out` is relative to this file's dir (demo/), so hello.webp lands in demo/.
launch  = "bash --norc --noprofile -i"
cols    = 64
rows    = 10
font_px = 18
out     = "hello.webp"
env     = { PS1 = "$ ", COLORTERM = "truecolor" }
quit_keys = ["C-d"]

[[scene]]
card    = { text = "hello, world", fg = "#fef9c3" }
hold_cs = 150

[[scene]]
text    = "printf '\\033[38;2;120;220;255mhi from ansidrama\\033[0m\\n'"
hold_cs = 40
[[scene]]
keys    = ["Enter"]
hold_cs = 220
```

- [ ] **Step 2: Write the outer trailer `demo/readme.toml`**

`out` climbs out of `demo/` to land the file at `docs/demo/ansidrama.webp`.

```toml
# Outer trailer: drive bash through the self-referential demo.
# `out` is relative to this file's dir (demo/): ../docs/demo → docs/demo/.
launch  = "bash --norc --noprofile -i"
cols    = 84
rows    = 22
font_px = 18
card_font_px = 40
out     = "../docs/demo/ansidrama.webp"
env     = { PS1 = "$ ", COLORTERM = "truecolor" }
quit_keys = ["C-d"]

[[scene]]
card    = { lines = ["ansidrama", "no browser · no ffmpeg · no tmux"], fg = "#fef9c3" }
hold_cs = 300

# Reveal the (real, colourised) inner script.
[[scene]]
text    = "cat hello.toml"
hold_cs = 40
[[scene]]
keys    = ["Enter"]
hold_cs = 250

# Run it for real — the OK: line is genuine output.
[[scene]]
text    = "ansidrama record hello.toml"
hold_cs = 40
[[scene]]
keys    = ["Enter"]
hold_cs = 450

# Show the file it made.
[[scene]]
text    = "ls -l hello.webp"
hold_cs = 40
[[scene]]
keys    = ["Enter"]
hold_cs = 250

[[scene]]
card    = { text = "a star is born 🌟", fg = "#fef9c3" }
hold_cs = 350
```

- [ ] **Step 3: Ignore the demo byproduct**

Append to `.gitignore`:

```gitignore
/demo/hello.webp
```

- [ ] **Step 4: Generate the demo and eyeball it**

Run: `make demo 2>&1 | tail -5`
Expected: ends with `Wrote docs/demo/ansidrama.webp`; `docs/demo/ansidrama.webp` exists (`ls -l docs/demo/ansidrama.webp`).

Dump frames to inspect the trailer renders correctly:
```bash
BIN="$(cargo metadata --format-version 1 | python3 -c 'import json,sys;print(json.load(sys.stdin)["target_directory"])')/release/ansidrama"
cd demo && PATH="$(dirname "$BIN"):$PATH" ansidrama record readme.toml --dump-png /tmp/ansidrama-demo-frames && ls /tmp/ansidrama-demo-frames | head
```
Read a few PNGs (Read tool) to confirm: title card → `cat hello.toml` output → the real `OK: wrote hello.webp (...)` line → `ls -l hello.webp` → "a star is born" card. If prompt/timing looks off (e.g. no `$ ` prompt, clipped output), tune `PS1`, `startup_ms`/`settle_ms`, `hold_cs`, or `cols`/`rows` and re-run `make demo`. This visual loop is part of the task.

- [ ] **Step 5: Commit**

```bash
git add demo/hello.toml demo/readme.toml docs/demo/ansidrama.webp .gitignore
git commit -m "demo: self-recording README trailer (make demo) + committed webp"
```

---

### Task 7: CI workflow (`.github/workflows/ci.yml`)

Mirrors byonk: fmt/clippy/test/build on push/PR to `main`. No tmux install (record is tmux-free; the smoke test drives `bash`, present on runners).

**Files:**
- Create: `.github/workflows/ci.yml`

- [ ] **Step 1: Write ci.yml**

```yaml
name: CI

on:
  push:
    branches: [main]
  pull_request:
    branches: [main]

env:
  CARGO_TERM_COLOR: always

jobs:
  check:
    name: Check & Lint
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4

      - name: Install Rust toolchain
        uses: dtolnay/rust-toolchain@stable
        with:
          components: rustfmt, clippy

      - name: Cache cargo registry
        uses: actions/cache@v4
        with:
          path: |
            ~/.cargo/registry
            ~/.cargo/git
            target
          key: ${{ runner.os }}-cargo-${{ hashFiles('**/Cargo.lock') }}
          restore-keys: |
            ${{ runner.os }}-cargo-

      - name: Check formatting
        run: cargo fmt --check

      - name: Run Clippy
        run: cargo clippy --all-targets -- -D warnings

  test:
    name: Test
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4

      - name: Install Rust toolchain
        uses: dtolnay/rust-toolchain@stable

      - name: Cache cargo registry
        uses: actions/cache@v4
        with:
          path: |
            ~/.cargo/registry
            ~/.cargo/git
            target
          key: ${{ runner.os }}-cargo-${{ hashFiles('**/Cargo.lock') }}
          restore-keys: |
            ${{ runner.os }}-cargo-

      - name: Run tests
        run: cargo test

  build:
    name: Build
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4

      - name: Install Rust toolchain
        uses: dtolnay/rust-toolchain@stable

      - name: Cache cargo registry
        uses: actions/cache@v4
        with:
          path: |
            ~/.cargo/registry
            ~/.cargo/git
            target
          key: ${{ runner.os }}-cargo-${{ hashFiles('**/Cargo.lock') }}
          restore-keys: |
            ${{ runner.os }}-cargo-

      - name: Build
        run: cargo build
```

- [ ] **Step 2: Validate YAML syntax**

Run: `python3 -c "import yaml,sys; yaml.safe_load(open('.github/workflows/ci.yml')); print('YAML_OK')"`
Expected: `YAML_OK`. (`actionlint` is absent on this host; `yaml.safe_load` is the local syntax check. Real CI runs after the repo exists.)

- [ ] **Step 3: Commit**

```bash
git add .github/workflows/ci.yml
git commit -m "ci: add fmt/clippy/test/build workflow (no tmux)"
```

---

### Task 8: Release workflow (`.github/workflows/release.yml`)

Adapted from byonk: `version` → `build-binaries` (4-target matrix, deb/rpm folded into the musl legs) → `create-release`. byonk's `build-container`, `build-docs`, `deploy-docs`, and the Windows matrix leg are dropped.

**Files:**
- Create: `.github/workflows/release.yml`

**Interfaces:**
- Consumes: `CHANGES.md` shape (Task 2), `Cross.toml` (Task 3), `man/ansidrama.1` (Task 1), packaging metadata (Task 2).

- [ ] **Step 1: Write release.yml**

```yaml
name: Release

on:
  workflow_dispatch:
    inputs:
      release_type:
        description: 'Release type'
        required: true
        type: choice
        options:
          - bugfix
          - feature
          - major

env:
  CARGO_TERM_COLOR: always

jobs:
  version:
    name: Bump Version
    runs-on: ubuntu-latest
    permissions:
      contents: write
    outputs:
      version: ${{ steps.version.outputs.version }}
      tag: ${{ steps.version.outputs.tag }}
    steps:
      - uses: actions/checkout@v4
        with:
          fetch-depth: 0

      - name: Verify main branch
        run: |
          if [ "${{ github.ref }}" != "refs/heads/main" ]; then
            echo "::error::Releases must be created from the main branch"
            exit 1
          fi

      - name: Calculate new version
        id: version
        run: |
          LATEST=$(git tag -l 'v[0-9]*.[0-9]*.[0-9]*' | sort -V | tail -1 || echo "v0.0.0")
          if [ -z "$LATEST" ]; then
            LATEST="v0.0.0"
          fi
          MAJOR=$(echo "$LATEST" | sed 's/v\([0-9]*\)\.\([0-9]*\)\.\([0-9]*\)/\1/')
          MINOR=$(echo "$LATEST" | sed 's/v\([0-9]*\)\.\([0-9]*\)\.\([0-9]*\)/\2/')
          PATCH=$(echo "$LATEST" | sed 's/v\([0-9]*\)\.\([0-9]*\)\.\([0-9]*\)/\3/')
          case "${{ inputs.release_type }}" in
            major)   NEW_VERSION="$((MAJOR+1)).0.0" ;;
            feature) NEW_VERSION="${MAJOR}.$((MINOR+1)).0" ;;
            bugfix)  NEW_VERSION="${MAJOR}.${MINOR}.$((PATCH+1))" ;;
          esac
          echo "version=${NEW_VERSION}" >> $GITHUB_OUTPUT
          echo "tag=v${NEW_VERSION}" >> $GITHUB_OUTPUT
          echo "New version: ${NEW_VERSION}"

      - name: Update Cargo.toml version
        run: |
          sed -i 's/^version = ".*"/version = "${{ steps.version.outputs.version }}"/' Cargo.toml

      - name: Update CHANGES.md
        run: |
          DATE=$(date +%Y-%m-%d)
          VERSION="${{ steps.version.outputs.version }}"
          perl -i -0777 -pe '
            s/## Unreleased\n+(### New\n(.*?))?(\n### Changed\n(.*?))?(\n### Fixed\n(.*?))?\n+(?=##|\z)/
              "## Unreleased\n\n### New\n\n### Changed\n\n### Fixed\n\n" .
              "## '"$VERSION"' - '"$DATE"'\n" .
              ($2 ? "\n### New\n$2" : "") .
              ($4 ? "\n### Changed\n$4" : "") .
              ($6 ? "\n### Fixed\n$6" : "") .
              "\n"
            /se' CHANGES.md

      - name: Commit and tag
        run: |
          git config user.name "github-actions[bot]"
          git config user.email "github-actions[bot]@users.noreply.github.com"
          git add Cargo.toml CHANGES.md
          git commit -m "Release ${{ steps.version.outputs.tag }}"
          git tag -a "${{ steps.version.outputs.tag }}" -m "Release ${{ steps.version.outputs.tag }}"
          git push origin main --tags

  build-binaries:
    name: Build ${{ matrix.target }}
    needs: version
    runs-on: ${{ matrix.os }}
    strategy:
      matrix:
        include:
          - target: x86_64-unknown-linux-musl
            os: ubuntu-latest
            cross: true
            package: true
          - target: aarch64-unknown-linux-musl
            os: ubuntu-latest
            cross: true
            package: true
          - target: x86_64-apple-darwin
            os: macos-13
          - target: aarch64-apple-darwin
            os: macos-14
    steps:
      - uses: actions/checkout@v4
        with:
          ref: ${{ needs.version.outputs.tag }}

      - name: Install Rust toolchain
        uses: dtolnay/rust-toolchain@stable
        with:
          targets: ${{ matrix.target }}

      - name: Install cross
        if: matrix.cross
        run: cargo install cross --git https://github.com/cross-rs/cross

      - name: Build binary
        run: |
          if [ "${{ matrix.cross }}" = "true" ]; then
            RUSTFLAGS="-C target-feature=+crt-static" cross build --release --target ${{ matrix.target }}
          else
            cargo build --release --target ${{ matrix.target }}
          fi
        shell: bash

      - name: Create tarball
        run: |
          mkdir -p dist staging/ansidrama
          BINARY="target/${{ matrix.target }}/release/ansidrama"
          ARCHIVE="ansidrama-${{ needs.version.outputs.version }}-${{ matrix.target }}.tar.gz"
          cp "$BINARY" staging/ansidrama/
          mkdir -p staging/ansidrama/man
          cp man/ansidrama.1 staging/ansidrama/man/
          cp README.md LICENSE-MIT staging/ansidrama/
          tar -czvf "dist/${ARCHIVE}" -C staging ansidrama
        shell: bash

      - name: Build deb and rpm
        if: matrix.package
        run: |
          cargo install cargo-deb --version '^3.7'
          cargo install cargo-generate-rpm --version '^0.21'
          cargo deb --no-build --no-strip --target ${{ matrix.target }}
          cargo generate-rpm --target ${{ matrix.target }}
          cp target/${{ matrix.target }}/debian/*.deb dist/
          cp target/${{ matrix.target }}/generate-rpm/*.rpm dist/
        shell: bash

      - name: Upload artifact
        uses: actions/upload-artifact@v4
        with:
          name: ansidrama-${{ matrix.target }}
          path: dist/*

  create-release:
    name: Create GitHub Release
    needs: [version, build-binaries]
    runs-on: ubuntu-latest
    permissions:
      contents: write
    steps:
      - uses: actions/checkout@v4
        with:
          ref: ${{ needs.version.outputs.tag }}

      - name: Download all artifacts
        uses: actions/download-artifact@v4
        with:
          path: artifacts
          pattern: ansidrama-*
          merge-multiple: true

      - name: Extract release notes
        run: |
          VERSION="${{ needs.version.outputs.version }}"
          sed -n "/^## ${VERSION}/,/^## [0-9]/p" CHANGES.md | sed '$d' > release-notes.md
          echo "Release notes:"; cat release-notes.md

      - name: List artifacts
        run: ls -la artifacts/

      - name: Create Release
        uses: softprops/action-gh-release@v2
        with:
          tag_name: ${{ needs.version.outputs.tag }}
          name: ansidrama ${{ needs.version.outputs.tag }}
          body_path: release-notes.md
          files: artifacts/*
          fail_on_unmatched_files: true
          draft: false
          prerelease: false
```

Notes: `--no-strip` is passed to `cargo deb` because the binary is already stripped at build time (Task 2 `strip = true`), and a glibc host cannot strip an aarch64 musl binary. `cargo generate-rpm` does not strip by default, so no flag is needed. The tarball ships `ansidrama` + `man/ansidrama.1` + `README.md` + `LICENSE-MIT` (the font is embedded in the binary — nothing else to bundle).

- [ ] **Step 2: Validate YAML + sanity-check the notes slice against the real CHANGES.md**

Run: `python3 -c "import yaml; yaml.safe_load(open('.github/workflows/release.yml')); print('YAML_OK')"`
Expected: `YAML_OK`.

Dry-run the release-notes `sed` against the committed `CHANGES.md` using a hypothetical `0.1.0` header to confirm the slicer shape works once a version section exists:
```bash
sed 's/^## Unreleased/## 0.1.0 - 2026-07-07/' CHANGES.md | sed -n '/^## 0.1.0/,/^## [0-9]/p' | sed '$d'
```
Expected: prints the `## 0.1.0 - ...` heading plus the New/Changed/Fixed body (proves the notes extraction will find the section the `version` job creates).

- [ ] **Step 3: Commit**

```bash
git add .github/workflows/release.yml
git commit -m "ci: add release workflow (version bump, cross matrix, deb/rpm, gh release)"
```

---

### Task 9: README updates

Add the self-made demo as the hero, a CI badge, and an expanded Install section. Keep all existing content.

**Files:**
- Modify: `README.md`

**Interfaces:**
- Consumes: `docs/demo/ansidrama.webp` (Task 6).

- [ ] **Step 1: Add the hero WebP + CI badge under the title**

In `README.md`, immediately after the `# ansidrama` line (line 1), insert:

```markdown

[![CI](https://github.com/oetiker/ansidrama/actions/workflows/ci.yml/badge.svg)](https://github.com/oetiker/ansidrama/actions/workflows/ci.yml)

![ansidrama records itself](docs/demo/ansidrama.webp)
```

- [ ] **Step 2: Expand the Install section**

Replace the current Install section (the `## Install` heading and its fenced block plus the two paragraphs after it — `README.md:136-143`) with:

```markdown
## Install

**Prebuilt binaries** (Linux static-musl x86_64/aarch64, macOS x86_64/aarch64) —
download the tarball for your platform from the
[Releases page](https://github.com/oetiker/ansidrama/releases), unpack, and put
`ansidrama` on your `PATH`:

```sh
tar xzf ansidrama-*-x86_64-unknown-linux-musl.tar.gz
sudo install ansidrama/ansidrama /usr/local/bin/
```

**Debian/Ubuntu** — grab the `.deb` from the release and:

```sh
sudo dpkg -i ansidrama_*_amd64.deb
man ansidrama
```

**Fedora/RHEL/openSUSE** — grab the `.rpm` and:

```sh
sudo rpm -i ansidrama-*.x86_64.rpm
```

**From source:**

```sh
cargo install --path .        # or: cargo build --release
```

`record` embeds its own terminal (a PTY + VT parser), so it needs **nothing but
the binary** — same as `encode`. The font (JetBrains Mono, OFL) is bundled.
```

- [ ] **Step 3: Verify the README references resolve**

Run: `test -f docs/demo/ansidrama.webp && echo HERO_OK`
Expected: `HERO_OK` (the embedded image exists).

Run: `grep -c "actions/workflows/ci.yml" README.md`
Expected: `2` (badge image + link).

- [ ] **Step 4: Commit**

```bash
git add README.md
git commit -m "docs(readme): hero demo webp, CI badge, expanded install"
```

---

## Self-Review

**1. Spec coverage** (each spec deliverable → task):
- CI workflow → Task 7. ✓
- Release workflow (version/build/deb-rpm/create-release; container+docs dropped) → Task 8. ✓
- Cross.toml → Task 3. ✓
- Packaging metadata in Cargo.toml → Task 2 (validated Task 4). ✓
- Man page → Task 1. ✓
- CHANGES.md → Task 2. ✓
- Makefile → Task 5. ✓
- "Makes itself" demo (hello.toml, readme.toml, committed webp) → Task 6. ✓
- README (hero, install, badge) → Task 9. ✓
- Cargo.toml repository/homepage → Task 2. ✓
- Build risk verified first → Task 3 (before the matrix in Task 8). ✓
- Decisions locked: deb/rpm folded into musl legs (Task 8 matrix `package: true`); demo runs real `record hello.toml` (Task 6); macOS pinned macos-13/macos-14 (Task 8 matrix). ✓

**2. Placeholder scan:** No `TBD`/`TODO`/"add error handling"/"similar to Task N". All file contents are given in full.

**3. Type/name consistency:** binary name `ansidrama` throughout; CLI flags `-o`/`--out`/`--dump-png` match `src/main.rs`; config keys in the man page match `src/config.rs`; key names match `src/keys.rs`; `out` relative-to-config-dir behavior (`src/lib.rs:49`/`src/record.rs:258`) drives the demo `out` paths; asset paths match between the deb table, rpm table, and tarball staging; `CHANGES.md` `## Unreleased` + New/Changed/Fixed shape matches the Task 8 perl.

## Execution Handoff

Plan complete and saved to `docs/superpowers/plans/2026-07-07-release-trimmings.md`.
</content>
