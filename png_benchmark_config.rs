//! Shared settings for both temporary PNG benchmark encoder crates.

/// DEFLATE compression level used by both Rust encoders (valid range: 1–9).
///
/// Chromium's current low-compression canvas PNG path uses level 1.
pub const PNG_DEFLATE_LEVEL: u8 = 1;

const _: () = assert!(PNG_DEFLATE_LEVEL >= 1 && PNG_DEFLATE_LEVEL <= 9);
