//! Temporary PNG encoding benchmark entry point.

#[path = "../png_benchmark_config.rs"]
mod config;

use wasm_bindgen::prelude::*;

/// Encodes RGB8 or RGBA8 pixels with `png`'s default `flate2`/miniz_oxide backend.
///
/// Uses Chromium's level-1 low-compression setting as the miniz baseline.
#[wasm_bindgen]
pub fn encode_png_default(
    pixels: &[u8],
    width: u32,
    height: u32,
    has_alpha: bool,
    compression_variant: u8,
) -> Result<Vec<u8>, JsValue> {
    encode_pixels(pixels, width, height, has_alpha, compression_variant)
        .map_err(|error| JsValue::from_str(&error))
}

fn encode_pixels(
    pixels: &[u8],
    width: u32,
    height: u32,
    has_alpha: bool,
    compression_variant: u8,
) -> Result<Vec<u8>, String> {
    let channels = if has_alpha { 4 } else { 3 };
    let pixel_format = if has_alpha { "RGBA" } else { "RGB" };
    let expected_len = width as usize * height as usize * channels;
    if pixels.len() != expected_len {
        return Err(format!(
            "expected {expected_len} {pixel_format} bytes, received {}",
            pixels.len()
        ));
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
        configure_compression(&mut encoder, compression_variant)?;
        let mut writer = encoder.write_header().map_err(|error| error.to_string())?;
        writer
            .write_image_data(pixels)
            .map_err(|error| error.to_string())?;
        writer.finish().map_err(|error| error.to_string())?;
    }
    Ok(bytes)
}

fn configure_compression<W: std::io::Write>(
    encoder: &mut png::Encoder<'_, W>,
    compression_variant: u8,
) -> Result<(), String> {
    match compression_variant {
        0 => {
            encoder.set_compression(png::Compression::Balanced);
            encoder.set_filter(png::Filter::Adaptive);
        }
        1 => {
            encoder.set_deflate_compression(png::DeflateCompression::Level(
                config::PNG_DEFLATE_LEVEL_1,
            ));
            encoder.set_filter(png::Filter::Up);
        }
        2 => {
            encoder.set_deflate_compression(png::DeflateCompression::Level(
                config::PNG_DEFLATE_LEVEL_2,
            ));
            encoder.set_filter(png::Filter::Up);
        }
        _ => {
            return Err(format!(
                "unknown PNG compression variant: {compression_variant}"
            ));
        }
    }
    Ok(())
}
