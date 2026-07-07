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

use crate::chrome::Chrome;
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
    chrome: Chrome,
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
        let renderer = Renderer::new(cfg.font_px);
        let cell_h = renderer.cell_size().1;
        let chrome = match &cfg.chrome {
            Some(c) => Chrome::from_config(c, cell_h, (0, 0, 0)).context("chrome config")?,
            None => Chrome::disabled(),
        };
        Ok(Recorder {
            renderer,
            chrome,
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
        let image = if self.chrome.is_active() {
            self.chrome.matte(&self.renderer, &img)
        } else {
            img
        };
        self.frames.push(Frame {
            image,
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
        let image = if self.chrome.is_active() {
            self.chrome.matte(&self.renderer, &img)
        } else {
            img
        };
        self.frames.push(Frame {
            image,
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
