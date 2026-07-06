//! Config schema for the two commands, parsed from TOML.
//!
//! `encode.toml` — a list of frames (each a captured `.ansi` file or a synthetic
//! card) with hold times. `record.toml` — a launch command plus a list of scenes
//! (keystrokes / typed text / friendly mouse / cards) to drive in tmux.

use std::collections::BTreeMap;

use anyhow::{bail, Result};
use serde::Deserialize;

use crate::mouse::{Click, Drag, Scroll};

fn df_hold() -> u16 {
    100
}
fn df_true() -> bool {
    true
}

/// A synthetic "silent-movie" title card.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Card {
    /// Single string; embedded `\n` splits into lines.
    #[serde(default)]
    pub text: Option<String>,
    /// Explicit lines (takes precedence over `text`).
    #[serde(default)]
    pub lines: Option<Vec<String>>,
    #[serde(default = "default_card_fg")]
    pub fg: String,
    #[serde(default = "default_card_bg")]
    pub bg: String,
    #[serde(default)]
    pub bold: bool,
    /// Draw the double-line intertitle frame (default true).
    #[serde(default = "df_true")]
    pub border: bool,
    /// Per-card font size override (else the config's `card_font_px`).
    #[serde(default)]
    pub font_px: Option<f32>,
}

fn default_card_fg() -> String {
    "white".into()
}
fn default_card_bg() -> String {
    "black".into()
}

impl Card {
    pub fn resolved_lines(&self) -> Vec<String> {
        if let Some(l) = &self.lines {
            l.clone()
        } else if let Some(t) = &self.text {
            t.split('\n').map(str::to_string).collect()
        } else {
            Vec::new()
        }
    }
}

// --- encode -----------------------------------------------------------------

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EncodeConfig {
    pub cols: u32,
    pub rows: u32,
    /// Terminal font pixel size — sets the cell size and thus the output resolution.
    #[serde(default = "default_font_px")]
    pub font_px: f32,
    /// Title-card font pixel size.
    #[serde(default = "default_card_font_px")]
    pub card_font_px: f32,
    /// Cap the animation frame rate (clamps each frame's minimum hold).
    #[serde(default = "default_max_fps")]
    pub max_fps: u32,
    #[serde(default)]
    pub out: Option<String>,
    #[serde(default, rename = "frame")]
    pub frames: Vec<FrameSpec>,
}

