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

#[derive(Debug)]
pub struct Scene {
    background: u32,
    shapes: HashMap<ShapeId, Shape>,
    next_id: ShapeId,
}

impl Default for Scene {
    fn default() -> Self {
        Scene { background: 0, shapes: HashMap::new(), next_id: 1 }
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

    pub fn get(&self, id: ShapeId) -> Result<&Shape, String> {
        self.shapes.get(&id).ok_or_else(killed)
    }

    pub fn get_mut(&mut self, id: ShapeId) -> Result<&mut Shape, String> {
        self.shapes.get_mut(&id).ok_or_else(killed)
    }

    /// removes the shape. a shape that is already gone is not an error: killing twice does nothing
    pub fn kill(&mut self, id: ShapeId) {
        self.shapes.remove(&id);
    }

    /// every shape dies (the window closed)
    pub fn clear(&mut self) {
        self.shapes.clear();
    }

    pub fn len(&self) -> usize {
        self.shapes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.shapes.is_empty()
    }

    /// the visible shapes, back to front: by layer, and by creation inside a layer
    pub fn draw_order(&self) -> Vec<&Shape> {
        let mut visible: Vec<(i64, ShapeId, &Shape)> =
            self.shapes.iter().filter(|(_, s)| s.visible).map(|(id, s)| (s.layer, *id, s)).collect();
        visible.sort_unstable_by_key(|(layer, id, _)| (*layer, *id));
        visible.into_iter().map(|(_, _, shape)| shape).collect()
    }
}

fn killed() -> String {
    "this shape was killed".to_string()
}
