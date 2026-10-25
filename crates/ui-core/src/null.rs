//! the null backend: opens no window. for the tests and the CI, which have no display.
//! a test presses keys and closes the window itself, and reads back the last frame shown.

use std::cell::{Cell, RefCell};
use std::collections::HashSet;
use std::rc::Rc;

use crate::backend::{Backend, Button, Key};

/// the state of the simulated mouse
#[derive(Default)]
pub struct NullMouse {
    /// where the pointer is, in window pixels, or `None` when it is outside the window
    pub position: Option<(f64, f64)>,
    pub down: HashSet<Button>,
}

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
    /// the typed characters, shared so a test can type while the session holds the backend
    typed: Rc<RefCell<String>>,
    /// the mouse, shared so a test can move it while the session holds the backend
    mouse: Rc<RefCell<NullMouse>>,
    /// shared, so a test can take the focus away while the session holds the backend
    focused: Rc<Cell<bool>>,
    /// shared like `focused`: a test resizes the window while the session holds the backend
    size: Rc<Cell<(usize, usize)>>,
    /// how many frames were shown
    pub frames_shown: usize,
    /// a copy of the last frame shown
    pub last_frame: Vec<u32>,
    /// how many times the window was really closed. shared, so a test can read it after the
    /// session is dropped
    closes: Rc<Cell<usize>>,
}

impl NullBackend {
    /// a backend that starts with the window focused
    pub fn new() -> Self {
        let backend = Self::default();
        backend.focused.set(true);
        backend
    }

    /// what a test types: the next frame reads it
    pub fn typed_handle(&self) -> Rc<RefCell<String>> {
        Rc::clone(&self.typed)
    }

    /// the mouse: a test moves it or presses a button while the session holds the backend
    pub fn mouse_handle(&self) -> Rc<RefCell<NullMouse>> {
        Rc::clone(&self.mouse)
    }

    /// the size of the window: `set` changes it for the session too
    pub fn size_handle(&self) -> Rc<Cell<(usize, usize)>> {
        Rc::clone(&self.size)
    }

    /// the focus of the window: `set` changes it for the session too
    pub fn focus_handle(&self) -> Rc<Cell<bool>> {
        Rc::clone(&self.focused)
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
    fn open(&mut self, _title: &str, width: usize, height: usize) -> Result<(), String> {
        self.open = true;
        self.size.set((width, height));
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

    fn is_focused(&self) -> bool {
        self.focused.get()
    }

    fn size(&self) -> (usize, usize) {
        self.size.get()
    }

    fn mouse_position(&self) -> Option<(f64, f64)> {
        self.mouse.borrow().position
    }

    fn is_button_down(&self, button: Button) -> bool {
        self.mouse.borrow().down.contains(&button)
    }

    fn take_typed(&mut self) -> String {
        std::mem::take(&mut *self.typed.borrow_mut())
    }

    fn close(&mut self) {
        if self.open {
            self.closes.set(self.closes.get() + 1);
        }
        self.open = false;
    }
}
