//! a window while it runs: the scene, the framebuffer, the keys and the frame time.
//! it knows neither the window library nor spg: it gets the backend and the time from outside.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use crate::backend::{Backend, Button, Key};
use crate::picture::Picture;
use crate::scene::{Align, Drawn, Image, Label, Scene, Shape, ShapeId};
use crate::text;

/// the largest `dt` the program gets, in seconds. stops jumps after a pause or a freeze
pub const MAX_DT: f64 = 0.1;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    /// open, but no frame has started yet
    Created,
    /// inside the loop
    Running,
    /// closed: the shapes are gone and nothing can be used any more
    Closed,
}

pub struct Session<B: Backend> {
    backend: B,
    width: usize,
    height: usize,
    pixels: Vec<u32>,
    scene: Scene,
    state: State,
    last_time: Option<f64>,
    close_requested: bool,
    /// the keys read at the start of the current frame. a snapshot: the whole frame sees
    /// the same keys, even if the user presses one in the middle of it
    down: HashSet<Key>,
    pressed: HashSet<Key>,
    released: HashSet<Key>,
    /// the focus as the last frame saw it (`None` before the first frame: that is not a change)
    focused: Option<bool>,
    /// the focus changes of the frames so far, not yet taken: `true` gained, `false` lost
    focus_changes: Vec<bool>,
    /// the size as the last frame saw it (`None` before the first frame)
    size: Option<(usize, usize)>,
    /// the new sizes of the frames so far, not yet taken
    resizes: Vec<(usize, usize)>,
    /// the mouse, as the last frame saw it, in framebuffer pixels
    mouse_inside: bool,
    mouse_x: f64,
    mouse_y: f64,
    /// the buttons held down by the last frame, and the ones that went down or up at it
    buttons_down: HashSet<Button>,
    buttons_pressed: HashSet<Button>,
    buttons_released: HashSet<Button>,
    /// whether a frame has read the mouse yet (the first frame reports no press or release)
    mouse_read: bool,
    /// the characters typed during the last frame
    typed: String,
    /// the topmost shape under the pointer that takes clicks, as of the last frame
    hovered: Option<ShapeId>,
    /// the shape that took each button when it went down: it keeps the button until it goes up
    captured: HashMap<Button, ShapeId>,
    /// per shape and button, as of the last frame
    shape_down: HashSet<(ShapeId, Button)>,
    shape_pressed: HashSet<(ShapeId, Button)>,
    shape_released: HashSet<(ShapeId, Button)>,
}

fn closed() -> String {
    "the window is closed".to_string()
}

impl<B: Backend> Session<B> {
    /// opens the window. both sides must be greater than zero
    pub fn open(mut backend: B, title: &str, width: usize, height: usize) -> Result<Self, String> {
        if width == 0 || height == 0 {
            return Err(format!("window size must be positive, got {width}x{height}"));
        }
        backend.open(title, width, height)?;
        Ok(Session {
            backend,
            width,
            height,
            pixels: vec![0; width * height],
            scene: Scene::default(),
            state: State::Created,
            last_time: None,
            close_requested: false,
            down: HashSet::new(),
            pressed: HashSet::new(),
            released: HashSet::new(),
            focused: None,
            focus_changes: Vec::new(),
            size: None,
            resizes: Vec::new(),
            mouse_inside: false,
            mouse_x: -1.0,
            mouse_y: -1.0,
            buttons_down: HashSet::new(),
            buttons_pressed: HashSet::new(),
            buttons_released: HashSet::new(),
            mouse_read: false,
            typed: String::new(),
            hovered: None,
            captured: HashMap::new(),
            shape_down: HashSet::new(),
            shape_pressed: HashSet::new(),
            shape_released: HashSet::new(),
        })
    }

    pub fn state(&self) -> State {
        self.state
    }

    pub fn width(&self) -> usize {
        self.width
    }

    pub fn height(&self) -> usize {
        self.height
    }

