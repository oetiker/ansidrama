//! Assemble rendered frames into an animated WebP.

use anyhow::{anyhow, Result};
use image::RgbaImage;
use webp::{AnimEncoder, AnimFrame, WebPConfig};

/// One frame ready to encode: its pixels and how long to hold it, in centiseconds.
pub struct Frame {
    pub image: RgbaImage,
    pub hold_cs: u16,
}

/// Encode frames into a looping, lossless animated WebP. All frames must share
/// the dimensions of the first (the renderer guarantees this for a fixed
/// `cols × rows`).
pub fn encode_webp(frames: &[Frame]) -> Result<Vec<u8>> {
    let first = frames
        .first()
        .ok_or_else(|| anyhow!("no frames to encode"))?;
    let (w, h) = first.image.dimensions();

    let mut config = WebPConfig::new().map_err(|_| anyhow!("WebPConfig::new failed"))?;
    config.lossless = 1; // crisp text, no block artefacts
    let mut encoder = AnimEncoder::new(w, h, &config);
    encoder.set_loop_count(0); // loop forever

    // libwebp wants each frame's timestamp to be its cumulative START time (frame
    // 0 at 0); the final frame's duration comes from a terminal marker which the
    // `webp` crate hardcodes to t=0. So we append a trailing DUPLICATE of the last
    // frame at the total time: that gives the real last frame its full hold, and
    // the identical duplicate then collapses to ~0ms, invisibly.
    let mut t_ms: i32 = 0;
    let mut starts = Vec::with_capacity(frames.len());
    for f in frames {
        starts.push(t_ms);
        t_ms += f.hold_cs as i32 * 10;
    }
    for (f, &start) in frames.iter().zip(&starts) {
        encoder.add_frame(AnimFrame::from_rgba(f.image.as_raw(), w, h, start));
    }
    let last = &frames.last().unwrap().image;
    encoder.add_frame(AnimFrame::from_rgba(last.as_raw(), w, h, t_ms));

    Ok(encoder.encode().to_vec())
}

/// Total loop duration of a frame list, in milliseconds.
pub fn total_ms(frames: &[Frame]) -> i32 {
    frames.iter().map(|f| f.hold_cs as i32 * 10).sum()
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{Rgba, RgbaImage};

    #[test]
    fn preserves_alpha_through_lossless() {
        let mut img = RgbaImage::from_pixel(8, 8, Rgba([10, 20, 30, 255]));
        img.put_pixel(0, 0, Rgba([0, 0, 0, 0])); // transparent corner
        let frames = vec![
            Frame {
                image: img.clone(),
                hold_cs: 10,
            },
            Frame {
                image: img,
                hold_cs: 10,
            },
        ];
        let bytes = encode_webp(&frames).unwrap();

        let decoded = webp::AnimDecoder::new(&bytes)
            .decode()
            .expect("decode animated webp");
        let f0 = decoded.get_frame(0).expect("frame 0");
        let (w, h) = (f0.width() as usize, f0.height() as usize);
        let data = f0.get_image();
        assert_eq!(
            data.len(),
            w * h * 4,
            "decoded as RGBA (alpha channel present)"
        );
        assert_eq!(data[3], 0, "top-left pixel stayed transparent");
    }
}
