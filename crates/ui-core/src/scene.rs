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

/// what a shape is drawn as inside its box: a rectangle fills the box, an ellipse fills the oval
/// inscribed in it (a circle when the box is square)
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Kind {
    Rect,
    Ellipse,
    /// a line with round ends: a band of the height's thickness, whose ends are half-discs
    Capsule,
    /// a closed shape through the vertices of `Shape::vertices`, filled by the even-odd rule
    Polygon,
}

/// a checked corner radius: finite and not negative
fn corner_value(name: &str, value: f64) -> Result<f64, String> {
    let value = finite(name, value)?;
    if value < 0.0 {
        return Err(format!("the {name} must not be negative, got {value}"));
    }
    Ok(value)
}

/// whether `p` is inside the polygon `vertices` (even-odd rule: a ray from `p` crosses it an odd number of times)
fn inside_polygon(vertices: &[(f64, f64)], (px, py): (f64, f64)) -> bool {
    let mut inside = false;
    let n = vertices.len();
    let mut j = n - 1;
    for i in 0..n {
        let (xi, yi) = vertices[i];
        let (xj, yj) = vertices[j];
        if (yi > py) != (yj > py) && px < (xj - xi) * (py - yi) / (yj - yi) + xi {
            inside = !inside;
        }
        j = i;
    }
    inside
}

/// the squared distance from `p` to the closest edge of the polygon
fn edge_distance_squared(vertices: &[(f64, f64)], p: (f64, f64)) -> f64 {
    let n = vertices.len();
    let mut best = f64::MAX;
    for i in 0..n {
        let (ax, ay) = vertices[i];
        let (bx, by) = vertices[(i + 1) % n];
        let (dx, dy) = (bx - ax, by - ay);
        let length2 = dx * dx + dy * dy;
        let t = if length2 == 0.0 { 0.0 } else { (((p.0 - ax) * dx + (p.1 - ay) * dy) / length2).clamp(0.0, 1.0) };
        let (cx, cy) = (ax + t * dx, ay + t * dy);
        best = best.min((p.0 - cx).powi(2) + (p.1 - cy).powi(2));
    }
    best
}

/// a filled rectangle or ellipse. every setter checks its value, so a shape never holds a bad one
#[derive(Debug, Clone, PartialEq)]
pub struct Shape {
    kind: Kind,
    /// the corners of a polygon, measured from the top-left corner of its box. empty for the others
    vertices: Vec<(f64, f64)>,
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
    /// the thickness of the outline in pixels, drawn inside the edges (0 for none)
    stroke: f64,
    /// the color of the outline, black until the program sets another
    stroke_color: u32,
    /// the turn of the shape in degrees around its pivot, clockwise on screen. `0` is no turn
    rotation: f64,
    /// the pivot as a fraction of the width and of the height: `0.5, 0.5` is the centre
    pivot_x: f64,
    pivot_y: f64,
    /// how solid the shape is, from `0` (invisible) to `1` (solid)
    alpha: f64,
    /// the corner radii of a rectangle, in pixels, clockwise from the top-left: top-left, top-right,
    /// bottom-right, bottom-left. `0` is a square corner. an ellipse or a line ignores them
    radii: [f64; 4],
    /// whether the inside is painted. without fill and without stroke the shape draws nothing
    fill: bool,
}

impl Shape {
    /// a visible rectangle on layer 0. `w` and `h` of zero are fine: nothing is drawn
    pub fn rect(x: f64, y: f64, w: f64, h: f64, color: u32) -> Result<Shape, String> {
        Shape::of(Kind::Rect, x, y, w, h, color)
    }

    /// a visible ellipse inside the box `x, y, w, h`, on layer 0. a square box makes a circle
    pub fn ellipse(x: f64, y: f64, w: f64, h: f64, color: u32) -> Result<Shape, String> {
        Shape::of(Kind::Ellipse, x, y, w, h, color)
    }