    /// starts a frame. `now` is the current time in seconds (monotonic).
    ///
    /// draws the scene, shows it, reads the keys and returns the frame's `dt`. the first call
    /// shows the scene as the program built it before the loop.
    /// returns `None` when the window must close: the user closed it, Esc was pressed, or
    /// `request_close` was called during the previous frame. the state then becomes `Closed`
    /// and the shapes die.
    pub fn begin_frame(&mut self, now: f64) -> Result<Option<f64>, String> {
        if self.state == State::Closed {
            return Ok(None);
        }
        self.render();
        self.backend.show(&self.pixels, self.width, self.height)?;
        let still_open = self.backend.poll();
        self.read_keys();
        self.read_mouse();
        self.typed = self.backend.take_typed();
        let focused = self.backend.is_focused();
        if let Some(before) = self.focused {
            if before != focused {
                self.focus_changes.push(focused);
            }
        }
        self.focused = Some(focused);
        let size = self.backend.size();
        if let Some(before) = self.size {
            if before != size {
                self.resizes.push(size);
            }
        }
        self.size = Some(size);

        let escape = self.pressed.contains(&Key::Escape) || self.down.contains(&Key::Escape);
        if !still_open || escape || self.close_requested {
            self.close_now();
            return Ok(None);
        }

        let dt = match self.last_time {
            None => 0.0,
            Some(last) => (now - last).clamp(0.0, MAX_DT),
        };
        self.last_time = Some(now);
        self.state = State::Running;
        Ok(Some(dt))
    }

    /// asks the window to close. the current frame finishes, and the window closes at the next
    /// `begin_frame`. asking a window that is already closed does nothing
    pub fn request_close(&mut self) {
        if self.state != State::Closed {
            self.close_requested = true;
        }
    }

    // --- the scene ---------------------------------------------------------------

    /// the scene, while the window is open
    pub fn scene(&self) -> Result<&Scene, String> {
        match self.state {
            State::Closed => Err(closed()),
            _ => Ok(&self.scene),
        }
    }

    pub fn scene_mut(&mut self) -> Result<&mut Scene, String> {
        match self.state {
            State::Closed => Err(closed()),
            _ => Ok(&mut self.scene),
        }
    }

    /// a new visible rectangle on layer 0, drawn from the next frame on
    pub fn add_rect(&mut self, x: f64, y: f64, w: f64, h: f64, color: u32) -> Result<ShapeId, String> {
        let shape = Shape::rect(x, y, w, h, color)?;
        Ok(self.scene_mut()?.add(shape))
    }

    /// a new visible line of `thickness` pixels between two points, on layer 0, drawn from the next frame on
    pub fn add_line(&mut self, x1: f64, y1: f64, x2: f64, y2: f64, thickness: f64, color: u32) -> Result<ShapeId, String> {
        let shape = Shape::line(x1, y1, x2, y2, thickness, color)?;
        Ok(self.scene_mut()?.add(shape))
    }

    /// a new visible polygon through the points `points` (x, y pairs), on layer 0, drawn from the next frame on
    pub fn add_polygon(&mut self, points: &[f64], color: u32) -> Result<ShapeId, String> {
        let shape = Shape::polygon(points, color)?;
        Ok(self.scene_mut()?.add(shape))
    }

    /// a new visible ellipse in the box `x, y, w, h`, on layer 0, drawn from the next frame on
    pub fn add_ellipse(&mut self, x: f64, y: f64, w: f64, h: f64, color: u32) -> Result<ShapeId, String> {
        let shape = Shape::ellipse(x, y, w, h, color)?;
        Ok(self.scene_mut()?.add(shape))
    }

    pub fn shape(&self, id: ShapeId) -> Result<&Shape, String> {
        self.scene()?.get(id)
    }

    pub fn shape_mut(&mut self, id: ShapeId) -> Result<&mut Shape, String> {
        self.scene_mut()?.get_mut(id)
    }

    /// a new line of text. `size` is in pixels, from 1 to 512: the em of an outline font, or the height of a bitmap character
    pub fn add_label(&mut self, x: f64, y: f64, text: &str, color: u32, size: i64) -> Result<ShapeId, String> {
        let label = Label::new(x, y, text, color, size)?;
        Ok(self.scene_mut()?.add_label(label))
    }

    pub fn label(&self, id: ShapeId) -> Result<&Label, String> {
        self.scene()?.get_label(id)
    }

    pub fn label_mut(&mut self, id: ShapeId) -> Result<&mut Label, String> {
        self.scene_mut()?.get_label_mut(id)
    }

