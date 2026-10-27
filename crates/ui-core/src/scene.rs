//! the retained scene of a window: the shapes it draws by itself on every frame.
//!
//! a shape lives from `Scene::add` to `Scene::kill` (or until the window closes). its id is
//! never reused, so a handle to a killed shape is always caught, never pointing at a newer shape.

use std::collections::HashMap;
use std::sync::Arc;

use crate::picture::{Picture, MAX_SIDE};

pub type ShapeId = u64;

fn finite(name: &str, value: f64) -> Result<f64, String> {
    if value.is_finite() {
        Ok(value)
    } else {
        Err(format!("{name} must be a finite number"))
    }
}

fn size(name: &str, value: f64) -> Result<f64, String> {
    finite(name, value)?;
    if value < 0.0 {
        Err(format!("{name} can not be negative, got {value}"))
    } else {
        Ok(value)
    }
}

/// a filled rectangle. every setter checks its value, so a shape never holds a bad one
#[derive(Debug, Clone, PartialEq)]
pub struct Shape {
    x: f64,
    y: f64,
    w: f64,
    h: f64,
    color: u32,
    visible: bool,
    layer: i64,
    /// a click-through shape is never the target of a click: the click goes to what is under it
    /// (a bullet passing through targets)
    click_through: bool,
}

impl Shape {
    /// a visible rectangle on layer 0. `w` and `h` of zero are fine: nothing is drawn
    pub fn rect(x: f64, y: f64, w: f64, h: f64, color: u32) -> Result<Shape, String> {
        Ok(Shape {
            x: finite("x", x)?,
            y: finite("y", y)?,
            w: size("w", w)?,
            h: size("h", h)?,
            color,
            visible: true,
            layer: 0,
            click_through: false,
        })
    }

    pub fn x(&self) -> f64 {
        self.x
    }
    pub fn y(&self) -> f64 {
        self.y
    }
    pub fn w(&self) -> f64 {
        self.w
    }
    pub fn h(&self) -> f64 {
        self.h
    }
    pub fn color(&self) -> u32 {
        self.color
    }
    pub fn visible(&self) -> bool {
        self.visible
    }
    pub fn layer(&self) -> i64 {
        self.layer
    }
    pub fn click_through(&self) -> bool {
        self.click_through
    }
    pub fn set_click_through(&mut self, value: bool) {
        self.click_through = value;
    }

    pub fn set_x(&mut self, value: f64) -> Result<(), String> {
        self.x = finite("x", value)?;
        Ok(())
    }
    pub fn set_y(&mut self, value: f64) -> Result<(), String> {
        self.y = finite("y", value)?;
        Ok(())
    }
    pub fn set_w(&mut self, value: f64) -> Result<(), String> {
        self.w = size("w", value)?;
        Ok(())
    }
    pub fn set_h(&mut self, value: f64) -> Result<(), String> {
        self.h = size("h", value)?;
        Ok(())
    }
    pub fn set_color(&mut self, value: u32) {
        self.color = value;
    }
    pub fn set_visible(&mut self, value: bool) {
        self.visible = value;
    }
    pub fn set_layer(&mut self, value: i64) {
        self.layer = value;
    }
}

/// the largest size of a label: a glyph cell is 8 pixels, so 8 makes 64 pixels a character
/// the largest text size, in pixels: the em of an outline font, the height of a bitmap character
pub const MAX_TEXT_SIZE: u32 = 512;

/// a line of text. it is drawn like a shape, in the same order, and takes no clicks
#[derive(Debug, Clone, PartialEq)]
pub struct Label {
    x: f64,
    y: f64,
    text: String,
    font: crate::text::Font,
    spacing: crate::text::Spacing,
    /// the outline drawn around the text, in whole pixels. `0` means no outline
    stroke: f64,
    stroke_color: u32,
    /// the slant of the text in degrees: 0 is upright, positive leans right
    italic: f64,
    color: u32,
    size: u32,
    /// the width of the text before it wraps, in pixels. `0` means the text is not wrapped
    wrap: f64,
    align: Align,
    visible: bool,
    layer: i64,
}

