//! Turn a frame source (captured ANSI or a synthetic card) into rendered pixels.

use anyhow::{Context, Result};
use image::RgbaImage;

use crate::config::Card;
use crate::raster::Renderer;
use crate::{color, grid};

/// Rasterize captured ANSI (from `tmux capture-pane -e -p -N`) to a frame.
pub fn render_ansi(r: &Renderer, cols: u32, rows: u32, ansi: &str) -> RgbaImage {
    let g = grid::parse_grid(ansi);
    r.render(&g, cols, rows)
}

/// Rasterize a synthetic title card to a frame.
pub fn render_card(r: &Renderer, cols: u32, rows: u32, card: &Card) -> Result<RgbaImage> {
    let fg = color::parse(&card.fg)
        .map_err(anyhow::Error::msg)
        .context("card `fg`")?;
    let bg = color::parse(&card.bg)
        .map_err(anyhow::Error::msg)
        .context("card `bg`")?;
    let lines = card.resolved_lines();
    let g = grid::card(cols, rows, &lines, fg, bg, card.bold, card.border);
    Ok(r.render(&g, cols, rows))
}