    /// a new image on layer 0, drawn from the next frame on. `width` and `height` are pixels (0 keeps the picture's own)
    pub fn add_image(&mut self, x: f64, y: f64, picture: Arc<Picture>, width: i64, height: i64) -> Result<ShapeId, String> {
        let image = Image::new(x, y, picture, width, height)?;
        Ok(self.scene_mut()?.add_image(image))
    }

    pub fn image(&self, id: ShapeId) -> Result<&Image, String> {
        self.scene()?.get_image(id)
    }

    pub fn image_mut(&mut self, id: ShapeId) -> Result<&mut Image, String> {
        self.scene_mut()?.get_image_mut(id)
    }

    /// removes a shape. a shape that is already gone, or that went with its window, is fine:
    /// killing twice does nothing
    pub fn kill_shape(&mut self, id: ShapeId) {
        if self.state != State::Closed {
            self.scene.kill(id);
        }
    }

    // --- keys -----------------------------------------------------------------------

    /// the focus changes since the last call, in order: `true` when the window gained the focus,
    /// `false` when it lost it
    pub fn take_focus_changes(&mut self) -> Vec<bool> {
        std::mem::take(&mut self.focus_changes)
    }

    /// the mouse x of the last frame, in framebuffer pixels. `-1` before the pointer was over the window
    pub fn mouse_x(&self) -> f64 {
        self.mouse_x
    }

    /// the mouse y of the last frame, in framebuffer pixels
    pub fn mouse_y(&self) -> f64 {
        self.mouse_y
    }

    /// the characters typed during the last frame, in order (control keys are not characters)
    pub fn typed(&self) -> &str {
        &self.typed
    }

    /// whether the pointer is over the window, as of the last frame
    pub fn mouse_inside(&self) -> bool {
        self.mouse_inside
    }

    /// whether the mouse button is held down, as of the last frame
    pub fn button_down(&self, button: Button) -> bool {
        self.buttons_down.contains(&button)
    }

    /// whether the button went down since the previous frame: true on one frame only
    pub fn button_pressed(&self, button: Button) -> bool {
        self.buttons_pressed.contains(&button)
    }

    /// whether the button went up since the previous frame
    pub fn button_released(&self, button: Button) -> bool {
        self.buttons_released.contains(&button)
    }

    /// the shape a click would reach, as of the last frame
    pub fn hovered(&self) -> Option<ShapeId> {
        self.hovered
    }

    /// whether the pointer is over this shape, and the shape is the one a click would reach
    pub fn shape_hovered(&self, id: ShapeId) -> bool {
        self.hovered == Some(id)
    }

    /// whether the shape holds the button down (it took the button when it went down)
    pub fn shape_button_down(&self, id: ShapeId, button: Button) -> bool {
        self.shape_down.contains(&(id, button))
    }

    /// whether the shape took the button this frame: the press that went to it
    pub fn shape_button_pressed(&self, id: ShapeId, button: Button) -> bool {
        self.shape_pressed.contains(&(id, button))
    }

    /// whether the shape gave the button back this frame
    pub fn shape_button_released(&self, id: ShapeId, button: Button) -> bool {
        self.shape_released.contains(&(id, button))
    }

    /// the new sizes of the window since the last call, in order (in pixels). the framebuffer
    /// stays the size it was opened with
    pub fn take_resizes(&mut self) -> Vec<(usize, usize)> {
        std::mem::take(&mut self.resizes)
    }

    /// whether the key is held down, as of the start of this frame
    pub fn is_down(&self, key: Key) -> bool {
        self.down.contains(&key)
    }

    /// whether the key went down since the previous frame: true on one frame only, even when
    /// the key is held for many
    pub fn was_pressed(&self, key: Key) -> bool {
        self.pressed.contains(&key)
    }

    /// whether the key went up since the previous frame
    pub fn was_released(&self, key: Key) -> bool {
        self.released.contains(&key)
    }

    // --- the framebuffer -------------------------------------------------------------

    /// a pixel of the last frame drawn, for tests and debugging
    pub fn pixel(&self, x: usize, y: usize) -> Option<u32> {
        if x < self.width && y < self.height {
            self.pixels.get(y * self.width + x).copied()
        } else {
            None
        }
    }

    pub fn backend(&self) -> &B {
        &self.backend
    }

