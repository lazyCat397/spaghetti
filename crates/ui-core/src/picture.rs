//! a picture: the pixels of a PNG file, decoded once. `win.image` decodes the file when the program
//! asks for it, never inside the frame loop, and the scene keeps the result.
//!
//! only PNG is read, with any color type and bit depth. everything comes out as 8-bit RGBA, so a
//! picture always has the same layout, and the transparency of the file is kept.

use png::{ColorType, Decoder, Limits, Transformations};

/// the largest side of a picture, in pixels. a picture of this side is 64 MiB of RGBA
pub const MAX_SIDE: usize = 4096;

/// the first eight bytes of every PNG file
const SIGNATURE: [u8; 8] = [0x89, b'P', b'N', b'G', b'\r', b'\n', 0x1a, b'\n'];

/// a decoded PNG: its size, and the red, green, blue and alpha of every pixel, row by row, top first
#[derive(Debug, Clone, PartialEq)]
pub struct Picture {
    path: String,
    width: usize,
    height: usize,
    rgba: Vec<u8>,
}

impl Picture {
    /// decodes the bytes of a PNG file. `path` is the name used in the error messages and kept
    /// with the picture. every error says what went wrong and names the file
    pub fn decode(path: &str, bytes: &[u8]) -> Result<Picture, String> {
        if !bytes.starts_with(&SIGNATURE) {
            return Err(format!("\"{path}\" is not a PNG file: only PNG images can be shown"));
        }
        let mut decoder = Decoder::new(bytes);
        // palettes and gray levels become RGB, 16 bits per channel become 8: the rest is handled below
        decoder.set_transformations(Transformations::normalize_to_color8());
        // the largest picture takes RGBA bytes for its own frame, and the decoder gets room for it
        // (its default limit is 64 MiB, one byte short of the largest picture)
        decoder.set_limits(Limits { bytes: 2 * MAX_SIDE * MAX_SIDE * 4 });
        let mut reader = decoder.read_info().map_err(|e| unusable(path, e))?;
        let (width, height) = {
            let info = reader.info();
            (info.width as usize, info.height as usize)
        };
        if width > MAX_SIDE || height > MAX_SIDE {
            return Err(format!("\"{path}\" is {width}x{height} pixels, but the largest side is {MAX_SIDE}"));
        }
        let mut buffer = vec![0; reader.output_buffer_size()];
        let frame = reader.next_frame(&mut buffer).map_err(|e| unusable(path, e))?;
        buffer.truncate(frame.buffer_size());
        let rgba = to_rgba(path, frame.color_type, &buffer)?;
        Ok(Picture { path: path.to_string(), width, height, rgba })
    }

    /// the path the picture was loaded from, as the program wrote it
    pub fn path(&self) -> &str {
        &self.path
    }

    pub fn width(&self) -> usize {
        self.width
    }

    pub fn height(&self) -> usize {
        self.height
    }

    /// the red, green, blue and alpha of the pixel at (`x`, `y`). the caller keeps them in range
    pub fn pixel(&self, x: usize, y: usize) -> [u8; 4] {
        let i = (y * self.width + x) * 4;
        [self.rgba[i], self.rgba[i + 1], self.rgba[i + 2], self.rgba[i + 3]]
    }
}

fn unusable(path: &str, reason: png::DecodingError) -> String {
    format!("\"{path}\" is not a usable PNG file: {reason}")
}

