//! the retained scene of a window: the shapes it draws by itself on every frame.
//!
//! a shape lives from `Scene::add` to `Scene::kill` (or until the window closes). its id is
//! never reused, so a handle to a killed shape is always caught, never pointing at a newer shape.

use std::collections::HashMap;

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

/// the largest scale of a label: a glyph cell is 8 pixels, so 8 makes 64 pixels a character
pub const MAX_SCALE: u32 = 8;

/// a line of text. it is drawn like a shape, in the same order, and takes no clicks
#[derive(Debug, Clone, PartialEq)]
pub struct Label {
    x: f64,
    y: f64,
    text: String,
    font: crate::text::Font,
    color: u32,
    scale: u32,
    /// the width of the text before it wraps, in pixels. `0` means the text is not wrapped
    wrap: f64,
    align: Align,
    visible: bool,
    layer: i64,
}

impl Label {
    /// a visible label on layer 0. `scale` goes from 1 to `MAX_SCALE`
    pub fn new(x: f64, y: f64, text: &str, color: u32, scale: i64) -> Result<Label, String> {
        crate::text::parse(text)?;
        Ok(Label {
            x: finite("x", x)?,
            y: finite("y", y)?,
            text: text.to_string(),
            font: crate::text::Font::Bitmap,
            color,
            scale: scale_value(scale)?,
            wrap: 0.0,
            align: Align::Left,
            visible: true,
            layer: 0,
        })
    }

    /// the width and height of the text on screen, in pixels: the widest line and the number of
    /// lines, after wrapping
    pub fn size(&self) -> Result<(f64, f64), String> {
        crate::text::size(&self.text, &self.font, self.scale, self.wrap)
    }

    /// the font the text is drawn with
    pub fn font(&self) -> &crate::text::Font {
        &self.font
    }

    pub fn set_font(&mut self, value: crate::text::Font) {
        self.font = value;
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
    pub fn scale(&self) -> u32 {
        self.scale
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
    pub fn set_scale(&mut self, value: i64) -> Result<(), String> {
        self.scale = scale_value(value)?;
        Ok(())
    }
    pub fn set_visible(&mut self, value: bool) {
        self.visible = value;
    }
    pub fn set_layer(&mut self, value: i64) {
        self.layer = value;
    }
}

pub fn scale_value(value: i64) -> Result<u32, String> {
    if (1..=MAX_SCALE as i64).contains(&value) {
        Ok(value as u32)
    } else {
        Err(format!("the text scale must be between 1 and {MAX_SCALE}, got {value}"))
    }
}

/// where the lines of a label sit, inside the width of the label
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Align {
    Left,
    Center,
    Right,
}

/// something the scene draws: a rectangle or a label
pub enum Drawn<'a> {
    Shape(&'a Shape),
    Label(&'a Label),
}

#[derive(Debug)]
pub struct Scene {
    background: u32,
    shapes: HashMap<ShapeId, Shape>,
    labels: HashMap<ShapeId, Label>,
    next_id: ShapeId,
}

impl Default for Scene {
    fn default() -> Self {
        Scene { background: 0, shapes: HashMap::new(), labels: HashMap::new(), next_id: 1 }
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

    /// removes the shape or the label. one that is already gone is not an error: killing twice does nothing
    pub fn kill(&mut self, id: ShapeId) {
        self.shapes.remove(&id);
        self.labels.remove(&id);
    }

    /// every shape and label dies (the window closed)
    pub fn clear(&mut self) {
        self.shapes.clear();
        self.labels.clear();
    }

    pub fn len(&self) -> usize {
        self.shapes.len() + self.labels.len()
    }

    pub fn is_empty(&self) -> bool {
        self.shapes.is_empty() && self.labels.is_empty()
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

    /// everything the frame draws, back to front, shapes and labels in one order
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
        all.sort_unstable_by_key(|(layer, id, _)| (*layer, *id));
        all.into_iter().map(|(_, _, drawn)| drawn).collect()
    }
}

fn killed() -> String {
    "this shape was killed".to_string()
}