    pub fn backend_mut(&mut self) -> &mut B {
        &mut self.backend
    }

    /// the background, then every visible shape and label back to front. the parts outside the
    /// window are cut off
    fn render(&mut self) {
        let Session { scene, pixels, width, height, .. } = self;
        pixels.fill(scene.background());
        for drawn in scene.draw_list() {
            match drawn {
                Drawn::Shape(shape) => {
                    if shape.is_plain() {
                        fill_box(pixels, *width, *height, shape.x(), shape.y(), shape.w(), shape.h(), shape.color());
                        let stroke = shape.stroke().round();
                        if stroke > 0.0 {
                            draw_outline(pixels, *width, *height, shape, stroke);
                        }
                    } else {
                        draw_turned_shape(pixels, *width, *height, shape);
                    }
                }
                Drawn::Label(label) => draw_label(pixels, *width, *height, label),
                Drawn::Image(image) => draw_image(pixels, *width, *height, image),
            }
        }
    }

    /// the mouse position in framebuffer pixels (the window may be shown at another size), and the
    /// buttons that went down or up since the last frame
    fn read_mouse(&mut self) {
        let (window_w, window_h) = self.backend.size();
        match self.backend.mouse_position() {
            Some((x, y)) => {
                self.mouse_inside = true;
                self.mouse_x = x * self.width as f64 / window_w.max(1) as f64;
                self.mouse_y = y * self.height as f64 / window_h.max(1) as f64;
            }
            None => self.mouse_inside = false,
        }
        let now: HashSet<Button> = Button::ALL.into_iter().filter(|&b| self.backend.is_button_down(b)).collect();
        if self.mouse_read {
            self.buttons_pressed = now.difference(&self.buttons_down).copied().collect();
            self.buttons_released = self.buttons_down.difference(&now).copied().collect();
        } else {
            self.buttons_pressed.clear();
            self.buttons_released.clear();
            self.mouse_read = true;
        }
        self.buttons_down = now;
        self.route_clicks();
    }

    /// the topmost shape under the pointer that takes clicks, and what it gets: a button that goes
    /// down belongs to the shape under the pointer at that moment, and the shape keeps it until the
    /// button goes up, even if the pointer leaves the shape meanwhile
    fn route_clicks(&mut self) {
        self.hovered = if self.mouse_inside { self.hit_test() } else { None };
        self.shape_down.clear();
        self.shape_pressed.clear();
        self.shape_released.clear();
        for button in self.buttons_pressed.clone() {
            match self.hovered {
                Some(id) => {
                    self.captured.insert(button, id);
                    self.shape_pressed.insert((id, button));
                }
                None => {
                    self.captured.remove(&button);
                }
            }
        }
        for button in self.buttons_released.clone() {
            if let Some(id) = self.captured.remove(&button) {
                self.shape_released.insert((id, button));
            }
        }
        for (&button, &id) in &self.captured {
            if self.buttons_down.contains(&button) {
                self.shape_down.insert((id, button));
            }
        }
    }

    /// the topmost visible shape under the pointer that is not click-through. shapes that were
    /// killed are not in the scene, so they are never found
    fn hit_test(&self) -> Option<ShapeId> {
        let (x, y) = (self.mouse_x, self.mouse_y);
        self.scene
            .draw_order_ids()
            .into_iter()
            .rev()
            .find(|(_, s)| !s.click_through() && s.contains(x, y))
            .map(|(id, _)| id)
    }

    fn read_keys(&mut self) {
        self.down = Key::ALL.iter().copied().filter(|&k| self.backend.is_down(k)).collect();
        self.pressed = Key::ALL.iter().copied().filter(|&k| self.backend.was_pressed(k)).collect();
        self.released = Key::ALL.iter().copied().filter(|&k| self.backend.was_released(k)).collect();
    }

    fn close_now(&mut self) {
        self.backend.close();
        self.scene.clear();
        self.pixels = Vec::new(); // a closed window holds no framebuffer
        self.state = State::Closed;
    }
}

impl<B: Backend> Drop for Session<B> {
    fn drop(&mut self) {
        if self.state != State::Closed {
            self.close_now();
        }
    }
}