impl Label {
    /// a visible label on layer 0. `size` goes from 1 to `MAX_TEXT_SIZE`
    pub fn new(x: f64, y: f64, text: &str, color: u32, size: i64) -> Result<Label, String> {
        crate::text::parse(text)?;
        Ok(Label {
            x: finite("x", x)?,
            y: finite("y", y)?,
            text: text.to_string(),
            font: crate::text::Font::Bitmap,
            spacing: crate::text::Spacing::default(),
            stroke: 0.0,
            stroke_color: 0x000000,
            italic: 0.0,
            color,
            size: text_size(size)?,
            wrap: 0.0,
            align: Align::Left,
            visible: true,
            layer: 0,
        })
    }

    /// the width and height of the text on screen, in pixels: the widest line and the number of
    /// lines, after wrapping
    pub fn size(&self) -> Result<(f64, f64), String> {
        crate::text::size(&self.text, &self.font, self.size, &self.spacing, self.wrap)
    }

    /// the font the text is drawn with
    pub fn font(&self) -> &crate::text::Font {
        &self.font
    }

    pub fn set_font(&mut self, value: crate::text::Font) {
        self.font = value;
    }

    /// the outline thickness in pixels, and its color
    pub fn stroke(&self) -> f64 {
        self.stroke
    }

    pub fn stroke_color(&self) -> u32 {
        self.stroke_color
    }

    pub fn set_stroke(&mut self, value: f64) -> Result<(), String> {
        let value = finite("stroke", value)?;
        if value < 0.0 {
            return Err(format!("the stroke must be 0 (no outline) or more pixels, got {value}"));
        }
        self.stroke = value;
        Ok(())
    }

    /// the slant in degrees, from -45 to 45. `0` is upright
    pub fn italic(&self) -> f64 {
        self.italic
    }

    pub fn set_italic(&mut self, value: f64) -> Result<(), String> {
        let value = finite("italic", value)?;
        if !(-45.0..=45.0).contains(&value) {
            return Err(format!("the italic slant must be between -45 and 45 degrees, got {value}"));
        }
        self.italic = value;
        Ok(())
    }

    pub fn set_stroke_color(&mut self, value: u32) {
        self.stroke_color = value;
    }

    /// the spacing between letters, words and lines
    pub fn spacing(&self) -> crate::text::Spacing {
        self.spacing
    }

    pub fn set_letter_spacing(&mut self, value: f64) -> Result<(), String> {
        self.spacing.letter = finite("letterSpacing", value)?;
        Ok(())
    }

    pub fn set_word_spacing(&mut self, value: f64) -> Result<(), String> {
        self.spacing.word = finite("wordSpacing", value)?;
        Ok(())
    }

    pub fn set_line_spacing(&mut self, value: f64) -> Result<(), String> {
        self.spacing.line = finite("lineSpacing", value)?;
        Ok(())
    }

    pub fn wrap(&self) -> f64 {
        self.wrap
    }

    pub fn align(&self) -> Align {
        self.align
    }

    pub fn set_wrap(&mut self, value: f64) -> Result<(), String> {
        if !value.is_finite() || value < 0.0 {
            return Err(format!("the wrap width must be 0 (no wrap) or more pixels, got {value}"));
        }
        self.wrap = value;
        Ok(())
    }

    pub fn set_align(&mut self, value: Align) {
        self.align = value;
    }

    pub fn x(&self) -> f64 {
        self.x
    }
    pub fn y(&self) -> f64 {
        self.y
    }
    pub fn text(&self) -> &str {
        &self.text
    }
    pub fn color(&self) -> u32 {
        self.color
    }
    pub fn text_size(&self) -> u32 {
        self.size
    }
    pub fn visible(&self) -> bool {
        self.visible
    }
    pub fn layer(&self) -> i64 {
        self.layer
    }

    pub fn set_x(&mut self, value: f64) -> Result<(), String> {
        self.x = finite("x", value)?;
        Ok(())
    }
    pub fn set_y(&mut self, value: f64) -> Result<(), String> {
        self.y = finite("y", value)?;
        Ok(())
    }
    pub fn set_text(&mut self, value: &str) -> Result<(), String> {
        crate::text::parse(value)?;
        self.text = value.to_string();
        Ok(())
    }
    pub fn set_color(&mut self, value: u32) {
        self.color = value;
    }
    pub fn set_text_size(&mut self, value: i64) -> Result<(), String> {
        self.size = text_size(value)?;
        Ok(())
    }
    pub fn set_visible(&mut self, value: bool) {
        self.visible = value;
    }
    pub fn set_layer(&mut self, value: i64) {
        self.layer = value;
    }
}

