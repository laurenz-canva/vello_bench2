//! Shared settings for the temporary PNG benchmark encoder crates.

/// Chromium's current low-compression canvas PNG path uses level 1 with miniz.
pub const PNG_DEFLATE_LEVEL_1: u8 = 1;

/// Level 2 avoids zlib-rs's fixed-Huffman level-1 quick path.
pub const PNG_DEFLATE_LEVEL_2: u8 = 2;

const _: () = assert!(PNG_DEFLATE_LEVEL_1 >= 1 && PNG_DEFLATE_LEVEL_1 <= 9);
const _: () = assert!(PNG_DEFLATE_LEVEL_2 >= 1 && PNG_DEFLATE_LEVEL_2 <= 9);
