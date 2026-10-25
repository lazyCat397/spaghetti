//! text: the glyphs of the 8x8 bitmap font, for basic Latin and Latin-1 (accents included).
//! a character outside those is drawn as `?`. the font is monospaced: every character is the same
//! width, so wrapping counts characters.

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

/// the lines of a text as written: a newline starts the next one
pub fn lines(text: &str) -> Vec<&str> {
    text.split('\n').collect()
}

/// how many characters fit on a line of `wrap` pixels at `scale`. `0` means no wrap
fn columns(wrap: f64, scale: u32) -> Option<usize> {
    if wrap <= 0.0 {
        None
    } else {
        let cell = (CELL * scale) as f64;
        Some(((wrap / cell).floor() as usize).max(1))
    }
}

/// the lines a text takes on screen: each written line broken into words that fit in `wrap`
/// pixels (a word longer than a line is cut). with `wrap` at `0` the lines are as written
pub fn layout(text: &str, scale: u32, wrap: f64) -> Vec<String> {
    let Some(cols) = columns(wrap, scale) else {
        return lines(text).into_iter().map(str::to_string).collect();
    };
    let mut out = Vec::new();
    for written in lines(text) {
        let mut current = String::new();
        for word in written.split(' ') {
            let mut word_chars: Vec<char> = word.chars().collect();
            // a word that does not fit on a line of its own is cut into pieces
            while word_chars.len() > cols {
                if !current.is_empty() {
                    out.push(std::mem::take(&mut current));
                }
                out.push(word_chars.drain(..cols).collect());
            }
            let word: String = word_chars.into_iter().collect();
            let fits = current.is_empty() || current.chars().count() + 1 + word.chars().count() <= cols;
            if fits {
                if !current.is_empty() {
                    current.push(' ');
                }
                current.push_str(&word);
            } else {
                out.push(std::mem::take(&mut current));
                current = word;
            }
        }
        out.push(current);
    }
    out
}

/// the width and height of the text on screen, in pixels at `scale`: the widest line, and the
/// number of lines after wrapping
pub fn size(text: &str, scale: u32, wrap: f64) -> (f64, f64) {
    let lines = layout(text, scale, wrap);
    let cols = lines.iter().map(|l| l.chars().count()).max().unwrap_or(0) as u32;
    ((cols * CELL * scale) as f64, (lines.len() as u32 * CELL * scale) as f64)
}
