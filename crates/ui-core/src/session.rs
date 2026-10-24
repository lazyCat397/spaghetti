//! a window while it runs: the loop state, the framebuffer, the keys and the frame time.
//! it knows neither the window library nor spg: it gets the backend and the time from outside.

use std::collections::HashSet;

use crate::backend::{Backend, Key};

/// the largest `dt` the program gets, in seconds. stops jumps after a pause or a freeze
pub const MAX_DT: f64 = 0.1;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    /// open, but no frame has started yet: drawing is not allowed
    Created,
    /// inside the loop: drawing is allowed
    Running,
    /// closed: nothing else is allowed
    Closed,
}

pub struct Session<B: Backend> {
    backend: B,
    width: usize,
    height: usize,
    pixels: Vec<u32>,
    state: State,
    last_time: Option<f64>,
    close_requested: bool,
    /// the keys read at the start of the current frame. a snapshot: the whole frame sees
    /// the same keys, even if the user presses one in the middle of it
    down: HashSet<Key>,
    pressed: HashSet<Key>,
    released: HashSet<Key>,
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
            state: State::Created,
            last_time: None,
            close_requested: false,
            down: HashSet::new(),
            pressed: HashSet::new(),
            released: HashSet::new(),
        })
    }

    pub fn state(&self) -> State {
        self.state
    }

    /// starts a frame. `now` is the current time in seconds (monotonic).
    ///
    /// shows the previous frame, reads the keys and returns the frame's `dt`.
    /// returns `None` when the window must close: the user closed it, Esc was pressed, or
    /// `request_close` was called during the previous frame. the state then becomes `Closed`.
    pub fn begin_frame(&mut self, now: f64) -> Result<Option<f64>, String> {
        if self.state == State::Closed {
            return Ok(None);
        }
        if self.state == State::Running {
            self.backend.show(&self.pixels, self.width, self.height)?;
        }
        let still_open = self.backend.poll();
        self.read_keys();

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
    /// `begin_frame`
    pub fn request_close(&mut self) -> Result<(), String> {
        self.require_running()?;
        self.close_requested = true;
        Ok(())
    }

    /// paints the whole window with one color
    pub fn clear(&mut self, color: u32) -> Result<(), String> {
        self.require_running()?;
        self.pixels.fill(color);
        Ok(())
    }

    /// a filled rectangle, in pixels. the parts outside the window are cut off
    pub fn rect(&mut self, x: f64, y: f64, w: f64, h: f64, color: u32) -> Result<(), String> {
        self.require_running()?;
        if ![x, y, w, h].iter().all(|v| v.is_finite()) {
            return Err("coordinates and size must be finite numbers".to_string());
        }
        let x0 = (x.round() as i64).clamp(0, self.width as i64) as usize;
        let y0 = (y.round() as i64).clamp(0, self.height as i64) as usize;
        let x1 = ((x + w).round() as i64).clamp(0, self.width as i64) as usize;
        let y1 = ((y + h).round() as i64).clamp(0, self.height as i64) as usize;
        for row in y0..y1 {
            let start = row * self.width;
            self.pixels[start + x0..start + x1.max(x0)].fill(color);
        }
        Ok(())
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

    /// a pixel of the current framebuffer, for tests and debugging
    pub fn pixel(&self, x: usize, y: usize) -> Option<u32> {
        if x < self.width && y < self.height {
            Some(self.pixels[y * self.width + x])
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

    fn read_keys(&mut self) {
        self.down = Key::ALL.iter().copied().filter(|&k| self.backend.is_down(k)).collect();
        self.pressed = Key::ALL.iter().copied().filter(|&k| self.backend.was_pressed(k)).collect();
        self.released = Key::ALL.iter().copied().filter(|&k| self.backend.was_released(k)).collect();
    }

    fn require_running(&self) -> Result<(), String> {
        match self.state {
            State::Running => Ok(()),
            State::Created => Err("the window is not running yet: call nextFrame first".to_string()),
            State::Closed => Err("the window is closed".to_string()),
        }
    }

    fn close_now(&mut self) {
        self.backend.close();
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