/// turns the decoded bytes into RGBA. after `normalize_to_color8` the decoder only hands out
/// 8-bit gray, gray with alpha, RGB or RGBA, so every other color type is a bug, not a file
fn to_rgba(path: &str, color: ColorType, bytes: &[u8]) -> Result<Vec<u8>, String> {
    let rgba = match color {
        ColorType::Rgba => bytes.to_vec(),
        ColorType::Rgb => bytes.chunks_exact(3).flat_map(|p| [p[0], p[1], p[2], 255]).collect(),
        ColorType::GrayscaleAlpha => bytes.chunks_exact(2).flat_map(|p| [p[0], p[0], p[0], p[1]]).collect(),
        ColorType::Grayscale => bytes.iter().flat_map(|&g| [g, g, g, 255]).collect(),
        ColorType::Indexed => return Err(format!("\"{path}\" could not be read: a palette was not expanded")),
    };
    Ok(rgba)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// encodes a PNG with the given color type, depth and rows of samples (the test writes the bytes)
    fn encode(width: u32, height: u32, color: ColorType, depth: png::BitDepth, data: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        {
            let mut encoder = png::Encoder::new(&mut out, width, height);
            encoder.set_color(color);
            encoder.set_depth(depth);
            let mut writer = encoder.write_header().unwrap();
            writer.write_image_data(data).unwrap();
        }
        out
    }

    #[test]
    fn rgba_keeps_every_channel() {
        let data = [255, 0, 0, 255, 0, 255, 0, 128, 0, 0, 255, 0, 9, 9, 9, 1];
        let bytes = encode(2, 2, ColorType::Rgba, png::BitDepth::Eight, &data);
        let p = Picture::decode("a.png", &bytes).unwrap();
        assert_eq!((p.width(), p.height(), p.path()), (2, 2, "a.png"));
        assert_eq!(p.pixel(0, 0), [255, 0, 0, 255]);
        assert_eq!(p.pixel(1, 0), [0, 255, 0, 128], "a half transparent pixel keeps its alpha");
        assert_eq!(p.pixel(0, 1), [0, 0, 255, 0], "a fully transparent pixel keeps alpha zero");
        assert_eq!(p.pixel(1, 1), [9, 9, 9, 1]);
    }

    #[test]
    fn rgb_gets_full_alpha() {
        let bytes = encode(1, 1, ColorType::Rgb, png::BitDepth::Eight, &[10, 20, 30]);
        assert_eq!(Picture::decode("a.png", &bytes).unwrap().pixel(0, 0), [10, 20, 30, 255]);
    }

    #[test]
    fn gray_is_copied_to_the_three_channels() {
        let bytes = encode(2, 1, ColorType::Grayscale, png::BitDepth::Eight, &[7, 200]);
        let p = Picture::decode("a.png", &bytes).unwrap();
        assert_eq!(p.pixel(0, 0), [7, 7, 7, 255]);
        assert_eq!(p.pixel(1, 0), [200, 200, 200, 255]);
    }

    #[test]
    fn gray_with_alpha_keeps_the_alpha() {
        let bytes = encode(1, 1, ColorType::GrayscaleAlpha, png::BitDepth::Eight, &[90, 60]);
        assert_eq!(Picture::decode("a.png", &bytes).unwrap().pixel(0, 0), [90, 90, 90, 60]);
    }

    #[test]
    fn sixteen_bits_per_channel_are_cut_to_eight() {
        // 0xFF00 is the high byte 255 and the low byte 0: it reads as 255
        let bytes = encode(1, 1, ColorType::Rgba, png::BitDepth::Sixteen, &[0xFF, 0x00, 0x80, 0x00, 0x00, 0x00, 0xFF, 0xFF]);
        assert_eq!(Picture::decode("a.png", &bytes).unwrap().pixel(0, 0), [255, 128, 0, 255]);
    }

    #[test]
    fn a_palette_with_transparency_is_expanded() {
        let mut out = Vec::new();
        {
            let mut encoder = png::Encoder::new(&mut out, 2, 1);
            encoder.set_color(ColorType::Indexed);
            encoder.set_depth(png::BitDepth::Eight);
            encoder.set_palette(vec![255, 0, 0, 0, 0, 255]);
            encoder.set_trns(vec![0]);
            let mut writer = encoder.write_header().unwrap();
            writer.write_image_data(&[0, 1]).unwrap();
        }
        let p = Picture::decode("a.png", &out).unwrap();
        assert_eq!(p.pixel(0, 0), [255, 0, 0, 0], "palette 0 is red and fully transparent");
        assert_eq!(p.pixel(1, 0), [0, 0, 255, 255], "palette 1 is blue and opaque");
    }

    #[test]
    fn a_file_that_is_not_png_is_refused_by_name() {
        let jpeg_start = [0xFF, 0xD8, 0xFF, 0xE0, 0, 0x10, b'J', b'F', b'I', b'F'];
        let err = Picture::decode("photo.jpg", &jpeg_start).unwrap_err();
        assert_eq!(err, "\"photo.jpg\" is not a PNG file: only PNG images can be shown");
        assert!(Picture::decode("empty.png", &[]).unwrap_err().contains("not a PNG file"));
    }

    #[test]
    fn a_truncated_png_is_refused_by_name() {
        let bytes = encode(4, 4, ColorType::Rgba, png::BitDepth::Eight, &[200; 64]);
        let err = Picture::decode("cut.png", &bytes[..bytes.len() / 2]).unwrap_err();
        assert!(err.starts_with("\"cut.png\" is not a usable PNG file: "), "{err}");
    }

    #[test]
    fn a_picture_wider_than_the_limit_is_refused_before_decoding() {
        // one row of pixels, one wider than the limit: only the header is read
        let width = MAX_SIDE as u32 + 1;
        let data = vec![0u8; width as usize * 4];
        let bytes = encode(width, 1, ColorType::Rgba, png::BitDepth::Eight, &data);
        let err = Picture::decode("wide.png", &bytes).unwrap_err();
        assert_eq!(err, format!("\"wide.png\" is {width}x1 pixels, but the largest side is {MAX_SIDE}"));
    }
}
