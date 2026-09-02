//! Companion WASM module for the zlib-rs half of the PNG benchmark.
//!
//! This must remain a separate Cargo build: `png`'s `zlib-rs` feature changes
//! the `flate2` backend for the entire dependency graph.

#![cfg(target_arch = "wasm32")]

#[path = "../../png_benchmark_config.rs"]
mod config;

use wasm_bindgen::prelude::*;

/// Encodes RGB8 or RGBA8 pixels using `png` with its zlib-rs feature enabled.
#[wasm_bindgen]
pub fn encode_png_zlib_rs(
    pixels: &[u8],
    width: u32,
    height: u32,
    has_alpha: bool,
) -> Result<Vec<u8>, JsValue> {
    let channels = if has_alpha { 4 } else { 3 };
    let pixel_format = if has_alpha { "RGBA" } else { "RGB" };
    let expected_len = width as usize * height as usize * channels;
    if pixels.len() != expected_len {
        return Err(JsValue::from_str(&format!(
            "expected {expected_len} {pixel_format} bytes, received {}",
            pixels.len()
        )));
    }

    let mut bytes = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut bytes, width, height);
        encoder.set_color(if has_alpha {
            png::ColorType::Rgba
        } else {
            png::ColorType::Rgb
        });
        encoder.set_depth(png::BitDepth::Eight);
        encoder.set_deflate_compression(png::DeflateCompression::Level(config::PNG_DEFLATE_LEVEL));
        encoder.set_filter(png::Filter::Up);
        let mut writer = encoder
            .write_header()
            .map_err(|error| JsValue::from_str(&error.to_string()))?;
        writer
            .write_image_data(pixels)
            .map_err(|error| JsValue::from_str(&error.to_string()))?;
        writer
            .finish()
            .map_err(|error| JsValue::from_str(&error.to_string()))?;
    }
    Ok(bytes)
}