/// paints a rectangle of the framebuffer, cut at its edges. `x` and `y` are rounded to pixels
#[allow(clippy::too_many_arguments)]
fn fill_box(pixels: &mut [u32], width: usize, height: usize, x: f64, y: f64, w: f64, h: f64, color: u32) {
    let x0 = (x.round() as i64).clamp(0, width as i64) as usize;
    let y0 = (y.round() as i64).clamp(0, height as i64) as usize;
    let x1 = ((x + w).round() as i64).clamp(0, width as i64) as usize;
    let y1 = ((y + h).round() as i64).clamp(0, height as i64) as usize;
    if x1 <= x0 {
        return;
    }
    for row in y0..y1 {
        let start = row * width;
        pixels[start + x0..start + x1].fill(color);
    }
}

/// paints the outline of a shape inside its edges: four bands `stroke` pixels thick, cut at the
/// window like any rectangle
#[allow(clippy::too_many_arguments)]
fn draw_outline(pixels: &mut [u32], width: usize, height: usize, shape: &Shape, stroke: f64) {
    let (x, y, w, h, c) = (shape.x(), shape.y(), shape.w(), shape.h(), shape.stroke_color());
    fill_box(pixels, width, height, x, y, w, stroke, c);
    fill_box(pixels, width, height, x, y + h - stroke, w, stroke, c);
    fill_box(pixels, width, height, x, y, stroke, h, c);
    fill_box(pixels, width, height, x + w - stroke, y, stroke, h, c);
}

/// draws a shape that is turned, see-through, without fill or an ellipse. every pixel of its box is
/// tested against the shape's own geometry at the pixel's center, then the pixel takes the fill or
/// the outline, mixed by the shape's alpha
fn draw_turned_shape(pixels: &mut [u32], width: usize, height: usize, shape: &Shape) {
    let alpha = (shape.alpha() * 255.0).round() as u8;
    if alpha == 0 {
        return;
    }
    let (left, top, right, bottom) = shape.bounds();
    let x0 = left.floor().clamp(0.0, width as f64) as usize;
    let y0 = top.floor().clamp(0.0, height as f64) as usize;
    let x1 = right.ceil().clamp(0.0, width as f64) as usize;
    let y1 = bottom.ceil().clamp(0.0, height as f64) as usize;
    let turn = shape.turn();
    let stroke = shape.stroke().round();
    for py in y0..y1 {
        for px in x0..x1 {
            let local = shape.to_local(turn, px as f64 + 0.5, py as f64 + 0.5);
            if !shape.covers(local, 0.0) {
                continue;
            }
            if shape.covers(local, stroke) {
                if shape.fill() {
                    blend(pixels, width, height, px as i64, py as i64, shape.color(), alpha);
                }
            } else {
                blend(pixels, width, height, px as i64, py as i64, shape.stroke_color(), alpha);
            }
        }
    }
}

/// draws an image into its `width` x `height` pixels, nearest neighbor: each screen pixel takes the
/// picture pixel under it. the pixels that are not transparent are mixed into the framebuffer by their
/// alpha, and the part outside the window is cut off
fn draw_image(pixels: &mut [u32], width: usize, height: usize, image: &Image) {
    let picture = image.picture();
    let (dw, dh) = (image.width() as i64, image.height() as i64);
    let (pw, ph) = (picture.width() as i64, picture.height() as i64);
    let x0 = image.x().round() as i64;
    let y0 = image.y().round() as i64;
    let x1 = (x0 + dw).min(width as i64);
    let y1 = (y0 + dh).min(height as i64);
    for py in y0.max(0)..y1 {
        // the picture row under this screen row: never negative, since py >= y0, and below ph since py - y0 < dh
        let row = ((py - y0) * ph / dh) as usize;
        for px in x0.max(0)..x1 {
            let column = ((px - x0) * pw / dw) as usize;
            let [r, g, b, alpha] = picture.pixel(column, row);
            if alpha > 0 {
                let color = (r as u32) << 16 | (g as u32) << 8 | b as u32;
                blend(pixels, width, height, px, py, color, alpha);
            }
        }
    }
}

