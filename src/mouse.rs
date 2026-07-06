//! Friendly mouse primitives → SGR (1006) mouse-report byte sequences.
//!
//! A TUI with mouse tracking on reads `ESC [ < b ; col ; row M` (press/motion)
//! and `… m` (release) from its input; those bytes are what we feed the app via
//! tmux `send-keys -l`. The config never spells them out — it says `click`,
//! `drag`, `scroll`, and this module emits the wire form. Coordinates are
//! 1-based, matching the terminal.

use serde::Deserialize;

/// SGR button code for a named mouse button.
fn button_code(b: Button) -> u32 {
    match b {
        Button::Left => 0,
        Button::Middle => 1,
        Button::Right => 2,
    }
}

#[derive(Clone, Copy, Deserialize, Default, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum Button {
    #[default]
    Left,
    Middle,
    Right,
}

fn press(b: u32, x: u32, y: u32) -> String {
    format!("\x1b[<{b};{x};{y}M")
}
fn release(b: u32, x: u32, y: u32) -> String {
    format!("\x1b[<{b};{x};{y}m")
}
/// Motion while a button is held sets the SGR "drag" bit (32).
fn motion(b: u32, x: u32, y: u32) -> String {
    format!("\x1b[<{};{x};{y}M", b + 32)
}

/// A single click: press then release at one spot.
#[derive(Deserialize)]
pub struct Click {
    pub x: u32,
    pub y: u32,
    #[serde(default)]
    pub button: Button,
}

impl Click {
    pub fn sequences(&self) -> Vec<String> {
        let b = button_code(self.button);
        vec![press(b, self.x, self.y), release(b, self.x, self.y)]
    }
}

/// A press-drag-release across a straight path, `steps` interpolated moves.
#[derive(Deserialize)]
pub struct Drag {
    pub from: [u32; 2],
    pub to: [u32; 2],
    #[serde(default = "default_steps")]
    pub steps: u32,
    #[serde(default)]
    pub button: Button,
}

fn default_steps() -> u32 {
    4
}

impl Drag {
    pub fn sequences(&self) -> Vec<String> {
        let b = button_code(self.button);
        let (x0, y0) = (self.from[0], self.from[1]);
        let (x1, y1) = (self.to[0], self.to[1]);
        let mut seq = vec![press(b, x0, y0)];
        let steps = self.steps.max(1);
        for i in 1..=steps {
            // Linear interpolation, rounded to whole cells.
            let x = x0 as i64 + (x1 as i64 - x0 as i64) * i as i64 / steps as i64;
            let y = y0 as i64 + (y1 as i64 - y0 as i64) * i as i64 / steps as i64;
            seq.push(motion(b, x as u32, y as u32));
        }
        seq.push(release(b, x1, y1));
        seq
    }
}

/// A wheel scroll at a point, `n` clicks in a direction.
#[derive(Deserialize)]
pub struct Scroll {
    pub x: u32,
    pub y: u32,
    pub dir: ScrollDir,
    #[serde(default = "default_scroll_n")]
    pub n: u32,
}

fn default_scroll_n() -> u32 {
    3
}

#[derive(Clone, Copy, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum ScrollDir {
    Up,
    Down,
}

impl Scroll {
    pub fn sequences(&self) -> Vec<String> {
        // SGR wheel: up = 64, down = 65 (press-style 'M').
        let code = match self.dir {
            ScrollDir::Up => 64,
            ScrollDir::Down => 65,
        };
        (0..self.n.max(1))
            .map(|_| format!("\x1b[<{code};{};{}M", self.x, self.y))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn click_press_release() {
        let c = Click {
            x: 10,
            y: 11,
            button: Button::Left,
        };
        assert_eq!(c.sequences(), vec!["\x1b[<0;10;11M", "\x1b[<0;10;11m"]);
    }

    #[test]
    fn right_click_button_code() {
        let c = Click {
            x: 3,
            y: 4,
            button: Button::Right,
        };
        assert_eq!(c.sequences()[0], "\x1b[<2;3;4M");
    }

    #[test]
    fn drag_interpolates_with_motion_bit() {
        let d = Drag {
            from: [10, 10],
            to: [14, 10],
            steps: 2,
            button: Button::Left,
        };
        // press at start, 2 motions (drag bit 32), release at end.
        assert_eq!(
            d.sequences(),
            vec![
                "\x1b[<0;10;10M",
                "\x1b[<32;12;10M",
                "\x1b[<32;14;10M",
                "\x1b[<0;14;10m"
            ]
        );
    }

    #[test]
    fn scroll_down_emits_wheel() {
        let s = Scroll {
            x: 50,
            y: 8,
            dir: ScrollDir::Down,
            n: 2,
        };
        assert_eq!(s.sequences(), vec!["\x1b[<65;50;8M", "\x1b[<65;50;8M"]);
    }
}
