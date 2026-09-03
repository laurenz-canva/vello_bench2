//! Shared settings for the temporary PNG benchmark encoder crates.

/// Chromium's current low-compression canvas PNG path uses level 1 with miniz.
pub const PNG_MINIZ_DEFLATE_LEVEL: u8 = 1;

/// Level 2 avoids zlib-rs's fixed-Huffman level-1 quick path.
pub const PNG_ZLIB_RS_DEFLATE_LEVEL: u8 = 2;

const _: () = assert!(PNG_MINIZ_DEFLATE_LEVEL >= 1 && PNG_MINIZ_DEFLATE_LEVEL <= 9);
const _: () = assert!(PNG_ZLIB_RS_DEFLATE_LEVEL >= 1 && PNG_ZLIB_RS_DEFLATE_LEVEL <= 9);
