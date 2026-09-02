//! Temporary PNG encoding benchmark entry point.

use wasm_bindgen::prelude::*;

/// Encodes RGBA8 pixels with `png`'s default `flate2`/miniz_oxide backend.
///
/// Keep this configuration in sync with the companion zlib-rs crate so that
/// the benchmark changes only the DEFLATE backend.
#[wasm_bindgen]
pub fn encode_png_default(rgba: &[u8], width: u32, height: u32) -> Result<Vec<u8>, JsValue> {
    encode_rgba(rgba, width, height).map_err(|error| JsValue::from_str(&error))
}

fn encode_rgba(rgba: &[u8], width: u32, height: u32) -> Result<Vec<u8>, String> {
    let expected_len = width as usize * height as usize * 4;
    if rgba.len() != expected_len {
        return Err(format!(
            "expected {expected_len} RGBA bytes, received {}",
            rgba.len()
        ));
    }

    let mut bytes = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut bytes, width, height);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        encoder.set_compression(png::Compression::Balanced);
        encoder.set_filter(png::Filter::Adaptive);
        let mut writer = encoder.write_header().map_err(|error| error.to_string())?;
        writer
            .write_image_data(rgba)
            .map_err(|error| error.to_string())?;
        writer.finish().map_err(|error| error.to_string())?;
    }
    Ok(bytes)
}
