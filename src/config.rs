//! Config schema for the two commands, parsed from TOML.
//!
//! `encode.toml` — a list of frames (each a captured `.ansi` file or a synthetic
//! card) with hold times. `record.toml` — a launch command plus a list of scenes
//! (keystrokes / typed text / friendly mouse / cards) to drive in an embedded terminal.

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
fn default_chrome_bar() -> String {
    "#2b2b2b".into()
}
fn default_chrome_text() -> String {
    "#d0d0d0".into()
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
    /// Per-card title-font override (else the config's `card_font_px`).
    #[serde(default)]
    pub font_px: Option<f32>,
    /// Per-card subtitle-font override (else the config's `card_subtitle_px`).
    #[serde(default)]
    pub subtitle_px: Option<f32>,
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

/// Window-chrome style drawn around the terminal screen area.
#[derive(Deserialize, Clone, Copy, PartialEq, Debug, Default)]
#[serde(rename_all = "lowercase")]
pub enum ChromeStyle {
    /// No title bar — padding only (or nothing).
    #[default]
    None,
    /// macOS: three traffic-light dots top-left, title centered.
    Macos,
    /// Generic Linux: a single close button top-right, title left-aligned.
    Linux,
}

/// Optional window chrome + padding around the cell grid. Absent ⇒ no change.
#[derive(Deserialize, Clone)]
#[serde(deny_unknown_fields)]
pub struct ChromeConfig {
    #[serde(default)]
    pub style: ChromeStyle,
    /// Title shown in the bar (empty ⇒ blank bar).
    #[serde(default)]
    pub title: String,
    /// Terminal-bg-filled inset (px) around the cells (works even with style "none").
    #[serde(default)]
    pub padding: u32,
    /// Title-bar color.
    #[serde(default = "default_chrome_bar")]
    pub bar: String,
    /// Title-text (and Linux close-glyph) color.
    #[serde(default = "default_chrome_text")]
    pub text: String,
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
    /// Title-card subtitle font pixel size (lines after the first).
    #[serde(default = "default_card_subtitle_px")]
    pub card_subtitle_px: f32,
    /// Cap the animation frame rate (clamps each frame's minimum hold).
    #[serde(default = "default_max_fps")]
    pub max_fps: u32,
    #[serde(default)]
    pub out: Option<String>,
    #[serde(default)]
    pub chrome: Option<ChromeConfig>,
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
/// Default card subtitle font size (lines after the first).
pub fn default_card_subtitle_px() -> f32 {
    22.0
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
    /// Shell command line launched inside the embedded terminal.
    pub launch: String,
    pub cols: u32,
    pub rows: u32,
    /// Terminal font pixel size — sets the cell size and thus the output resolution.
    #[serde(default = "default_font_px")]
    pub font_px: f32,
    /// Title-card font pixel size (cards are not bound to the terminal cell grid).
    #[serde(default = "default_card_font_px")]
    pub card_font_px: f32,
    /// Title-card subtitle font pixel size (lines after the first).
    #[serde(default = "default_card_subtitle_px")]
    pub card_subtitle_px: f32,
    /// Cap the animation frame rate (clamps each frame's minimum hold).
    #[serde(default = "default_max_fps")]
    pub max_fps: u32,
    #[serde(default)]
    pub out: Option<String>,
    #[serde(default)]
    pub chrome: Option<ChromeConfig>,
    /// Extra environment for the launched command.
    #[serde(default)]
    pub env: BTreeMap<String, String>,
    /// Milliseconds to wait after launch before the first capture — a floor,
    /// so a slow first paint is fully drawn before anything is captured.
    #[serde(default = "default_startup")]
    pub startup_ms: u64,
    /// Grid snapshot interval.
    #[serde(default = "d_sample")]
    pub sample_ms: u64,
    /// Grace for the app's first grid change after an input. Only spent in full
    /// by an input that draws nothing.
    #[serde(default = "d_change")]
    pub change_ms: u64,
    /// How long the grid must hold still to call a screen settled (pacing).
    #[serde(default = "d_stable")]
    pub stable_ms: u64,
    /// How long a state must persist to earn a frame (assembly).
    #[serde(default = "d_persist")]
    pub persist_ms: u64,
    /// Bound on a wait with no `await`.
    #[serde(default = "d_wait_cap")]
    pub wait_cap_ms: u64,
    /// Default `await` timeout.
    #[serde(default = "d_await")]
    pub await_ms: u64,
    /// Play the whole recording at measured time.
    #[serde(default)]
    pub realtime: bool,
    /// Backstop on accumulated grid memory.
    #[serde(default = "d_max_mb")]
    pub max_capture_mb: u64,
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
fn default_type_cs() -> u16 {
    9
}
fn default_move_cs() -> u16 {
    4
}

fn d_sample() -> u64 {
    10
}
fn d_change() -> u64 {
    150
}
fn d_stable() -> u64 {
    40
}
fn d_persist() -> u64 {
    40
}
fn d_wait_cap() -> u64 {
    3000
}
fn d_await() -> u64 {
    5000
}
fn d_max_mb() -> u64 {
    256
}

/// `await = "text"` or `await = { find = "text", row = -1 }`.
#[derive(Deserialize)]
#[serde(untagged)]
pub enum AwaitSpec {
    Text(String),
    Scoped(ScopedAwait),
}

/// The table form of `await`. A named struct rather than an inline variant
/// because `deny_unknown_fields` is a container attribute — serde rejects it on
/// a variant — and without it `await = { find = "x", typo = 1 }` would silently
/// drop `typo`, which is against this config's fail-loudly posture.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScopedAwait {
    pub find: String,
    #[serde(default)]
    pub row: Option<i32>,
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
    /// What this scene's finished screen looks like. Declaring it replaces the
    /// timing guess with a fact, and a failure aborts the run.
    #[serde(default, rename = "await")]
    pub await_spec: Option<AwaitSpec>,
    /// Per-scene `await` timeout override.
    #[serde(default)]
    pub await_ms: Option<u64>,
    /// This screen never holds still (spinner, clock, progress bar).
    #[serde(default)]
    pub animated: bool,
}

impl RecordConfig {
    /// Reject configurations whose parts contradict each other, at load rather
    /// than silently mid-recording.
    ///
    /// Every case here is an `await` that *cannot* be honoured. An unhonoured
    /// `await` is the worst possible failure for this feature: the author has
    /// declared what "done" looks like, the parser has blessed it, and the
    /// recorder then captures whatever a timing guess happens to land on — the
    /// silently-wrong-frame shape the whole `await` mechanism exists to remove.
    /// So none of these are warnings.
    pub fn validate(&self) -> Result<()> {
        for (i, s) in self.scenes.iter().enumerate() {
            if s.await_spec.is_none() {
                continue;
            }
            if s.card.is_some() {
                bail!(
                    "scene {i} sets `await` on a `card`, but a card is a synthetic frame \
                     that never touches the terminal — there is no screen to wait for.\n\
                     remove the `await` from that scene"
                );
            }
            if s.animated {
                bail!(
                    "scene {i} sets both `await` and `animated = true`, but an animated \
                     scene never waits for a settled screen — it dwells for each input's \
                     authored time and captures whatever is there, so the `await` could \
                     only be ignored.\n\
                     remove one of the two"
                );
            }
            if self.realtime {
                bail!(
                    "scene {i} sets `await`, but the config sets `realtime = true`, which \
                     plays every scene at measured time and never waits — the `await` \
                     could only be ignored.\n\
                     remove the `await`, or remove `realtime`"
                );
            }
        }
        Ok(())
    }
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

    /// Compile this scene's `await`, validating the row against the screen.
    /// Called at load so a bad pattern fails in milliseconds, not minutes in.
    pub fn pattern(&self, rows: u32) -> anyhow::Result<Option<crate::pattern::Pattern>> {
        let (find, row) = match &self.await_spec {
            None => return Ok(None),
            Some(AwaitSpec::Text(t)) => (t.as_str(), None),
            Some(AwaitSpec::Scoped(s)) => (s.find.as_str(), s.row),
        };
        if let Some(r) = row {
            let rows = rows as i32;
            if r >= rows || r < -rows {
                anyhow::bail!("await row {r} is outside the {rows}-row screen");
            }
        }
        Ok(Some(crate::pattern::Pattern::new(find, row)?))
    }
}

#[cfg(test)]
mod await_tests {
    use super::*;

    fn cfg(scene: &str) -> RecordConfig {
        let text = format!("launch = 'true'\ncols = 10\nrows = 4\n[[scene]]\n{scene}\n");
        toml::from_str(&text).unwrap()
    }

    #[test]
    fn timing_defaults_match_the_spec() {
        let c = cfg("keys = ['a']");
        assert_eq!(c.sample_ms, 10);
        assert_eq!(c.change_ms, 150);
        assert_eq!(c.stable_ms, 40);
        assert_eq!(c.persist_ms, 40);
        assert_eq!(c.wait_cap_ms, 3000);
        assert_eq!(c.await_ms, 5000);
        assert_eq!(c.max_capture_mb, 256);
        assert!(!c.realtime);
    }

    #[test]
    fn await_accepts_a_bare_string() {
        let c = cfg("keys = ['t']\nawait = 'theme: light'");
        let p = c.scenes[0].pattern(c.rows).unwrap().unwrap();
        assert_eq!(p.row(), None);
    }

    #[test]
    fn await_accepts_a_row_scoped_table() {
        let c = cfg("keys = ['t']\nawait = { find = 'theme: light', row = -1 }");
        let p = c.scenes[0].pattern(c.rows).unwrap().unwrap();
        assert_eq!(p.row(), Some(-1));
    }

    #[test]
    fn await_table_rejects_an_unknown_key() {
        let text = "launch = 'true'\ncols = 10\nrows = 4\n[[scene]]\nkeys = ['t']\n\
                    await = { find = 'x', row = -1, await_ms = 8000 }\n";
        let e: Result<RecordConfig, _> = toml::from_str(text);
        let err = e
            .err()
            .expect("a typo inside the await table must not be dropped");
        assert!(
            err.to_string().contains("await_ms"),
            "error should name the offending key: {err}"
        );
    }

    #[test]
    fn a_bad_regex_fails_at_load_not_at_runtime() {
        let c = cfg("keys = ['t']\nawait = 'unclosed('");
        assert!(c.scenes[0].pattern(c.rows).is_err());
    }

    #[test]
    fn a_row_outside_the_screen_is_rejected() {
        let c = cfg("keys = ['t']\nawait = { find = 'x', row = 9 }");
        let err = c.scenes[0].pattern(c.rows).unwrap_err().to_string();
        assert!(err.contains("row"), "error should name the row: {err}");
    }

    // --- an `await` that cannot be honoured is rejected at load ---

    #[test]
    fn await_on_an_animated_scene_is_rejected() {
        let c = cfg("keys = ['t']\nanimated = true\nawait = 'done'");
        let err = c.validate().unwrap_err().to_string();
        assert!(err.contains("scene 0"), "should name the scene: {err}");
        assert!(
            err.contains("animated"),
            "should name the combination: {err}"
        );
    }

    #[test]
    fn await_under_global_realtime_is_rejected() {
        let text = "launch = 'true'\ncols = 10\nrows = 4\nrealtime = true\n\
                    [[scene]]\nkeys = ['t']\nawait = 'done'\n";
        let c: RecordConfig = toml::from_str(text).unwrap();
        let err = c.validate().unwrap_err().to_string();
        assert!(err.contains("scene 0"), "should name the scene: {err}");
        assert!(
            err.contains("realtime"),
            "should name the combination: {err}"
        );
    }

    #[test]
    fn await_on_a_card_scene_is_rejected() {
        let c = cfg("card = { text = 'hi' }\nawait = 'done'");
        let err = c.validate().unwrap_err().to_string();
        assert!(err.contains("scene 0"), "should name the scene: {err}");
        assert!(err.contains("card"), "should name the combination: {err}");
    }

    /// The guard must not fire on the configurations it is not about: an
    /// `animated` or `realtime` scene with no `await` is perfectly legal, and
    /// so is an `await` on an ordinary scene.
    #[test]
    fn validate_accepts_the_legitimate_combinations() {
        cfg("keys = ['t']\nanimated = true").validate().unwrap();
        cfg("keys = ['t']\nawait = 'done'").validate().unwrap();
        cfg("card = { text = 'hi' }").validate().unwrap();
        let rt: RecordConfig = toml::from_str(
            "launch = 'true'\ncols = 10\nrows = 4\nrealtime = true\n[[scene]]\nkeys = ['t']\n",
        )
        .unwrap();
        rt.validate().unwrap();
    }

    #[test]
    fn animated_defaults_to_false() {
        let c = cfg("keys = ['a']");
        assert!(!c.scenes[0].animated);
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

    #[test]
    fn parse_encode_chrome() {
        let cfg: EncodeConfig = toml::from_str(
            r##"
            cols = 80
            rows = 24
            [chrome]
            style = "macos"
            title = "hello.sh"
            padding = 12
            [[frame]]
            file = "0.ansi"
            "##,
        )
        .unwrap();
        let ch = cfg.chrome.unwrap();
        assert_eq!(ch.style, ChromeStyle::Macos);
        assert_eq!(ch.title, "hello.sh");
        assert_eq!(ch.padding, 12);
        assert_eq!(ch.bar, "#2b2b2b"); // default
        assert_eq!(ch.text, "#d0d0d0"); // default
    }

    #[test]
    fn parse_record_chrome_linux() {
        let cfg: RecordConfig = toml::from_str(
            r##"
            launch = "x"
            cols = 1
            rows = 1
            [chrome]
            style = "linux"
            "##,
        )
        .unwrap();
        assert_eq!(cfg.chrome.unwrap().style, ChromeStyle::Linux);
    }

    #[test]
    fn chrome_absent_is_none_option() {
        let cfg: EncodeConfig =
            toml::from_str("cols = 1\nrows = 1\n[[frame]]\ncard = { text = \"x\" }").unwrap();
        assert!(cfg.chrome.is_none());
    }

    #[test]
    fn chrome_rejects_unknown_key() {
        let e: Result<EncodeConfig, _> = toml::from_str(
            r##"
            cols = 1
            rows = 1
            [chrome]
            style = "macos"
            bogus = 1
            [[frame]]
            card = { text = "x" }
            "##,
        );
        assert!(e.is_err());
    }
}
