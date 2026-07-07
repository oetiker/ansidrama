//! ansidrama — assemble or record a terminal session into a crisp animated WebP.
//!
//! A *drama* is a sequence of frames, each held for a duration. A frame is either
//! a captured terminal snapshot (ANSI text, e.g. from `tmux capture-pane -e -p
//! -N`) or a synthetic silent-movie **title card**. Two entry points:
//!
//! - [`encode`] — the primitive: a list of ANSI snapshots + cards + holds → WebP.
//!   No terminal driving; bring your own frames.
//! - [`record`] — drive a command in an embedded terminal (its own PTY + VT
//!   parser, no tmux) per a scene script, capture each frame, then hand off to
//!   the same encode path.

use std::path::Path;

use anyhow::{bail, Context, Result};

pub mod chrome;
pub mod color;
pub mod config;
pub mod cursor;
pub mod encode;
pub mod frame;
pub mod grid;
#[cfg(unix)]
pub mod keys;
pub mod mouse;
pub mod raster;
#[cfg(unix)]
pub mod record;
#[cfg(unix)]
pub mod term;

use crate::chrome::Chrome;
use crate::config::{EncodeConfig, FrameSource};
use crate::encode::{encode_webp, total_ms, Frame};
use crate::raster::Renderer;

/// Run the `encode` command: build frames from an `encode.toml` (captured `.ansi`
/// files and/or synthetic cards) and write an animated WebP.
pub fn encode(
    config_path: &Path,
    out_override: Option<&Path>,
    dump_png: Option<&Path>,
) -> Result<()> {
    let text = std::fs::read_to_string(config_path)
        .with_context(|| format!("read config {}", config_path.display()))?;
    let cfg: EncodeConfig = toml::from_str(&text).context("parse encode config")?;
    if cfg.frames.is_empty() {
        bail!("config has no [[frame]] entries");
    }
    let base = config_path.parent().unwrap_or(Path::new("."));

    let out_path = match (out_override, cfg.out.as_deref()) {
        (Some(o), _) => o.to_path_buf(),
        (None, Some(o)) => base.join(o),
        (None, None) => bail!("no output path: pass -o <file> or set `out = ...` in the config"),
    };

    if let Some(d) = dump_png {
        std::fs::create_dir_all(d).ok();
    }
    let renderer = Renderer::new(cfg.font_px);
    let cell_h = renderer.cell_size().1;
    let chrome = match &cfg.chrome {
        Some(c) => Chrome::from_config(c, cell_h, (0, 0, 0)).context("chrome config")?,
        None => Chrome::disabled(),
    };
    let min_cs = crate::config::min_hold_cs(cfg.max_fps);
    let mut frames: Vec<Frame> = Vec::with_capacity(cfg.frames.len());
    for (i, spec) in cfg.frames.iter().enumerate() {
        let image = match spec.source()? {
            FrameSource::File(f) => {
                let p = base.join(f);
                let ansi = std::fs::read_to_string(&p)
                    .with_context(|| format!("read frame {}", p.display()))?;
                frame::render_ansi(&renderer, cfg.cols, cfg.rows, &ansi)
            }
            FrameSource::Card(c) => frame::render_card(
                &renderer,
                cfg.cols,
                cfg.rows,
                c,
                cfg.card_font_px,
                cfg.card_subtitle_px,
            )?,
        };
        let image = if chrome.is_active() {
            chrome.matte(&renderer, &image)
        } else {
            image
        };
        if let Some(d) = dump_png {
            let _ = image.save(d.join(format!("frame{i:02}.png")));
        }
        frames.push(Frame {
            image,
            hold_cs: spec.hold_cs.max(min_cs),
        });
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

/// Run the `record` command (see [`record::run`]).
#[cfg(unix)]
pub fn record(
    config_path: &Path,
    out_override: Option<&Path>,
    dump_png: Option<&Path>,
) -> Result<()> {
    record::run(config_path, out_override, dump_png)
}
