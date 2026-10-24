//! a window while it runs: the scene, the framebuffer, the keys and the frame time.
//! it knows neither the window library nor spg: it gets the backend and the time from outside.

use std::collections::HashSet;

use crate::backend::{Backend, Key};
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
