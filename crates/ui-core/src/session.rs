//! a window while it runs: the scene, the framebuffer, the keys and the frame time.
//! it knows neither the window library nor spg: it gets the backend and the time from outside.

use std::collections::{HashMap, HashSet};

use crate::backend::{Backend, Button, Key};
use crate::scene::{Scene, Shape, ShapeId};

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

    pub fn shape(&self, id: ShapeId) -> Result<&Shape, String> {
        self.scene()?.get(id)
    }

    pub fn shape_mut(&mut self, id: ShapeId) -> Result<&mut Shape, String> {
        self.scene_mut()?.get_mut(id)
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

    /// the background, then every visible shape back to front. the parts outside the window are
    /// cut off
    fn render(&mut self) {
        let Session { scene, pixels, width, height, .. } = self;
        pixels.fill(scene.background());
        for shape in scene.draw_order() {
            let x0 = (shape.x().round() as i64).clamp(0, *width as i64) as usize;
            let y0 = (shape.y().round() as i64).clamp(0, *height as i64) as usize;
            let x1 = ((shape.x() + shape.w()).round() as i64).clamp(0, *width as i64) as usize;
            let y1 = ((shape.y() + shape.h()).round() as i64).clamp(0, *height as i64) as usize;
            if x1 <= x0 {
                continue;
            }
            for row in y0..y1 {
                let start = row * *width;
                pixels[start + x0..start + x1].fill(shape.color());
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
            .find(|(_, s)| !s.click_through() && x >= s.x() && x < s.x() + s.w() && y >= s.y() && y < s.y() + s.h())
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