    /// a visible line of `thickness` pixels from `(x1, y1)` to `(x2, y2)`, with round ends, on layer 0.
    /// it is a capsule: its width is the length plus the thickness, its height is the thickness, and
    /// it is turned to point from the first end to the second. a zero-length line is a dot
    pub fn line(x1: f64, y1: f64, x2: f64, y2: f64, thickness: f64, color: u32) -> Result<Shape, String> {
        let (x1, y1) = (finite("x1", x1)?, finite("y1", y1)?);
        let (x2, y2) = (finite("x2", x2)?, finite("y2", y2)?);
        let thickness = size("thickness", thickness)?;
        let (dx, dy) = (x2 - x1, y2 - y1);
        let length = (dx * dx + dy * dy).sqrt();
        let (w, h) = (length + thickness, thickness);
        let (cx, cy) = ((x1 + x2) / 2.0, (y1 + y2) / 2.0);
        let mut shape = Shape::of(Kind::Capsule, cx - w / 2.0, cy - h / 2.0, w, h, color)?;
        shape.rotation = dy.atan2(dx).to_degrees();
        Ok(shape)
    }

    /// a visible polygon on layer 0. `points` is a flat list of x, y pairs, at least three of them. the
    /// box of the shape is the smallest one that holds the points, and the points are measured from it
    pub fn polygon(points: &[f64], color: u32) -> Result<Shape, String> {
        if points.len() < 6 || points.len() % 2 != 0 {
            return Err(format!("a polygon needs at least 3 points, as x, y pairs, got {} numbers", points.len()));
        }
        let mut corners = Vec::with_capacity(points.len() / 2);
        for pair in points.chunks(2) {
            corners.push((finite("x", pair[0])?, finite("y", pair[1])?));
        }
        let left = corners.iter().map(|c| c.0).fold(f64::MAX, f64::min);
        let top = corners.iter().map(|c| c.1).fold(f64::MAX, f64::min);
        let right = corners.iter().map(|c| c.0).fold(f64::MIN, f64::max);
        let bottom = corners.iter().map(|c| c.1).fold(f64::MIN, f64::max);
        let mut shape = Shape::of(Kind::Polygon, left, top, right - left, bottom - top, color)?;
        shape.vertices = corners.into_iter().map(|(x, y)| (x - left, y - top)).collect();
        Ok(shape)
    }

    fn of(kind: Kind, x: f64, y: f64, w: f64, h: f64, color: u32) -> Result<Shape, String> {
        Ok(Shape {
            kind,
            vertices: Vec::new(),
            x: finite("x", x)?,
            y: finite("y", y)?,
            w: size("w", w)?,
            h: size("h", h)?,
            color,
            visible: true,
            layer: 0,
            click_through: false,
            stroke: 0.0,
            stroke_color: 0x000000,
            rotation: 0.0,
            pivot_x: 0.5,
            pivot_y: 0.5,
            alpha: 1.0,
            radii: [0.0; 4],
            fill: true,
        })
    }

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

    pub fn set_stroke_color(&mut self, value: u32) {
        self.stroke_color = value;
    }

    pub fn rotation(&self) -> f64 {
        self.rotation
    }

    pub fn set_rotation(&mut self, value: f64) -> Result<(), String> {
        self.rotation = finite("rotation", value)?;
        Ok(())
    }

    pub fn pivot_x(&self) -> f64 {
        self.pivot_x
    }

    pub fn set_pivot_x(&mut self, value: f64) -> Result<(), String> {
        self.pivot_x = finite("pivotX", value)?;
        Ok(())
    }

    pub fn pivot_y(&self) -> f64 {
        self.pivot_y
    }

    pub fn set_pivot_y(&mut self, value: f64) -> Result<(), String> {
        self.pivot_y = finite("pivotY", value)?;
        Ok(())
    }

    pub fn alpha(&self) -> f64 {
        self.alpha
    }

    pub fn set_alpha(&mut self, value: f64) -> Result<(), String> {
        let value = finite("alpha", value)?;
        if !(0.0..=1.0).contains(&value) {
            return Err(format!("the alpha must be between 0 and 1, got {value}"));
        }
        self.alpha = value;
        Ok(())
    }

    /// the top-left corner radius as set (the others are in `top_right_radius` and so on)
    pub fn radius(&self) -> f64 {
        self.radii[0]
    }