pub fn text_size(value: i64) -> Result<u32, String> {
    if (1..=MAX_TEXT_SIZE as i64).contains(&value) {
        Ok(value as u32)
    } else {
        Err(format!("the text size must be between 1 and {MAX_TEXT_SIZE} pixels, got {value}"))
    }
}

/// a picture placed in the scene: its top-left corner at (x, y), drawn `width` x `height` pixels.
/// the picture is sampled nearest-neighbor (no smoothing), so it stretches or shrinks blockily. its
/// transparent pixels show what is behind it, and the partly transparent ones mix with it
#[derive(Debug, Clone, PartialEq)]
pub struct Image {
    x: f64,
    y: f64,
    picture: Arc<Picture>,
    width: u32,
    height: u32,
    visible: bool,
    layer: i64,
}

impl Image {
    /// a visible image on layer 0. a width or height of 0 keeps the picture's own size on that side
    pub fn new(x: f64, y: f64, picture: Arc<Picture>, width: i64, height: i64) -> Result<Image, String> {
        let width = image_side("width", width, picture.width())?;
        let height = image_side("height", height, picture.height())?;
        Ok(Image {
            x: finite("x", x)?,
            y: finite("y", y)?,
            picture,
            width,
            height,
            visible: true,
            layer: 0,
        })
    }

    /// the picture the image shows
    pub fn picture(&self) -> &Picture {
        &self.picture
    }

    /// the size on screen, in pixels
    pub fn width(&self) -> u32 {
        self.width
    }
    pub fn height(&self) -> u32 {
        self.height
    }

    pub fn x(&self) -> f64 {
        self.x
    }
    pub fn y(&self) -> f64 {
        self.y
    }
    pub fn visible(&self) -> bool {
        self.visible
    }
    pub fn layer(&self) -> i64 {
        self.layer
    }

    pub fn set_x(&mut self, value: f64) -> Result<(), String> {
        self.x = finite("x", value)?;
        Ok(())
    }
    pub fn set_y(&mut self, value: f64) -> Result<(), String> {
        self.y = finite("y", value)?;
        Ok(())
    }
    pub fn set_width(&mut self, value: i64) -> Result<(), String> {
        self.width = image_side("width", value, self.picture.width())?;
        Ok(())
    }
    pub fn set_height(&mut self, value: i64) -> Result<(), String> {
        self.height = image_side("height", value, self.picture.height())?;
        Ok(())
    }
    pub fn set_visible(&mut self, value: bool) {
        self.visible = value;
    }
    pub fn set_layer(&mut self, value: i64) {
        self.layer = value;
    }
}

/// one side of an image in pixels: 0 keeps the picture's own side, anything else from 1 to `MAX_SIDE`
fn image_side(name: &str, value: i64, natural: usize) -> Result<u32, String> {
    if value == 0 {
        return Ok(natural as u32);
    }
    if (1..=MAX_SIDE as i64).contains(&value) {
        Ok(value as u32)
    } else {
        Err(format!("the image {name} must be between 0 and {MAX_SIDE} pixels, got {value}"))
    }
}

/// where the lines of a label sit, inside the width of the label
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Align {
    Left,
    Center,
    Right,
}

