# Design: release trimmings (CI, packages, man page, demo, release automation)

Date: 2026-07-07
Status: approved (pending spec review)
Sub-project 2 of 2. Sub-project 1 (retire tmux) is complete and merged to `main`.

## Overview

`ansidrama` is a working, MIT-licensed Rust CLI (`encode` + `record` → animated
WebP) that has never been publicly released. This sub-project adds everything
needed for a public GitHub release: CI, cross-compiled binaries, `.deb`/`.rpm`
packages, a man page, a changelog, a Makefile, a self-referential README demo,
and a byonk-style release workflow. The structure mirrors the sibling project
`../byonk`, minus its container and docs-site machinery.

The first release is **v0.1.0**, published from **github.com/oetiker/ansidrama**.

## Goals

- CI on every push/PR: format, lint, test, build.
- A one-click GitHub release: version bump → cross-built binaries + deb + rpm →
  GitHub Release with changelog notes.
- Linux `x86_64` + `aarch64` (static musl) and macOS `x86_64` + `aarch64`
  binaries; `.deb` + `.rpm` for Linux.
- A hand-written man page installed by the packages.
- A committed, self-made README demo WebP (the "star of its own trailer").
- Rust-native packaging (no Go/nfpm) — config lives in `Cargo.toml`.

## Non-goals (explicitly out of scope)

- Windows builds (`record` needs a PTY).
- macOS `.app`/`.dmg` (GUI-app formats — wrong for a CLI; tarball + a possible
  future Homebrew tap is the CLI story).
- Container image, mdBook/GitHub-Pages docs site.
- Publishing to crates.io (also blocked by the `vt100` git dependency).
- Replacing libwebp with a pure-Rust encoder (noted as a future follow-up; see
  Build risk).

## Current state

- `Cargo.toml`: `version = "0.1.0"`, `license = "MIT"`, no `repository`/
  `homepage`, no packaging metadata.
- `LICENSE-MIT` present; bundled JetBrains Mono under OFL in `assets/`.
- No `.github/`, no `CHANGES.md`, no `Makefile`, no `man/`, no `docs/demo/`.
- No git remote, no tags.
- The `webp` crate builds libwebp via `cc` (the one C dependency).
- Reference project: `../byonk/.github/workflows/{ci,release}.yml`,
  `../byonk/Cross.toml`, `../byonk/Makefile`.

## Deliverables

### 1. CI — `.github/workflows/ci.yml`

Mirrors byonk. On push/PR to `main`, three jobs (or one with steps):
`cargo fmt --check`, `cargo clippy -- -D warnings`, `cargo test`, `cargo build`,
with `actions/cache` on the cargo registry + target. tmux is **not** installed —
`record` is tmux-free and the `record_smoke` test drives `bash` (present on
runners).

### 2. Release — `.github/workflows/release.yml`

Adapted from byonk; its `build-container`, `build-docs`, `deploy-docs` jobs are
dropped. `workflow_dispatch` with a `release_type` choice (`bugfix`/`feature`/
`major`).

- **version** job: verify `main`; compute the next version from the latest
  `v*.*.*` tag (none → `v0.0.0`, so **`feature` → 0.1.0**); `sed` the version
  into `Cargo.toml`; roll `CHANGES.md` with byonk's perl (move `## Unreleased`
  content into `## <version> - <date>`); commit as `github-actions[bot]`; tag
  `vX.Y.Z`; push `main --tags`. Outputs `version` and `tag`.
- **build-binaries** job: matrix
  - `x86_64-unknown-linux-musl` (ubuntu, `cross`, `cross: true`)
  - `aarch64-unknown-linux-musl` (ubuntu, `cross`, `cross: true`)
  - `x86_64-apple-darwin` (macos)
  - `aarch64-apple-darwin` (macos)
  Build with `cross` (musl legs, `RUSTFLAGS="-C target-feature=+crt-static"`) or
  `cargo` (macOS). Stage a `tar.gz` per target: `ansidrama` + `man/ansidrama.1`
  + `README.md` + `LICENSE-MIT`. Upload as an artifact.
- **deb/rpm** (in the two `linux-musl` legs, where the binary already exists):
  install `cargo-deb` + `cargo-generate-rpm`; run
  `cargo deb --no-build --target <triple>` and
  `cargo generate-rpm --target <triple>`; upload the `.deb` + `.rpm`. (Folding
  packaging into the build leg avoids a separate download-and-restage job.)
- **create-release** job: download all artifacts; slice release notes from
  `CHANGES.md` for this version; `softprops/action-gh-release@v2` attaching all
  tarballs + `.deb` + `.rpm`. Uses `${{ github.repository }}` throughout, so it
  is repo-slug-agnostic.

### 3. `Cross.toml`

Copied from byonk: musl cross images for both linux targets, `RUSTFLAGS`
passthrough (needed so the `webp`/libwebp `cc` build cross-compiles to static
musl).

### 4. Packaging metadata — in `Cargo.toml` (Rust-native)

- `[package.metadata.deb]` for **cargo-deb**: `maintainer = "Tobias Oetiker
  <tobi@oetiker.ch>"`, `license-file`/`depends`/`section`/`priority`, and an
  `assets` list installing the binary → `/usr/bin/ansidrama`, `ansidrama.1` →
  `/usr/share/man/man1/`, `README.md` + `LICENSE-MIT` → `/usr/share/doc/
  ansidrama/`. No `tmux` dependency.
