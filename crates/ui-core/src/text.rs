//! text: the glyphs of a text, drawn with the 8x8 bitmap font (basic Latin and Latin-1, accents
//! included; a character outside those is drawn as `?`, and the font is monospaced), or with an
//! outline font loaded from a `.ttf` or `.otf` file, where each character has its own width.
//!
//! a text may color parts of itself with tags, always opened by `%{`:
//! - `%{#rrggbb}` starts a color: the characters after it are drawn in that color
//! - `%{/}` ends the color: the characters after it take the label's color again
//! - `%%{` is a literal `%{`
//! every other character, `%` and braces included, is drawn as written. the tags take no space on
//! screen. an unclosed tag or a tag that is not a color or `/` is an error

use std::sync::Arc;

use font8x8::unicode::UnicodeFonts;
use font8x8::{BASIC_FONTS, LATIN_FONTS};

use crate::color;

/// the size of a glyph cell, before the scale of the label
pub const CELL: u32 = 8;

/// for an outline font, a scale of 1 is this many pixels tall (the em size)
pub const OUTLINE_PIXELS: u32 = 16;

/// a font file loaded for labels: where it came from, and the parsed font
pub struct Outline {
    pub path: String,
    font: fontdue::Font,
}

impl Outline {
    fn px(scale: u32) -> f32 {
        (OUTLINE_PIXELS * scale) as f32
    }
}

impl std::fmt::Debug for Outline {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Outline({:?})", self.path)
    }
}

impl PartialEq for Outline {
    fn eq(&self, other: &Outline) -> bool {
        self.path == other.path
    }
}

/// the font a label draws with
#[derive(Debug, Clone, PartialEq)]
pub enum Font {
    /// the 8x8 bitmap font, the default
    Bitmap,
    /// a `.ttf` or `.otf` file
    Outline(Arc<Outline>),
}

impl Font {
    /// reads a font file. `path` is used as the name in error messages and kept for the label
    pub fn load(path: &str, bytes: &[u8]) -> Result<Font, String> {
        let font = fontdue::Font::from_bytes(bytes, fontdue::FontSettings::default())
            .map_err(|reason| format!("\"{path}\" is not a usable font file: {reason}"))?;
        Ok(Font::Outline(Arc::new(Outline { path: path.to_string(), font })))
    }

    /// the path the font was loaded from, or `""` for the bitmap font
    pub fn path(&self) -> &str {
        match self {
            Font::Bitmap => "",
            Font::Outline(o) => &o.path,
        }
    }
}

/// one character of a text as it is drawn: the character, and its color if a tag set one
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Styled {
    pub c: char,
    pub color: Option<u32>,
}

/// the rows of a bitmap glyph: bit `col` of row `row` is set where the glyph has ink
pub fn glyph(c: char) -> [u8; 8] {
    BASIC_FONTS
        .get(c)
        .or_else(|| LATIN_FONTS.get(c))
        .or_else(|| BASIC_FONTS.get('?'))
        .unwrap_or([0; 8])
}

/// the coverage of an outline glyph, one byte per pixel (255 is solid ink), with where it sits:
/// `left` pixels from the pen, and its top `top` pixels above the baseline's bottom edge
pub struct Coverage {
    pub left: f64,
    pub top: f64,
    pub width: usize,
    pub height: usize,
    pub pixels: Vec<u8>,
}

/// the coverage of `c` in `font` at `scale`. `None` for the bitmap font, which is drawn by `glyph`
pub fn coverage(font: &Font, c: char, scale: u32) -> Option<Coverage> {
    match font {
        Font::Bitmap => None,
        Font::Outline(o) => {
            let (m, pixels) = o.font.rasterize(c, Outline::px(scale));
            // fontdue's ymin is the bottom edge of the glyph above the baseline
            Some(Coverage {
                left: m.xmin as f64,
                top: -(m.ymin as f64) - m.height as f64,
                width: m.width,
                height: m.height,
                pixels,
            })
        }
    }
}

/// the baseline of a line, from its top: the distance down to where letters sit
pub fn baseline(font: &Font, scale: u32) -> f64 {
    match font {
        Font::Bitmap => (CELL * scale) as f64,
        Font::Outline(o) => o
            .font
            .horizontal_line_metrics(Outline::px(scale))
            .map(|m| m.ascent as f64)
            .unwrap_or(Outline::px(scale) as f64),
    }
}

