//! The `record` command: drive a command in an embedded terminal (its own PTY
//! plus a `vt100` parser — no tmux) and turn a scene script into an animation.
//!
//! Three phases, deliberately separated:
//!
//! 1. **Drive** — `Recorder` sends one input at a time, waits for its result,
//!    and records a [`Mark`] on the timeline. It never renders anything.
//! 2. **Capture** — a [`Sampler`] thread snapshots the screen continuously, so
//!    nothing the app draws between inputs is missed and no capture is ever
//!    blocked behind a rasterisation.
//! 3. **Assemble, then render** — [`assemble`] turns the state log plus the
//!    marks into frame specs, and only those surviving specs are rasterised.
//!
//! Each scene still expands into many frames — one per key, one per typed
//! character, one per mouse-cursor cell-step — so keyboard and mouse actions
//! play out step by step. Cursor-only moves reuse the last screen; drags
//! re-capture each step so live UI (e.g. a resize preview) is shown.

use std::path::Path;
use std::time::{Duration, Instant};

use anyhow::{bail, Context, Result};
use image::RgbaImage;

// Aliased: `config::FrameSource` is a different type with the same name (the
// `encode` path's file-or-card), so neither name is imported bare here.
use crate::assemble::FrameSource as Source;
use crate::assemble::{assemble, FrameKind, InputMark, Mark};
use crate::chrome::Chrome;
use crate::config::{min_hold_cs, Action, RecordConfig, Scene};
use crate::cursor;
use crate::encode::{encode_webp, total_ms, Frame};
use crate::frame;
use crate::grid::Cell;
use crate::mouse::{Button, Scroll};
use crate::pattern::Pattern;
use crate::raster::Renderer;
use crate::sampler::{Sampler, WaitOutcome};
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

/// Drives the embedded terminal and accumulates timeline marks. Rendering is
/// not its job — see [`run`].
struct Recorder<'a> {
    cfg: &'a RecordConfig,
    /// Declared before `term` so the sampler thread is joined before the child
    /// is killed and the parser goes away.
    sampler: Sampler,
    term: Term,
    marks: Vec<Mark>,
    /// One compiled `await` per scene, compiled at startup so a bad pattern
    /// fails in milliseconds rather than minutes into a recording.
    patterns: Vec<Option<Pattern>>,
    /// Where the pointer was left standing, so the next `move_to` knows where
    /// to animate *from*. Not the same thing as "does this frame draw a
    /// pointer" — that is decided per input by the caller of `capture`.
    last_mouse: Option<(u32, u32)>,
    min_cs: u16,
}