/// something the scene draws: a rectangle, a label or an image
pub enum Drawn<'a> {
    Shape(&'a Shape),
    Label(&'a Label),
    Image(&'a Image),
}

#[derive(Debug)]
pub struct Scene {
    background: u32,
    shapes: HashMap<ShapeId, Shape>,
    labels: HashMap<ShapeId, Label>,
    images: HashMap<ShapeId, Image>,
    next_id: ShapeId,
}

impl Default for Scene {
    fn default() -> Self {
        Scene {
            background: 0,
            shapes: HashMap::new(),
            labels: HashMap::new(),
            images: HashMap::new(),
            next_id: 1,
        }
    }
}

impl Scene {
    pub fn background(&self) -> u32 {
        self.background
    }

    pub fn set_background(&mut self, color: u32) {
        self.background = color;
    }

    pub fn add(&mut self, shape: Shape) -> ShapeId {
        let id = self.next_id;
        self.next_id += 1;
        self.shapes.insert(id, shape);
        id
    }

    pub fn add_label(&mut self, label: Label) -> ShapeId {
        let id = self.next_id;
        self.next_id += 1;
        self.labels.insert(id, label);
        id
    }

    pub fn add_image(&mut self, image: Image) -> ShapeId {
        let id = self.next_id;
        self.next_id += 1;
        self.images.insert(id, image);
        id
    }

    pub fn get(&self, id: ShapeId) -> Result<&Shape, String> {
        self.shapes.get(&id).ok_or_else(killed)
    }

    pub fn get_mut(&mut self, id: ShapeId) -> Result<&mut Shape, String> {
        self.shapes.get_mut(&id).ok_or_else(killed)
    }

    pub fn get_label(&self, id: ShapeId) -> Result<&Label, String> {
        self.labels.get(&id).ok_or_else(killed)
    }

    pub fn get_label_mut(&mut self, id: ShapeId) -> Result<&mut Label, String> {
        self.labels.get_mut(&id).ok_or_else(killed)
    }

    pub fn get_image(&self, id: ShapeId) -> Result<&Image, String> {
        self.images.get(&id).ok_or_else(killed_image)
    }

    pub fn get_image_mut(&mut self, id: ShapeId) -> Result<&mut Image, String> {
        self.images.get_mut(&id).ok_or_else(killed_image)
    }

    /// removes the shape, label or image. one that is already gone is not an error: killing twice does nothing
    pub fn kill(&mut self, id: ShapeId) {
        self.shapes.remove(&id);
        self.labels.remove(&id);
        self.images.remove(&id);
    }

    /// every shape and label dies (the window closed)
    pub fn clear(&mut self) {
        self.shapes.clear();
        self.labels.clear();
        self.images.clear();
    }

    pub fn len(&self) -> usize {
        self.shapes.len() + self.labels.len() + self.images.len()
    }

    pub fn is_empty(&self) -> bool {
        self.shapes.is_empty() && self.labels.is_empty() && self.images.is_empty()
    }

    /// the visible shapes, back to front: by layer, and by creation inside a layer
    pub fn draw_order(&self) -> Vec<&Shape> {
        self.draw_order_ids().into_iter().map(|(_, shape)| shape).collect()
    }

    /// the same, with the ids (the mouse needs them to tell which shape a click went to). labels
    /// take no clicks, so they are not here
    pub fn draw_order_ids(&self) -> Vec<(ShapeId, &Shape)> {
        let mut visible: Vec<(i64, ShapeId, &Shape)> =
            self.shapes.iter().filter(|(_, s)| s.visible).map(|(id, s)| (s.layer, *id, s)).collect();
        visible.sort_unstable_by_key(|(layer, id, _)| (*layer, *id));
        visible.into_iter().map(|(_, id, shape)| (id, shape)).collect()
    }

    /// everything the frame draws, back to front, shapes, labels and images in one order
    pub fn draw_list(&self) -> Vec<Drawn<'_>> {
        let mut all: Vec<(i64, ShapeId, Drawn<'_>)> = Vec::new();
        for (id, s) in &self.shapes {
            if s.visible {
                all.push((s.layer, *id, Drawn::Shape(s)));
            }
        }
        for (id, l) in &self.labels {
            if l.visible {
                all.push((l.layer, *id, Drawn::Label(l)));
            }
        }
        for (id, i) in &self.images {
            if i.visible {
                all.push((i.layer, *id, Drawn::Image(i)));
            }
        }
        all.sort_unstable_by_key(|(layer, id, _)| (*layer, *id));
        all.into_iter().map(|(_, _, drawn)| drawn).collect()
    }
}

fn killed() -> String {
    "this shape was killed".to_string()
}

fn killed_image() -> String {
    "this image was killed".to_string()
}
