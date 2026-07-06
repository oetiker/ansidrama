//! The `record` command: drive a command in a detached tmux session, run the
//! scene script (keys / typed text / friendly mouse / cards), capture one
//! coloured frame per scene, rasterize, and encode an animated WebP.

use std::path::Path;
use std::process::Command;
use std::thread::sleep;
use std::time::Duration;

use anyhow::{bail, Context, Result};

use crate::config::{Action, RecordConfig};
use crate::cursor;
use crate::encode::{encode_webp, total_ms, Frame};
use crate::frame;
use crate::raster::Renderer;

/// The cell a mouse action ends on (for the pointer overlay), if any.
fn mouse_target(action: &Action<'_>) -> Option<(u32, u32)> {
    match action {
        Action::Click(c) => Some((c.x, c.y)),
        Action::Drag(d) => Some((d.to[0], d.to[1])),
        Action::Scroll(s) => Some((s.x, s.y)),
        _ => None,
    }
}

const SESSION: &str = "ansidrama_rec";

fn tmux(args: &[&str]) -> Result<Vec<u8>> {
    let out = Command::new("tmux")
        .args(args)
        .output()
        .context("spawn tmux (is it installed?)")?;
    anyhow::ensure!(out.status.success(), "tmux {:?} failed", args);
    Ok(out.stdout)
}

/// Send one key/mouse token. ESC-prefixed tokens are raw byte sequences (mouse
/// reports / literal escapes) sent with `-l`; everything else is a named tmux key.
fn send(token: &str) -> Result<()> {
    if token.starts_with('\x1b') {
        send_literal(token)
    } else {
        tmux(&["send-keys", "-t", SESSION, token]).map(|_| ())
    }
}

/// Send raw bytes to the pane (`send-keys -l`) — for mouse reports and typed text.
fn send_literal(token: &str) -> Result<()> {
    tmux(&["send-keys", "-t", SESSION, "-l", token]).map(|_| ())
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

    // Launch the app in a detached, fixed-size tmux session.
    let _ = tmux(&["kill-session", "-t", SESSION]);
    let launch = format!("{}; tmux wait-for -S ansidrama_done", cfg.launch);
    let (cols, rows) = (cfg.cols.to_string(), cfg.rows.to_string());
    let mut new_args: Vec<&str> =
        vec!["new-session", "-d", "-s", SESSION, "-x", &cols, "-y", &rows];
    // Pass env via `-e KEY=VAL` (tmux ≥ 3.2). Default COLORTERM=truecolor so the
    // app emits 24-bit colour. Keep the strings alive for the call.
    let mut env = cfg.env.clone();
    env.entry("COLORTERM".to_string())
        .or_insert_with(|| "truecolor".to_string());
    let env_args: Vec<String> = env.iter().map(|(k, v)| format!("{k}={v}")).collect();
    for e in &env_args {
        new_args.push("-e");
        new_args.push(e);
    }
    new_args.push("bash");
    new_args.push("-lc");
    new_args.push(&launch);
    tmux(&new_args).context("tmux new-session")?;
    // Tell tmux the virtual terminal is 24-bit-colour capable, so truecolor SGR
    // survives into `capture-pane -e` (else it quantises to 256). Appended, so it
    // does not clobber an existing terminal-overrides on a shared server.
    let _ = tmux(&["set-option", "-ga", "terminal-overrides", ",*:RGB"]);

    let renderer = Renderer::new();
    let mut frames: Vec<Frame> = Vec::with_capacity(cfg.scenes.len());
    let dump_dir = dump_png.map(Path::to_path_buf);
    if let Some(d) = &dump_dir {
        std::fs::create_dir_all(d).ok();
    }

    sleep(Duration::from_millis(cfg.startup_ms)); // settle the first paint

    for (i, scene) in cfg.scenes.iter().enumerate() {
        let action = scene.action()?;
        let mouse = mouse_target(&action);
        let mut img = match &action {
            Action::Card(card) => {
                // Synthetic frame — nothing sent to the terminal.
                frame::render_card(&renderer, cfg.cols, cfg.rows, card)?
            }
            _ => {
                run_terminal_action(&action, cfg.key_delay_ms)?;
                sleep(Duration::from_millis(cfg.settle_ms));
                let captured = tmux(&["capture-pane", "-t", SESSION, "-e", "-p", "-N"])?;
                frame::render_ansi(
                    &renderer,
                    cfg.cols,
                    cfg.rows,
                    &String::from_utf8_lossy(&captured),
                )
            }
        };
        // Draw the pointer where the mouse acted, so click/drag/scroll read clearly.
        if cfg.cursor {
            if let Some((mx, my)) = mouse {
                let (px, py) = renderer.cell_origin(mx, my);
                cursor::stamp(&mut img, px, py);
            }
        }
        if let Some(d) = &dump_dir {
            let _ = img.save(d.join(format!("scene{i:02}.png")));
        }
        frames.push(Frame {
            image: img,
            hold_cs: scene.hold_cs,
        });
        eprintln!("  scene {i:02} captured");
    }

    // Quit the app and tear down the session.
    for k in &cfg.quit_keys {
        let _ = send(k);
    }
    let _ = tmux(&["kill-session", "-t", SESSION]);

    let webp = encode_webp(&frames)?;
    if let Some(parent) = out_path.parent() {
        std::fs::create_dir_all(parent).ok();
    }
    std::fs::write(&out_path, &webp).with_context(|| format!("write {}", out_path.display()))?;
    let (w, h) = frames[0].image.dimensions();
    eprintln!(
        "OK: wrote {} ({} scenes, {w}x{h}px, {:.1}s loop)",
        out_path.display(),
        frames.len(),
        total_ms(&frames) as f32 / 1000.0
    );
    Ok(())
}

/// Send a scene's terminal action (everything except a card), pausing
/// `key_delay_ms` between tokens.
fn run_terminal_action(action: &Action<'_>, key_delay_ms: u64) -> Result<()> {
    match action {
        // Typed text goes out literally, one character at a time.
        Action::Text(s) => {
            for c in s.chars() {
                send_literal(&c.to_string())?;
                sleep(Duration::from_millis(key_delay_ms));
            }
        }
        _ => {
            let tokens: Vec<String> = match action {
                Action::Keys(keys) => keys.to_vec(),
                Action::Click(c) => c.sequences(),
                Action::Drag(d) => d.sequences(),
                Action::Scroll(s) => s.sequences(),
                Action::Text(_) | Action::Card(_) => unreachable!(),
            };
            for t in &tokens {
                send(t)?;
                sleep(Duration::from_millis(key_delay_ms));
            }
        }
    }
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
