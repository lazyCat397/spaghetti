//! colors as `0x00RRGGBB`, ready for the framebuffer

/// `"#rrggbb"` -> color. only that form: a `#` and six hex digits
pub fn hex(text: &str) -> Result<u32, String> {
    let digits = text
        .strip_prefix('#')
        .ok_or_else(|| format!("color must start with '#', got \"{text}\""))?;
    if digits.len() != 6 || !digits.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(format!("color must be #rrggbb, got \"{text}\""));
    }
    u32::from_str_radix(digits, 16).map_err(|e| e.to_string())
}

/// three channels from 0 to 255 -> color
pub fn rgb(r: i64, g: i64, b: i64) -> Result<u32, String> {
    for (name, value) in [("red", r), ("green", g), ("blue", b)] {
        if !(0..=255).contains(&value) {
            return Err(format!("{name} must be between 0 and 255, got {value}"));
        }
    }
    Ok(((r as u32) << 16) | ((g as u32) << 8) | (b as u32))
}