/// the characters of a text with their colors, tags removed. a newline is kept as a character
pub fn parse(text: &str) -> Result<Vec<Styled>, String> {
    let chars: Vec<char> = text.chars().collect();
    let mut out = Vec::new();
    let mut current: Option<u32> = None;
    let mut i = 0;
    while i < chars.len() {
        if chars[i..].starts_with(&['%', '%', '{']) {
            // `%%{` is the escape for a literal `%{`
            out.push(Styled { c: '%', color: current });
            out.push(Styled { c: '{', color: current });
            i += 3;
        } else if chars[i..].starts_with(&['%', '{']) {
            let close = chars[i + 2..]
                .iter()
                .position(|&c| c == '}')
                .ok_or_else(|| {
                    let rest: String = chars[i + 2..].iter().collect();
                    format!("the text has a tag that is not closed: \"%{{{rest}\"")
                })?;
            let tag: String = chars[i + 2..i + 2 + close].iter().collect();
            if tag == "/" {
                current = None;
            } else if tag.starts_with('#') {
                current = Some(color::hex(&tag).map_err(|_| {
                    format!("the text color tag must be %{{#rrggbb}} or %{{/}}, got \"%{{{tag}}}\"")
                })?);
            } else {
                return Err(format!("the text has an unknown tag \"%{{{tag}}}\": use %{{#rrggbb}} or %{{/}}"));
            }
            i += 2 + close + 1;
        } else {
            out.push(Styled { c: chars[i], color: current });
            i += 1;
        }
    }
    Ok(out)
}

/// how far the pen moves after a character, in pixels at `scale`. the bitmap font is monospaced,
/// so every character moves the same distance
pub fn advance(font: &Font, c: char, scale: u32) -> f64 {
    match font {
        Font::Bitmap => (CELL * scale) as f64,
        Font::Outline(o) => o.font.metrics(c, Outline::px(scale)).advance_width as f64,
    }
}

/// the height of one line of text, in pixels at `scale`, without any extra spacing
pub fn line_height(font: &Font, scale: u32) -> f64 {
    match font {
        Font::Bitmap => (CELL * scale) as f64,
        Font::Outline(o) => o
            .font
            .horizontal_line_metrics(Outline::px(scale))
            .map(|m| m.new_line_size as f64)
            .unwrap_or(Outline::px(scale) as f64 * 1.2),
    }
}

/// the extra room a text adds between its characters, words and lines, in pixels. all zero by
/// default; negative values squeeze the text
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Spacing {
    /// added after every character but the last one of a line
    pub letter: f64,
    /// added after every space
    pub word: f64,
    /// added between one line and the next
    pub line: f64,
}

/// the distance from one line of text to the next, in pixels at `scale`, with the line spacing
pub fn line_pitch(font: &Font, scale: u32, spacing: &Spacing) -> f64 {
    line_height(font, scale) + spacing.line
}

/// the width one character takes on its own, with the word spacing if it is a space
fn glyph_width(font: &Font, c: char, scale: u32, spacing: &Spacing) -> f64 {
    advance(font, c, scale) + if c == ' ' { spacing.word } else { 0.0 }
}

/// the width of a run of characters, in pixels at `scale`: the letter spacing goes between
/// characters, never after the last one
pub fn line_width(line: &[Styled], font: &Font, scale: u32, spacing: &Spacing) -> f64 {
    if line.is_empty() {
        return 0.0;
    }
    let glyphs: f64 = line.iter().map(|s| glyph_width(font, s.c, scale, spacing)).sum();
    glyphs + spacing.letter * (line.len() - 1) as f64
}

/// the width of `a` followed by `b`, given both widths: the letter spacing goes between them
fn join(a_width: f64, a_len: usize, b_width: f64, b_len: usize, spacing: &Spacing) -> f64 {
    match (a_len, b_len) {
        (0, _) => b_width,
        (_, 0) => a_width,
        _ => a_width + spacing.letter + b_width,
    }
}

/// how many characters from the start of `chars` fit in `wrap` pixels. at least one, so a
/// character wider than the line still gets a line of its own
fn fit_prefix(chars: &[Styled], font: &Font, scale: u32, spacing: &Spacing, wrap: f64) -> usize {
    let mut width = 0.0;
    let mut count = 0;
    for s in chars {
        let next = join(width, count, glyph_width(font, s.c, scale, spacing), 1, spacing);
        if next > wrap && count > 0 {
            break;
        }
        width = next;
        count += 1;
    }
    count.max(1)
}

/// the lines a text takes on screen: each written line broken into words that fit in `wrap`
/// pixels (a word wider than a line is cut). with `wrap` at `0` the lines are as written
pub fn layout(chars: &[Styled], font: &Font, scale: u32, spacing: &Spacing, wrap: f64) -> Vec<Vec<Styled>> {
    // the written lines, split at each newline
    let written: Vec<&[Styled]> = chars.split(|s| s.c == '\n').collect();
    if wrap <= 0.0 {
        return written.into_iter().map(<[Styled]>::to_vec).collect();
    }
    let space = Styled { c: ' ', color: None };
    let space_width = glyph_width(font, ' ', scale, spacing);
    let mut out: Vec<Vec<Styled>> = Vec::new();
    for line in written {
        let mut current: Vec<Styled> = Vec::new();
        let mut current_width = 0.0;
        for word in line.split(|s| s.c == ' ') {
            let mut rest = word;
            loop {
                let rest_width = line_width(rest, font, scale, spacing);
                // the line so far, then a space if the line is not empty, then the rest of the word
                let (with_space_width, with_space_len) = if current.is_empty() {
                    (0.0, 0)
                } else {
                    (join(current_width, current.len(), space_width, 1, spacing), current.len() + 1)
                };
                let candidate = join(with_space_width, with_space_len, rest_width, rest.len(), spacing);
                if candidate <= wrap {
                    if !current.is_empty() {
                        current.push(space);
                    }
                    current.extend_from_slice(rest);
                    current_width = candidate;
                    break;
                }
                if current.is_empty() {
                    // the rest does not fit a line of its own: cut off the part that does
                    let n = fit_prefix(rest, font, scale, spacing, wrap);
                    if n == rest.len() {
                        // the last piece stays open: the next word may still join it
                        current = rest.to_vec();
                        current_width = rest_width;
                        break;
                    }
                    out.push(rest[..n].to_vec());
                    rest = &rest[n..];
                } else {
                    out.push(std::mem::take(&mut current));
                    current_width = 0.0;
                }
            }
        }
        out.push(current);
    }
    out
}

