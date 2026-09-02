//! Companion WASM module for the zlib-rs half of the PNG benchmark.
//!
//! This must remain a separate Cargo build: `png`'s `zlib-rs` feature changes
//! the `flate2` backend for the entire dependency graph.

#![cfg(target_arch = "wasm32")]

use wasm_bindgen::prelude::*;

/// Encodes RGBA8 pixels using `png` with its zlib-rs feature enabled.
#[wasm_bindgen]
pub fn encode_png_zlib_rs(rgba: &[u8], width: u32, height: u32) -> Result<Vec<u8>, JsValue> {
    let expected_len = width as usize * height as usize * 4;
    if rgba.len() != expected_len {
        return Err(JsValue::from_str(&format!(
            "expected {expected_len} RGBA bytes, received {}",
            rgba.len()
        )));
    }

    let mut bytes = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut bytes, width, height);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        encoder.set_compression(png::Compression::Balanced);
        encoder.set_filter(png::Filter::Adaptive);
        let mut writer = encoder
            .write_header()
            .map_err(|error| JsValue::from_str(&error.to_string()))?;
        writer
            .write_image_data(rgba)
            .map_err(|error| JsValue::from_str(&error.to_string()))?;
        writer
            .finish()
            .map_err(|error| JsValue::from_str(&error.to_string()))?;
    }
    Ok(bytes)
}