impl<'a> Recorder<'a> {
    fn spawn(cfg: &'a RecordConfig) -> Result<Recorder<'a>> {
        let patterns = cfg
            .scenes
            .iter()
            .enumerate()
            .map(|(i, s)| s.pattern(cfg.rows).with_context(|| format!("scene {i} await")))
            .collect::<Result<Vec<_>>>()?;
        let term = Term::spawn(cfg.cols as u16, cfg.rows as u16, &cfg.launch, &cfg.env)
            .context("start embedded terminal")?;
        let sampler = Sampler::start(
            term.handle(),
            Duration::from_millis(cfg.sample_ms),
            Duration::from_millis(cfg.persist_ms),
            (cfg.max_capture_mb as usize).saturating_mul(1024 * 1024),
        );
        Ok(Recorder {
            cfg,
            sampler,
            term,
            marks: Vec::new(),
            patterns,
            last_mouse: None,
            min_cs: min_hold_cs(cfg.max_fps),
        })
    }

    /// Wait for the first paint before anything is captured.
    ///
    /// `startup_ms` is a **floor**, not just a cap. A short stability window
    /// would otherwise be satisfied during the brief quiet *before* the first
    /// paint — an interactive shell has not printed its prompt yet — and that
    /// would seed a blank screen, with the first keystrokes landing ahead of
    /// the prompt (`pri` then `bash-5.2$` → `pribash-5.2$`). So we wait the
    /// floor out unconditionally, and only then run an ordinary stability wait
    /// to let whatever is still arriving settle.
    fn seed(&mut self) -> Result<()> {
        std::thread::sleep(Duration::from_millis(self.cfg.startup_ms));
        self.sampler
            .wait(
                None,
                Duration::from_millis(self.cfg.change_ms),
                Duration::from_millis(self.cfg.stable_ms),
                Duration::from_millis(self.cfg.wait_cap_ms),
            )
            .context("waiting for the first paint to settle")?;
        Ok(())
    }

    /// Wait for this input's result, then mark it on the timeline.
    ///
    /// `at` is when the input was sent; it bounds which sampled states belong
    /// to this input. `want` is the scene's `await` pattern, and is passed only
    /// for a scene's *final* input — an intermediate keystroke has no reason to
    /// produce the scene's finished screen. `mouse` is the pointer position
    /// this input's frames draw, or `None` for keyboard frames (which draw the
    /// app's text caret instead).
    fn capture(
        &mut self,
        scene: usize,
        at: Instant,
        authored_cs: u16,
        mouse: Option<(u32, u32)>,
        want: bool,
    ) -> Result<()> {
        let animated = self.cfg.realtime || self.cfg.scenes[scene].animated;
        let out = if animated {
            // A screen that never holds still has no "settled" moment to wait
            // for. Dwell for the authored time and take whatever is on screen.
            let dwell = Duration::from_millis(authored_cs as u64 * 10);
            let before = self.sampler.states().last_change();
            std::thread::sleep(dwell);
            let mut a = self.sampler.states();
            let now = Instant::now();
            // `moved` here means the screen actually changed during the dwell.
            // `moved=no` on an animated scene is a real red flag: a screen
            // declared animated that in fact held still contributes no frames
            // at all, because `assemble` measures animated scenes rather than
            // owing them one frame each.
            let moved = a.last_change() > before;
            // `assemble` gates the settled state on `!animated`, so this index
            // is never read downstream — an animated scene measures every
            // frame. The call is still load-bearing for its side effect:
            // force-committing pins the screen at the end of the dwell into
            // the state log, which is where the scene's last frame comes from.
            let state = a.settled_index(now);
            let states = a.committed().len();
            // The trace write can block on a slow stderr; the accumulator's
            // mutex must not be held across it, or the sampler thread stalls.
            drop(a);
            crate::sampler::trace("dwell", dwell, moved, None, states);
            WaitOutcome { state, hit_cap: false }
        } else {
            let want = if want {
                self.patterns[scene].as_ref()
            } else {
                None
            };
            let timeout = match want {
                Some(_) => Duration::from_millis(
                    self.cfg.scenes[scene].await_ms.unwrap_or(self.cfg.await_ms),
                ),
                None => Duration::from_millis(self.cfg.wait_cap_ms),
            };
            self.sampler
                .wait(
                    want,
                    Duration::from_millis(self.cfg.change_ms),
                    Duration::from_millis(self.cfg.stable_ms),
                    timeout,
                )
                .with_context(|| format!("scene {scene}"))?
        };
        self.marks.push(Mark::Input(InputMark {
            t: at,
            scene,
            settled: out.state,
            authored_cs,
            mouse,
            animated,
        }));
        Ok(())
    }

    /// Mark a synthetic card. Nothing is sent and nothing is waited for — a
    /// card does not touch the terminal.
    fn push_card(&mut self, scene: usize, hold_cs: u16) {
        self.marks.push(Mark::Card { scene, hold_cs });
    }

    /// Animate the pointer from its last position to `target` over the current
    /// (unchanged) screen — one frame per cell. Nothing is sent to the app and
    /// nothing is waited for. Leaves `last_mouse` set.
    fn move_to(&mut self, scene: usize, target: (u32, u32), move_cs: u16) {
        if let Some(from) = self.last_mouse {
            for mouse in line_cells(from, target) {
                self.marks.push(Mark::MouseMove { scene, mouse, hold_cs: move_cs });
            }
        }
        self.last_mouse = Some(target);
    }

    fn process(&mut self, i: usize) -> Result<()> {
        // Copy the `&'a` reference out so the scene borrow does not conflict
        // with the `&mut self` calls below.
        let cfg = self.cfg;
        let scene = &cfg.scenes[i];
        let type_cs = scene.type_cs.unwrap_or(cfg.type_cs);
        let move_cs = scene.move_cs.unwrap_or(cfg.move_cs);
        let hold_cs = scene.hold_cs;
        match scene.action()? {
            Action::Card(_) => self.push_card(i, hold_cs),

            Action::Keys(keys) => {
                if keys.is_empty() {
                    // "Hold the current screen" — nothing is sent, but the
                    // scene still owes one wait and one frame.
                    let at = Instant::now();
                    self.capture(i, at, hold_cs, None, true)?;
                } else {
                    let last = keys.len() - 1;
                    for (n, k) in keys.iter().enumerate() {
                        let at = Instant::now();
                        self.term.send_key(k)?;
                        self.capture(
                            i,
                            at,
                            if n == last { hold_cs } else { type_cs },
                            None,
                            n == last,
                        )?;
                    }
                }
            }

            Action::Text(s) => {
                let chars: Vec<char> = s.chars().collect();
                let last = chars.len().saturating_sub(1);
                for (n, c) in chars.iter().enumerate() {
                    let at = Instant::now();
                    self.term.send_bytes(c.to_string().as_bytes())?;
                    self.capture(
                        i,
                        at,
                        if n == last { hold_cs } else { type_cs },
                        None,
                        n == last,
                    )?;
                }
            }

            Action::Click(c) => {
                let at_cell = (c.x, c.y);
                let b = c.button;
                self.move_to(i, at_cell, move_cs);
                let t = Instant::now();
                self.term.send_bytes(sgr(b, at_cell, true).as_bytes())?; // press
                self.capture(i, t, move_cs, Some(at_cell), false)?;
                // The release is the last thing the app sees, so it carries the
                // scene's `await`.
                let t = Instant::now();
                self.term.send_bytes(sgr(b, at_cell, false).as_bytes())?; // release
                self.capture(i, t, hold_cs, Some(at_cell), true)?;
            }

            Action::Drag(d) => {
                let from = (d.from[0], d.from[1]);
                let to = (d.to[0], d.to[1]);
                let b = d.button;
                self.move_to(i, from, move_cs);
                let t = Instant::now();
                self.term.send_bytes(sgr(b, from, true).as_bytes())?; // press
                self.capture(i, t, move_cs, Some(from), false)?;
                for cell in line_cells(from, to) {
                    let t = Instant::now();
                    self.term.send_bytes(sgr_motion(b, cell).as_bytes())?; // drag
                    self.capture(i, t, move_cs, Some(cell), false)?;
                }
                let t = Instant::now();
                self.term.send_bytes(sgr(b, to, false).as_bytes())?; // release
                self.capture(i, t, hold_cs, Some(to), true)?;
                self.last_mouse = Some(to);
            }

            Action::Scroll(s) => {
                let at_cell = (s.x, s.y);
                self.move_to(i, at_cell, move_cs);
                let seqs = scroll_sequences(s);
                let last = seqs.len().saturating_sub(1);
                for (n, seq) in seqs.iter().enumerate() {
                    let t = Instant::now();
                    self.term.send_bytes(seq.as_bytes())?;
                    self.capture(
                        i,
                        t,
                        if n == last { hold_cs } else { move_cs },
                        Some(at_cell),
                        n == last,
                    )?;
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

/// `scene 03 (keys)` — enough to find the scene in the config by eye.
fn scene_label(i: usize, s: &Scene) -> String {
    let kind = match s.action() {
        Ok(Action::Keys(_)) => "keys",
        Ok(Action::Text(_)) => "text",
        Ok(Action::Click(_)) => "click",
        Ok(Action::Drag(_)) => "drag",
        Ok(Action::Scroll(_)) => "scroll",
        Ok(Action::Card(_)) => "card",
        Err(_) => "invalid",
    };
    format!("scene {i:02} ({kind})")
}

pub fn run(config_path: &Path, out_override: Option<&Path>, dump_png: Option<&Path>) -> Result<()> {
    let text = std::fs::read_to_string(config_path)
        .with_context(|| format!("read config {}", config_path.display()))?;
    let cfg: RecordConfig = toml::from_str(&text).context("parse record config")?;
    if cfg.scenes.is_empty() {
        bail!("config has no [[scene]] entries");
    }
    cfg.validate().context("record config")?;
    let base = config_path.parent().unwrap_or(Path::new("."));
    let out_path = resolve_out(out_override, cfg.out.as_deref(), base, config_path)?;

    // --- drive ---------------------------------------------------------
    let mut rec = Recorder::spawn(&cfg)?;
    rec.seed()?;
    for i in 0..cfg.scenes.len() {
        rec.process(i)?;
        eprintln!("  scene {i:02} → {} marks total", rec.marks.len());
        // A child that exits once the script is finished is a legitimate end.
        // One that exits with scenes still to play is not: everything after
        // this point would be recorded against a dead terminal.
        if rec.term.handle().is_eof() && i + 1 < cfg.scenes.len() {
            let remaining: Vec<String> = cfg.scenes[i + 1..]
                .iter()
                .enumerate()
                .map(|(n, s)| scene_label(i + 1 + n, s))
                .collect();
            bail!(
                "the launched command exited after {}, with {} scene(s) left to play: {}\n\
                 keep the command alive for the whole script (e.g. append `; sleep 5`), \
                 or drop the scenes it cannot answer",
                scene_label(i, &cfg.scenes[i]),
                remaining.len(),
                remaining.join(", "),
            );
        }
    }
    let end = Instant::now();

    // Quit the app now, while the sampler is still free to run: states it
    // records after `end` fall outside every input's window and are ignored.
    for k in &cfg.quit_keys {
        let _ = rec.term.send_key(k);
    }

    // --- assemble ------------------------------------------------------
    // The guard is held across rendering: the sampler thread simply blocks on
    // it, and the state log must not move under us while we index into it.
    let acc = rec.sampler.states();
    let state_times: Vec<Instant> = acc.committed().iter().map(|s| s.t).collect();
    let specs = assemble(&state_times, end, &rec.marks, rec.min_cs);

    // --- render --------------------------------------------------------
    let renderer = Renderer::new(cfg.font_px);
    let cell_h = renderer.cell_size().1;
    let chrome = match &cfg.chrome {
        Some(c) => Chrome::from_config(c, cell_h, (0, 0, 0)).context("chrome config")?,
        None => Chrome::disabled(),
    };
    let mut frames: Vec<Frame> = Vec::with_capacity(specs.len());
    // The last *clean* screen render, with no pointer or caret drawn on it.
    // `Reuse` must start from that, not from the composited frame, or pointers
    // would smear across a cursor-only move. Cards do not replace it, so a
    // pointer move after a card still reuses the terminal screen.
    let mut last_screen: Option<RgbaImage> = None;
    for spec in &specs {
        let mut img = match &spec.source {
            Source::State(i) => {
                let img = renderer.render(&acc.committed()[*i].grid, cfg.cols, cfg.rows);
                last_screen = Some(img.clone());
                img
            }
            Source::Card(s) => frame::render_card(
                &renderer,
                cfg.cols,
                cfg.rows,
                cfg.scenes[*s]
                    .card
                    .as_ref()
                    .expect("a card mark is only pushed for a scene with a card"),
                cfg.card_font_px,
                cfg.card_subtitle_px,
            )?,
            // Usually a pointer-only move follows a rendered screen, but not
            // always: an animated input contributes no frames at all when no
            // state timestamp falls inside its window, so under `realtime` two
            // mouse scenes over an unchanging screen reach here with nothing
            // rendered yet. Fall back to a committed state — a slightly early
            // screen is a far better outcome than a panic on a config that
            // parses cleanly.
            Source::Reuse => match &last_screen {
                Some(img) => img.clone(),
                None => {
                    let state = acc
                        .committed()
                        .last()
                        .expect("Sampler::start commits one state before any frame exists");
                    let img = renderer.render(&state.grid, cfg.cols, cfg.rows);
                    last_screen = Some(img.clone());
                    img
                }
            },
        };

        // A frame with a pointer position draws the pointer; otherwise
        // (keyboard/typing frames) it draws the app's text caret, if visible.
        if cfg.cursor {
            if let Some((x, y)) = spec.mouse {
                let (px, py) = renderer.cell_origin(x, y);
                cursor::stamp(&mut img, px, py);
            } else if let Source::State(i) = &spec.source {
                let state = &acc.committed()[*i];
                if let Some((cx, cy)) = state.caret {
                    let cell = state
                        .grid
                        .get(cy as usize)
                        .and_then(|r| r.get(cx as usize))
                        .copied()
                        .unwrap_or(Cell {
                            ch: ' ',
                            fg: (0, 0, 0),
                            bg: (255, 255, 255),
                            bold: false,
                        });
                    renderer.draw_block_cursor(&mut img, cx + 1, cy + 1, &cell);
                }
            }
        }

        frames.push(Frame {
            image: if chrome.is_active() {
                chrome.matte(&renderer, &img)
            } else {
                img
            },
            hold_cs: spec.hold_cs,
        });
    }
    drop(acc);
    drop(rec);

    // Optional per-frame PNG dump for inspection, plus a manifest mapping
    // each frame back to its scene. `record` no longer prints a running
    // `scene N -> M frames total` total that a bisect can do arithmetic on —
    // app-driven frames make a scene's contribution unpredictable — so the
    // manifest is what replaces that arithmetic with a lookup.
    if let Some(d) = dump_png {
        std::fs::create_dir_all(d).ok();
        for (i, f) in frames.iter().enumerate() {
            let _ = f.image.save(d.join(format!("frame{i:04}.png")));
        }
        let mut man = String::from("frame\tscene\tinput\tkind\thold_cs\n");
        for (i, spec) in specs.iter().enumerate() {
            let kind = match spec.kind {
                FrameKind::InputDriven => "input-driven",
                FrameKind::AppDriven => "app-driven",
                FrameKind::Card => "card",
            };
            let input = spec.input.map(|n| n.to_string()).unwrap_or_else(|| "-".to_string());
            man.push_str(&format!("{i:04}\t{}\t{input}\t{kind}\t{}\n", spec.scene, spec.hold_cs));
        }
        std::fs::write(d.join("manifest.tsv"), man).ok();
    }

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

#[cfg(all(test, unix))]
mod drive_tests {
    use super::*;
    use crate::pattern::screen_text;

    /// Drive a whole config through the recorder — no rendering, no output
    /// file — and return, for each input mark in order, the text of the screen
    /// its wait settled on.
    fn settled_screens(src: &str) -> Result<Vec<String>> {
        let cfg: RecordConfig = toml::from_str(src).unwrap();
        cfg.validate().unwrap();
        let mut rec = Recorder::spawn(&cfg)?;
        rec.seed()?;
        for i in 0..cfg.scenes.len() {
            rec.process(i)?;
        }
        let acc = rec.sampler.states();
        Ok(rec
            .marks
            .iter()
            .filter_map(|m| match m {
                Mark::Input(i) => Some(screen_text(&acc.committed()[i.settled].grid)),
                _ => None,
            })
            .collect())
    }

    /// A scene's `await` belongs to its LAST input, not its first — the rule
    /// that decides whether a recording is correct or silently one screen
    /// stale.
    ///
    /// The child prints `ONE` after the first key and `DONE` only after the
    /// second. Attached to the last key, as it must be, both waits settle
    /// normally. Attached to the first key — or to every key — that first wait
    /// would spend its whole `await_ms` hunting a screen that cannot exist
    /// until the second key is sent, and `settled_screens` returns an error
    /// instead. So this test discriminates the wiring rather than merely
    /// exercising it.
    #[test]
    fn await_is_attached_to_the_scenes_final_key() {
        let src = r#"
launch = "stty -echo; printf 'READY'; read -n1 a; printf ' ONE'; read -n1 b; printf ' DONE'; sleep 5"
cols = 30
rows = 3
startup_ms = 300
await_ms = 2500

[[scene]]
keys = ["a", "b"]
await = "DONE"
"#;
        let screens = settled_screens(src).expect("an await on the final key must settle");
        assert_eq!(screens.len(), 2, "one input mark per key");
        assert!(
            screens[0].contains("ONE") && !screens[0].contains("DONE"),
            "the first key settles on its own reply, before DONE exists: {:?}",
            screens[0]
        );
        assert!(
            screens[1].contains("DONE"),
            "the last key must settle on the awaited screen: {:?}",
            screens[1]
        );
    }

    /// An empty `keys = []` scene sends nothing but still owes exactly one
    /// wait and one mark — otherwise it contributes no frame at all.
    #[test]
    fn an_empty_keys_scene_still_produces_one_mark() {
        let src = r#"
launch = "printf 'HOLD ME'; sleep 5"
cols = 30
rows = 3
startup_ms = 300

[[scene]]
keys = []
hold_cs = 20
"#;
        let screens = settled_screens(src).unwrap();
        assert_eq!(screens.len(), 1, "exactly one mark for a hold-the-screen scene");
        assert!(screens[0].contains("HOLD ME"), "settled on: {:?}", screens[0]);
    }
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