/// the width and height of the text on screen, in pixels at `scale`: the widest line, and the
/// lines after wrapping with their spacing between them. an invalid text is an error
pub fn size(text: &str, font: &Font, scale: u32, spacing: &Spacing, wrap: f64) -> Result<(f64, f64), String> {
    let lines = layout(&parse(text)?, font, scale, spacing, wrap);
    let width = lines.iter().map(|l| line_width(l, font, scale, spacing)).fold(0.0, f64::max);
    let count = lines.len().max(1) as f64;
    Ok((width, count * line_height(font, scale) + (count - 1.0) * spacing.line))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plain(lines: &[Vec<Styled>]) -> Vec<String> {
        lines.iter().map(|l| l.iter().map(|s| s.c).collect()).collect()
    }

    #[test]
    fn tags_take_no_space_and_color_until_reset() {
        let s = parse("a%{#ff0000}bc%{/}d").unwrap();
        let text: String = s.iter().map(|x| x.c).collect();
        assert_eq!(text, "abcd");
        assert_eq!(s[0].color, None);
        assert_eq!(s[1].color, Some(0xff0000));
        assert_eq!(s[2].color, Some(0xff0000));
        assert_eq!(s[3].color, None);
    }

    #[test]
    fn braces_and_lone_percent_are_plain_text() {
        let s = parse("{x} 50% }").unwrap();
        let text: String = s.iter().map(|x| x.c).collect();
        assert_eq!(text, "{x} 50% }");
        let s = parse("100%% and 50%").unwrap();
        let text: String = s.iter().map(|x| x.c).collect();
        assert_eq!(text, "100%% and 50%");
        let s = parse("%%{x}").unwrap();
        let text: String = s.iter().map(|x| x.c).collect();
        assert_eq!(text, "%{x}");
    }

    #[test]
    fn bad_tags_are_errors() {
        assert!(parse("%{#zzz}").is_err());
        assert!(parse("%{bold}").is_err());
        assert!(parse("%{#ff0000").is_err());
    }

    #[test]
    fn wrapping_counts_only_visible_characters() {
        // at scale 2 a line of 32 pixels holds two characters; the tag does not count
        let s = parse("%{#ff0000}ab%{/} cd").unwrap();
        assert_eq!(plain(&layout(&s, &Font::Bitmap, 2, &Spacing::default(), 32.0)), vec!["ab", "cd"]);
        assert_eq!(size("%{#ff0000}ab%{/} cd", &Font::Bitmap, 2, &Spacing::default(), 32.0).unwrap(), (32.0, 32.0));
    }

    #[test]
    fn spacing_moves_letters_words_and_lines() {
        let plain = Spacing::default();
        let chars = parse("ab cd").unwrap();
        // letter spacing goes between characters only: 5 characters of 8 px, 4 gaps of 2 px
        let tight = Spacing { letter: 2.0, ..plain };
        assert_eq!(line_width(&chars, &Font::Bitmap, 1, &tight), 40.0 + 8.0);
        // word spacing goes after the space
        let words = Spacing { word: 4.0, ..plain };
        assert_eq!(line_width(&chars, &Font::Bitmap, 1, &words), 40.0 + 4.0);
        // line spacing adds between lines, not after the last one
        let lines = Spacing { line: 3.0, ..plain };
        assert_eq!(size("a\nb", &Font::Bitmap, 1, &lines, 0.0).unwrap(), (8.0, 8.0 + 8.0 + 3.0));
    }

    #[test]
    fn outline_fonts_load_and_are_proportional() {
        assert!(Font::load("broken.ttf", b"not a font").is_err());
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../examples/fonts/LiberationSans-Regular.ttf");
        let bytes = std::fs::read(path).unwrap();
        let font = Font::load("LiberationSans-Regular.ttf", &bytes).unwrap();
        assert_eq!(font.path(), "LiberationSans-Regular.ttf");
        assert!(advance(&font, 'i', 2) < advance(&font, 'm', 2), "an i is narrower than an m");
        assert!(coverage(&font, 'A', 2).unwrap().pixels.iter().any(|&p| p > 0));
        assert!(coverage(&Font::Bitmap, 'A', 2).is_none());
    }
}