- `[package.metadata.generate-rpm]` for **cargo-generate-rpm**: matching
  `assets`, `summary`, license, and (if needed) `[package.metadata.generate-rpm.
  requires]`.
- Both operate on the pre-built (cross) binary via `--no-build --target
  <triple>`. Stripping handled with `--no-strip` if host strip cannot handle the
  aarch64 target (resolved in the plan).

### 5. Man page — `man/ansidrama.1`

Hand-written roff: NAME, SYNOPSIS (`encode`/`record`), DESCRIPTION, the two
commands, OPTIONS (`-o`/`--out`, `--dump-png`), a CONFIGURATION pointer (TOML
schema summary), the supported **tmux-style key names** (from `keys.rs`),
EXAMPLES, AUTHOR (Tobias Oetiker), LICENSE (MIT). Ships in the tarball and is
installed by deb/rpm. `make man` previews it (`man ./man/ansidrama.1`).

### 6. `CHANGES.md`

byonk format: a `## Unreleased` block with `### New` / `### Changed` / `### Fixed`
subsections. Seeded so the first `feature` release rolls the initial feature set
(encode, record with the embedded terminal, title cards, truecolor, deb/rpm) into
`## 0.1.0 - <date>`. The release workflow's perl and note-slicer depend on this
exact shape.

### 7. `Makefile`

Trimmed byonk style: `fmt`, `lint` (clippy `-D warnings`), `test`, `check`
(fmt+lint+test), `release` (fmt+lint then `cargo build --release`), `package`
(local `cargo deb` + `cargo generate-rpm` for the host target), `man` (preview),
`demo` (regenerate `docs/demo/ansidrama.webp`), `help`. Caps parallelism to 4
cores per the machine policy.

### 8. The "Makes itself" demo

- `demo/hello.toml` — a tiny **real** `record` script that drives `printf`
  (non-recursive; it does NOT run ansidrama), producing a small `hello.webp`.
- `demo/readme.toml` — the outer `record` script that drives an interactive
  `bash` through the trailer:
  1. title card: `ansidrama` / `no browser · no ffmpeg · no tmux`
  2. `cat hello.toml` (the self-referential reveal — a real, colourised script)
  3. `ansidrama record hello.toml` → the **real** `OK: wrote hello.webp (…)` line
  4. `ls -l hello.webp`
  5. title card: `a star is born 🌟`
- Output `docs/demo/ansidrama.webp`, **committed** so GitHub renders it without
  CI. Regenerated by `make demo` (which builds the release binary first and puts
  it on `PATH` for the inner `ansidrama record` call). `out` is resolved relative
  to the config directory, so `readme.toml` sets `out` accordingly to land the
  file at `docs/demo/ansidrama.webp`.
- Determinism: every driven command (`bash`, `cat`, `ls`, `printf`, `ansidrama`)
  is present locally; the inner `record` drives `printf` only, so there is no
  recursion and generation is fast and repeatable.

### 9. README

- Embed `docs/demo/ansidrama.webp` as the hero at the top.
- Expand **Install**: per-target tarball download from the Releases page;
  `.deb`/`.rpm` install lines; `man ansidrama`; keep `cargo install --path .`.
- Add a CI status badge.
- Keep existing content (the two commands, scenes, cards, comparison).

### 10. `Cargo.toml` metadata

Add `repository = "https://github.com/oetiker/ansidrama"` and `homepage`
(same), plus the two `[package.metadata.*]` tables from #4.

## Build risk (the one to verify first)

The `webp` crate compiles **libwebp via `cc`**. The plan's first packaging task
must confirm `cross build --release --target x86_64-unknown-linux-musl` succeeds
(libwebp cross-compiles to static musl) before the full matrix is wired.
Fallback if it fails: switch the Linux targets to `-gnu` (glibc), which still
yields working deb/rpm (glibc is the distro norm). A pure-Rust WebP animation
muxer (over `image-webp`) is the longer-term way to remove this risk and is a
separate future sub-project.

## Testing / verification

- CI itself is the regression net for fmt/lint/test/build.
- `cargo deb` / `cargo generate-rpm` run locally (`make package`) on the host
  target to confirm the metadata produces installable packages; inspect contents
  (`dpkg-deb -c`, `rpm -qlp`) to verify the binary + man page + docs land at the
  right paths.
- `make demo` regenerates the WebP; the committed artifact is eyeballed (dump a
  few frames as PNG) to confirm the trailer renders.
- The release workflow is validated by a real `workflow_dispatch` run after the
  repo exists (see Sequencing) — this is the only step that cannot be exercised
  purely locally.

## Sequencing / handoff

1. Build and commit all of the above on a feature branch; merge to `main`.
2. User creates `github.com/oetiker/ansidrama` and pushes `main` (adds the
   `origin` remote).
3. User runs the **Release** workflow with `release_type = feature` → tags and
   publishes **v0.1.0** with binaries + deb + rpm.

Local, non-GitHub deliverables (CI/release YAML syntax, packaging metadata, man
page, demo, Makefile, README) are fully testable before step 2.

## Out of scope

Windows, `.app`/`.dmg`, container image, docs site, crates.io publish, and the
pure-Rust WebP conversion (a future sub-project, once an animated-WebP muxer
over `image-webp` exists).