/// draws a label. when it has a stroke, the outline is drawn first, from every offset inside the
/// stroke's radius, and the text is drawn over it, so the outline never covers the text itself
fn draw_label(pixels: &mut [u32], width: usize, height: usize, label: &Label) {
    let stroke = label.stroke();
    if stroke > 0.0 {
        let reach = stroke.ceil() as i64;
        for dy in -reach..=reach {
            for dx in -reach..=reach {
                if (dx * dx + dy * dy) as f64 <= stroke * stroke {
                    draw_text(pixels, width, height, label, dx as f64, dy as f64, Some(label.stroke_color()));
                }
            }
        }
    }
    draw_text(pixels, width, height, label, 0.0, 0.0, None);
}

/// draws the lines of a label shifted by (`shift_x`, `shift_y`). `paint` replaces every color of
/// the text (the outline uses it); `None` keeps the colors of the text and its tags
fn draw_text(pixels: &mut [u32], width: usize, height: usize, label: &Label, shift_x: f64, shift_y: f64, paint: Option<u32>) {
    let size = label.text_size();
    let font = label.font().clone();
    // the text was checked when it was set, so it parses
    let chars = text::parse(label.text()).unwrap_or_default();
    let spacing = label.spacing();
    let lines = text::layout(&chars, &font, size, &spacing, label.wrap());
    let line_pitch = text::line_pitch(&font, size, &spacing);
    let baseline = text::baseline(&font, size);
    let widths: Vec<f64> = lines.iter().map(|l| text::line_width(l, &font, size, &spacing)).collect();
    let block = if label.wrap() > 0.0 { label.wrap() } else { widths.iter().copied().fold(0.0, f64::max) };
    // italic: a pixel leans right by how far it is above the baseline, times the slant
    let slant = label.italic().to_radians().tan();
    for (line_index, line) in lines.iter().enumerate() {
        let offset = match label.align() {
            Align::Left => 0.0,
            Align::Center => (block - widths[line_index]) / 2.0,
            Align::Right => block - widths[line_index],
        };
        let top = label.y() + shift_y + line_index as f64 * line_pitch;
        let base_y = top + baseline;
        let mut pen = 0.0;
        for styled in line {
            let color = paint.unwrap_or_else(|| styled.color.unwrap_or(label.color()));
            let x0 = label.x() + shift_x + offset + pen;
            match text::coverage(&font, styled.c, size) {
                Some(glyph) => {
                    let gx = x0 + glyph.left;
                    let gy = top + baseline + glyph.top;
                    for row in 0..glyph.height {
                        let py = gy + row as f64;
                        for col in 0..glyph.width {
                            let alpha = glyph.pixels[row * glyph.width + col];
                            if alpha > 0 {
                                let px = gx + col as f64 + (base_y - py) * slant;
                                blend(pixels, width, height, px.round() as i64, py.round() as i64, color, alpha);
                            }
                        }
                    }
                }
                None => {
                    // the 8x8 grid stretched to `size` pixels: grid line i sits at i * size / 8, so every
                    // cell is drawn as one box with whole-pixel edges
                    let cell = text::CELL as usize;
                    let edge = |i: usize| (i as u32 * size / text::CELL) as f64;
                    for (row, bits) in text::glyph(styled.c).iter().enumerate() {
                        for col in 0..cell {
                            if bits >> col & 1 == 1 {
                                let (y, h) = (top + edge(row), edge(row + 1) - edge(row));
                                let (dx, w) = (edge(col), edge(col + 1) - edge(col));
                                let x = x0 + dx + (base_y - y) * slant;
                                fill_box(pixels, width, height, x, y, w, h, color);
                            }
                        }
                    }
                }
            }
            pen += text::advance(&font, styled.c, size) + spacing.letter + if styled.c == ' ' { spacing.word } else { 0.0 };
        }
    }
}

/// mixes `color` into the pixel at (x, y) by `alpha` (0 to 255), where the pixel is on screen
fn blend(pixels: &mut [u32], width: usize, height: usize, x: i64, y: i64, color: u32, alpha: u8) {
    if x < 0 || y < 0 || x >= width as i64 || y >= height as i64 {
        return;
    }
    let index = y as usize * width + x as usize;
    let background = pixels[index];
    let a = alpha as u32;
    let mix = |shift: u32| {
        let bg = (background >> shift) & 0xff;
        let fg = (color >> shift) & 0xff;
        (bg * (255 - a) + fg * a + 127) / 255
    };
    pixels[index] = (mix(16) << 16) | (mix(8) << 8) | mix(0);
}
