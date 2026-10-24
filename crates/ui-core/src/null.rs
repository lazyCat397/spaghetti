//! the null backend: opens no window. for the tests and the CI, which have no display.
//! a test presses keys and closes the window itself, and reads back the last frame shown.

use std::cell::Cell;
use std::collections::HashSet;
use std::rc::Rc;

use crate::backend::{Backend, Key};

#[derive(Default)]
pub struct NullBackend {
    down: HashSet<Key>,
    /// presses and releases made since the last `poll`
    pending_pressed: HashSet<Key>,
    pending_released: HashSet<Key>,
    /// what the last `poll` reports
    just_pressed: HashSet<Key>,
    just_released: HashSet<Key>,
    user_closed: bool,
    open: bool,
    /// how many frames were shown
    pub frames_shown: usize,
    /// a copy of the last frame shown
    pub last_frame: Vec<u32>,
    /// how many times the window was really closed. shared, so a test can read it after the
    /// session is dropped
    closes: Rc<Cell<usize>>,
}

impl NullBackend {
    pub fn new() -> Self {
        Self::default()
    }

    /// simulates a key going down
    pub fn press(&mut self, key: Key) {
        self.down.insert(key);
        self.pending_pressed.insert(key);
    }

    /// simulates a key going up
    pub fn release(&mut self, key: Key) {
        self.down.remove(&key);
        self.pending_released.insert(key);
    }

    /// simulates the user clicking the window's X button
    pub fn close_by_user(&mut self) {
        self.user_closed = true;
    }

    pub fn is_open(&self) -> bool {
        self.open
    }

    /// a close counter that outlives the session
    pub fn close_counter(&self) -> Rc<Cell<usize>> {
        Rc::clone(&self.closes)
    }
}

impl Backend for NullBackend {
    fn open(&mut self, _title: &str, _width: usize, _height: usize) -> Result<(), String> {
        self.open = true;
        Ok(())
    }

    fn show(&mut self, pixels: &[u32], _width: usize, _height: usize) -> Result<(), String> {
        self.frames_shown += 1;
        self.last_frame = pixels.to_vec();
        Ok(())
    }

    fn poll(&mut self) -> bool {
        self.just_pressed = std::mem::take(&mut self.pending_pressed);
        self.just_released = std::mem::take(&mut self.pending_released);
        !self.user_closed
    }

    fn is_down(&self, key: Key) -> bool {
        self.down.contains(&key)
    }

    fn was_pressed(&self, key: Key) -> bool {
        self.just_pressed.contains(&key)
    }

    fn was_released(&self, key: Key) -> bool {
        self.just_released.contains(&key)
    }

    fn close(&mut self) {
        if self.open {
            self.closes.set(self.closes.get() + 1);
        }
        self.open = false;
    }
}