/// Default terminal font pixel size (small is fine — the capture is dense).
pub fn default_font_px() -> f32 {
    18.0
}
/// Default title-card font pixel size — larger, since cards are read at a glance
/// and are not bound to the terminal cell grid.
pub fn default_card_font_px() -> f32 {
    44.0
}
/// Default frame-rate cap.
pub fn default_max_fps() -> u32 {
    30
}
/// Minimum per-frame hold (centiseconds) implied by a frame-rate cap. `0` = no cap.
pub fn min_hold_cs(max_fps: u32) -> u16 {
    match max_fps {
        0 => 1,
        fps => 100u32.div_ceil(fps).max(1) as u16,
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FrameSpec {
    /// Path to a captured ANSI snapshot (relative to the config file).
    #[serde(default)]
    pub file: Option<String>,
    /// …or a synthetic card instead of a captured frame.
    #[serde(default)]
    pub card: Option<Card>,
    #[serde(default = "df_hold")]
    pub hold_cs: u16,
}

/// What a frame draws — exactly one source.
pub enum FrameSource<'a> {
    File(&'a str),
    Card(&'a Card),
}

impl FrameSpec {
    pub fn source(&self) -> Result<FrameSource<'_>> {
        match (&self.file, &self.card) {
            (Some(f), None) => Ok(FrameSource::File(f)),
            (None, Some(c)) => Ok(FrameSource::Card(c)),
            (None, None) => bail!("frame has neither `file` nor `card`"),
            (Some(_), Some(_)) => bail!("frame has both `file` and `card` — pick one"),
        }
    }
}

// --- record -----------------------------------------------------------------

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RecordConfig {
    /// Shell command line launched inside the tmux pane.
    pub launch: String,
    pub cols: u32,
    pub rows: u32,
    /// Terminal font pixel size — sets the cell size and thus the output resolution.
    #[serde(default = "default_font_px")]
    pub font_px: f32,
    /// Title-card font pixel size (cards are not bound to the terminal cell grid).
    #[serde(default = "default_card_font_px")]
    pub card_font_px: f32,
    /// Cap the animation frame rate (clamps each frame's minimum hold).
    #[serde(default = "default_max_fps")]
    pub max_fps: u32,
    #[serde(default)]
    pub out: Option<String>,
    /// Extra environment for the launched command.
    #[serde(default)]
    pub env: BTreeMap<String, String>,
    /// Milliseconds to wait after launch before the first capture.
    #[serde(default = "default_startup")]
    pub startup_ms: u64,
    /// Milliseconds to let the screen settle after an input before capturing.
    #[serde(default = "default_settle")]
    pub settle_ms: u64,
    /// Default hold (centiseconds) for each per-key / per-typed-char frame.
    #[serde(default = "default_type_cs")]
    pub type_cs: u16,
    /// Default hold (centiseconds) for each mouse-cursor-step frame.
    #[serde(default = "default_move_cs")]
    pub move_cs: u16,
    /// Keys sent to quit the app after recording (e.g. `["M-x"]` or `["q"]`).
    #[serde(default)]
    pub quit_keys: Vec<String>,
    /// Draw a mouse-pointer arrow on click/drag/scroll frames.
    #[serde(default = "df_true")]
    pub cursor: bool,
    #[serde(default, rename = "scene")]
    pub scenes: Vec<Scene>,
}

fn default_startup() -> u64 {
    900
}
fn default_settle() -> u64 {
    350
}
fn default_type_cs() -> u16 {
    9
}
fn default_move_cs() -> u16 {
    4
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Scene {
    /// Hold (centiseconds) for the FINAL frame of this scene — the pause on the
    /// result. Intermediate per-event frames use `type_cs` / `move_cs`.
    #[serde(default = "df_hold")]
    pub hold_cs: u16,
    /// Per-scene override of the typing (per-key / per-char) frame hold.
    #[serde(default)]
    pub type_cs: Option<u16>,
    /// Per-scene override of the mouse-move frame hold.
    #[serde(default)]
    pub move_cs: Option<u16>,
    /// Named tmux keys (e.g. `"Down"`, `"Enter"`, `"C-F5"`) sent in order.
    #[serde(default)]
    pub keys: Option<Vec<String>>,
    /// A string typed literally, character by character.
    #[serde(default)]
    pub text: Option<String>,
    #[serde(default)]
    pub click: Option<Click>,
    #[serde(default)]
    pub drag: Option<Drag>,
    #[serde(default)]
    pub scroll: Option<Scroll>,
    /// A synthetic title card — no terminal interaction, just a held frame.
    #[serde(default)]
    pub card: Option<Card>,
}

/// What a scene does — exactly one action (besides the hold).
pub enum Action<'a> {
    Keys(&'a [String]),
    Text(&'a str),
    Click(&'a Click),
    Drag(&'a Drag),
    Scroll(&'a Scroll),
    Card(&'a Card),
}

impl Scene {
    pub fn action(&self) -> Result<Action<'_>> {
        let mut acts: Vec<Action<'_>> = Vec::new();
        if let Some(k) = &self.keys {
            acts.push(Action::Keys(k));
        }
        if let Some(t) = &self.text {
            acts.push(Action::Text(t));
        }
        if let Some(c) = &self.click {
            acts.push(Action::Click(c));
        }
        if let Some(d) = &self.drag {
            acts.push(Action::Drag(d));
        }
        if let Some(s) = &self.scroll {
            acts.push(Action::Scroll(s));
        }
        if let Some(c) = &self.card {
            acts.push(Action::Card(c));
        }
        match acts.len() {
            0 => bail!("scene has no action"),
            1 => Ok(acts.into_iter().next().unwrap()),
            _ => bail!(
                "scene has more than one action — use one of keys/text/click/drag/scroll/card per scene"
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_record_scene_variants() {
        let cfg: RecordConfig = toml::from_str(
            r##"
            launch = "myapp"
            cols = 80
            rows = 24
            [[scene]]
            keys = ["Down", "Enter"]
            hold_cs = 120
            [[scene]]
            click = { x = 5, y = 6 }
            [[scene]]
            card = { text = "Hello", fg = "#fef9c3" }
            hold_cs = 150
            "##,
        )
        .unwrap();
        assert_eq!(cfg.scenes.len(), 3);
        assert!(matches!(cfg.scenes[0].action().unwrap(), Action::Keys(k) if k.len() == 2));
        assert!(matches!(cfg.scenes[1].action().unwrap(), Action::Click(_)));
        assert!(matches!(cfg.scenes[2].action().unwrap(), Action::Card(_)));
        assert_eq!(
            cfg.scenes[2].card.as_ref().unwrap().resolved_lines(),
            vec!["Hello"]
        );
    }

    #[test]
    fn scene_rejects_two_actions() {
        let cfg: RecordConfig = toml::from_str(
            r##"
            launch = "x"
            cols = 1
            rows = 1
            [[scene]]
            keys = ["a"]
            text = "b"
            "##,
        )
        .unwrap();
        assert!(cfg.scenes[0].action().is_err());
    }

    #[test]
    fn parse_encode_frames() {
        let cfg: EncodeConfig = toml::from_str(
            r##"
            cols = 80
            rows = 24
            [[frame]]
            file = "000.ansi"
            hold_cs = 100
            [[frame]]
            card = { lines = ["A", "B"] }
            "##,
        )
        .unwrap();
        assert_eq!(cfg.frames.len(), 2);
        assert!(matches!(
            cfg.frames[0].source().unwrap(),
            FrameSource::File(_)
        ));
        assert!(matches!(
            cfg.frames[1].source().unwrap(),
            FrameSource::Card(_)
        ));
    }
}
