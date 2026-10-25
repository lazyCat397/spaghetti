//! text: the glyphs of the 8x8 bitmap font, for basic Latin and Latin-1 (accents included).
//! a character outside those is drawn as `?`.

use font8x8::unicode::UnicodeFonts;
use font8x8::{BASIC_FONTS, LATIN_FONTS};

/// the size of a glyph cell, before the scale of the label
pub const CELL: u32 = 8;

/// the rows of a glyph: bit `col` of row `row` is set where the glyph has ink
pub fn glyph(c: char) -> [u8; 8] {
    BASIC_FONTS
        .get(c)
        .or_else(|| LATIN_FONTS.get(c))
        .or_else(|| BASIC_FONTS.get('?'))
        .unwrap_or([0; 8])
}

/// the lines of a text: a newline starts the next one
pub fn lines(text: &str) -> Vec<&str> {
    text.split('\n').collect()
}

/// the width and height of the text, in pixels at the given scale
pub fn size(text: &str, scale: u32) -> (f64, f64) {
    let cols = lines(text).iter().map(|l| l.chars().count()).max().unwrap_or(0) as u32;
    let rows = lines(text).len() as u32;
    ((cols * CELL * scale) as f64, (rows * CELL * scale) as f64)
}
