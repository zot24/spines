use image::{DynamicImage, ImageFormat, Rgba, RgbaImage};
use std::io::Cursor;

use crate::error::Result;

/// A tall jewel-case spine. Art fills the face. A dark edge stands in for the plastic.
pub fn spine_png(art: Option<&[u8]>, fallback: [u8; 3]) -> Result<Vec<u8>> {
    let face = match art.and_then(|bytes| image::load_from_memory(bytes).ok()) {
        Some(img) => img,
        None => {
            let mut solid = RgbaImage::new(64, 280);
            for px in solid.pixels_mut() {
                *px = Rgba([fallback[0], fallback[1], fallback[2], 255]);
            }
            DynamicImage::ImageRgba8(solid)
        }
    };
    let face = face.resize_to_fill(64, 280, image::imageops::FilterType::Triangle);
    let mut canvas = RgbaImage::new(72, 292);
    for px in canvas.pixels_mut() {
        *px = Rgba([18, 16, 14, 255]);
    }
    image::imageops::overlay(&mut canvas, &face.to_rgba8(), 4, 6);
    for y in 6..286 {
        canvas.put_pixel(4, y, Rgba([255, 255, 255, 70]));
    }
    let mut out = Vec::new();
    DynamicImage::ImageRgba8(canvas).write_to(&mut Cursor::new(&mut out), ImageFormat::Png)?;
    Ok(out)
}

pub fn jpeg_bytes(art: &[u8]) -> Option<Vec<u8>> {
    let img = image::load_from_memory(art).ok()?;
    let mut out = Vec::new();
    img.write_to(&mut Cursor::new(&mut out), ImageFormat::Jpeg)
        .ok()?;
    Some(out)
}