    /// sets the radius of all four corners at once
    pub fn set_radius(&mut self, value: f64) -> Result<(), String> {
        let value = corner_value("radius", value)?;
        self.radii = [value; 4];
        Ok(())
    }

    pub fn top_left_radius(&self) -> f64 {
        self.radii[0]
    }

    pub fn set_top_left_radius(&mut self, value: f64) -> Result<(), String> {
        self.radii[0] = corner_value("topLeftRadius", value)?;
        Ok(())
    }

    pub fn top_right_radius(&self) -> f64 {
        self.radii[1]
    }

    pub fn set_top_right_radius(&mut self, value: f64) -> Result<(), String> {
        self.radii[1] = corner_value("topRightRadius", value)?;
        Ok(())
    }

    pub fn bottom_right_radius(&self) -> f64 {
        self.radii[2]
    }

    pub fn set_bottom_right_radius(&mut self, value: f64) -> Result<(), String> {
        self.radii[2] = corner_value("bottomRightRadius", value)?;
        Ok(())
    }

    pub fn bottom_left_radius(&self) -> f64 {
        self.radii[3]
    }

    pub fn set_bottom_left_radius(&mut self, value: f64) -> Result<(), String> {
        self.radii[3] = corner_value("bottomLeftRadius", value)?;
        Ok(())
    }

    pub fn fill(&self) -> bool {
        self.fill
    }

    pub fn set_fill(&mut self, value: bool) {
        self.fill = value;
    }

    /// true when the shape is drawn the plain way: not turned, solid and filled. the plain way is
    /// the fast path of the window; every other shape is tested pixel by pixel
    pub fn is_plain(&self) -> bool {
        self.kind == Kind::Rect && !self.has_corners() && self.rotation == 0.0 && self.alpha == 1.0 && self.fill
    }

    fn has_corners(&self) -> bool {
        self.radii.iter().any(|r| *r > 0.0)
    }

    /// the corner radii in use, clockwise from the top-left. when the two corners of one side do not
    /// fit in it (their radii add up to more than the side), all four are scaled down by the same factor
    fn corner_radii(&self) -> [f64; 4] {
        let [tl, tr, br, bl] = self.radii.map(|r| r.max(0.0));
        let (w, h) = (self.w, self.h);
        let mut factor: f64 = 1.0;
        for (sum, side) in [(tl + tr, w), (bl + br, w), (tl + bl, h), (tr + br, h)] {
            if sum > side {
                factor = factor.min(side / sum);
            }
        }
        [tl * factor, tr * factor, br * factor, bl * factor]
    }

    pub fn kind(&self) -> Kind {
        self.kind
    }

    /// whether the point `local` (measured in the unturned box, from its top-left corner) is inside
    /// the shape pulled in by `inset` on every side. `0` is the whole shape; the outline is what lies
    /// inside the whole shape but outside the shape pulled in by the stroke
    pub fn covers(&self, local: (f64, f64), inset: f64) -> bool {
        let (lx, ly) = local;
        match self.kind {
            Kind::Rect => {
                if !(lx >= inset && lx < self.w - inset && ly >= inset && ly < self.h - inset) {
                    return false;
                }
                // a corner's circle decides the points in its square; the shape pulled in by `inset`
                // has corners of radius `r - inset`, and a sharp corner when that is not positive
                let (w, h) = (self.w, self.h);
                let [tl, tr, br, bl] = self.corner_radii();
                let corners = [
                    (tl, lx < tl && ly < tl, (tl, tl)),
                    (tr, lx > w - tr && ly < tr, (w - tr, tr)),
                    (br, lx > w - br && ly > h - br, (w - br, h - br)),
                    (bl, lx < bl && ly > h - bl, (bl, h - bl)),
                ];
                for (r, in_square, (cx, cy)) in corners {
                    if in_square {
                        let rr = r - inset;
                        if rr <= 0.0 {
                            return true;
                        }
                        let (dx, dy) = (lx - cx, ly - cy);
                        return dx * dx + dy * dy < rr * rr;
                    }
                }
                true
            }
            Kind::Polygon => {
                if !inside_polygon(&self.vertices, local) {
                    return false;
                }
                // the shape pulled in by `inset` keeps the points at least `inset` away from every edge
                inset <= 0.0 || edge_distance_squared(&self.vertices, local) >= inset * inset
            }
            Kind::Capsule => {
                // the middle line runs between the centres of the two end discs, whose radius is half
                // the thickness; `inset` only shrinks the radius, so the outline lies inside the band
                let radius = self.h / 2.0;
                let cx = lx.clamp(radius, (self.w - radius).max(radius));
                let r = radius - inset;
                if r <= 0.0 {
                    return false;
                }
                let (dx, dy) = (lx - cx, ly - self.h / 2.0);
                dx * dx + dy * dy < r * r
            }
            Kind::Ellipse => {
                let (rx, ry) = (self.w / 2.0 - inset, self.h / 2.0 - inset);
                if rx <= 0.0 || ry <= 0.0 {
                    return false;
                }
                let (dx, dy) = ((lx - self.w / 2.0) / rx, (ly - self.h / 2.0) / ry);
                dx * dx + dy * dy < 1.0
            }
        }
    }

