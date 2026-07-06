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

/// Rasterize a synthetic title card to a frame at the terminal's pixel size, but
/// with the card's own fonts: the title line at `title_px`, subtitle lines at
/// `subtitle_px` (config `card_font_px` / `card_subtitle_px`, overridable per card).
pub fn render_card(
    r: &Renderer,
    cols: u32,
    rows: u32,
    card: &Card,
    title_px: f32,
    subtitle_px: f32,
) -> Result<RgbaImage> {
    let fg = color::parse(&card.fg)
        .map_err(anyhow::Error::msg)
        .context("card `fg`")?;
    let bg = color::parse(&card.bg)
        .map_err(anyhow::Error::msg)
        .context("card `bg`")?;
    let lines = card.resolved_lines();
    let (w, h) = r.frame_size(cols, rows);
    let tpx = card.font_px.unwrap_or(title_px);
    let spx = card.subtitle_px.unwrap_or(subtitle_px);
    Ok(r.render_card(w, h, &lines, fg, bg, card.bold, card.border, tpx, spx))
}