    /// the sine and cosine of the turn, computed once per shape rather than once per pixel
    pub fn turn(&self) -> (f64, f64) {
        self.rotation.to_radians().sin_cos()
    }

    /// the point of the window `(px, py)` seen in the shape's own unturned box, measured from its
    /// top-left corner. `turn` is the result of `Shape::turn`. the point is inside the box when
    /// both coordinates are at least 0 and below the width and the height
    pub fn to_local(&self, turn: (f64, f64), px: f64, py: f64) -> (f64, f64) {
        let (sin, cos) = turn;
        let (pivot_x, pivot_y) = (self.pivot_x * self.w, self.pivot_y * self.h);
        let (dx, dy) = (px - self.x - pivot_x, py - self.y - pivot_y);
        // undo the turn: a point turned by `rotation` clockwise comes back by the opposite turn
        let ux = dx * cos + dy * sin;
        let uy = -dx * sin + dy * cos;
        (pivot_x + ux, pivot_y + uy)
    }

    /// whether the window point `(px, py)` is inside the shape, turned with it. the click test
    /// and the drawing both use this, so a turned shape is hit where it is seen
    pub fn contains(&self, px: f64, py: f64) -> bool {
        if self.kind == Kind::Rect && !self.has_corners() && self.rotation == 0.0 {
            return px >= self.x && px < self.x + self.w && py >= self.y && py < self.y + self.h;
        }
        self.covers(self.to_local(self.turn(), px, py), 0.0)
    }

    /// the box that holds the shape in the window, turned: left, top, right and bottom in pixels
    pub fn bounds(&self) -> (f64, f64, f64, f64) {
        let (sin, cos) = self.turn();
        let (pivot_x, pivot_y) = (self.pivot_x * self.w, self.pivot_y * self.h);
        let (mut left, mut top, mut right, mut bottom) = (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
        for (cx, cy) in [(0.0, 0.0), (self.w, 0.0), (0.0, self.h), (self.w, self.h)] {
            let (dx, dy) = (cx - pivot_x, cy - pivot_y);
            let x = self.x + pivot_x + dx * cos - dy * sin;
            let y = self.y + pivot_y + dx * sin + dy * cos;
            left = left.min(x);
            top = top.min(y);
            right = right.max(x);
            bottom = bottom.max(y);
        }
        (left, top, right, bottom)
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
    /// whether the turned and see-through shapes are drawn smooth (their edges blended over
    /// several samples per pixel) or stepped. the plain shapes are always pixel-exact
    antialias: bool,
    shapes: HashMap<ShapeId, Shape>,
    labels: HashMap<ShapeId, Label>,
    images: HashMap<ShapeId, Image>,
    next_id: ShapeId,
}

impl Default for Scene {
    fn default() -> Self {
        Scene {
            background: 0,
            antialias: false,
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

    pub fn antialias(&self) -> bool {
        self.antialias
    }

    pub fn set_antialias(&mut self, value: bool) {
        self.antialias = value;
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
