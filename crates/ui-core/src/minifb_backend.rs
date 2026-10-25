//! the real backend, on `minifb`. checked by hand on Windows: the CI has no display

use std::collections::HashSet;

use std::cell::RefCell;
use std::rc::Rc;

use minifb::{InputCallback, Key as MKey, KeyRepeat, MouseButton, MouseMode, Window, WindowOptions};

use crate::backend::{Backend, Button, Key};

#[derive(Default)]
pub struct MinifbBackend {
    window: Option<Window>,
    /// `show` already pumped the events for this frame, so `poll` must not pump again: a
    /// second pump would clear the key presses the first one collected
    pumped: bool,
    pressed: HashSet<Key>,
    released: HashSet<Key>,
    focused: bool,
    size: (usize, usize),
    mouse: Option<(f64, f64)>,
    /// left, right, middle
    buttons: [bool; 3],
    /// the characters typed, collected by the callback of the window until the session takes them
    typed: Rc<RefCell<String>>,
}

/// receives the characters the window reports (minifb calls it while it handles events)
struct TypedCollector(Rc<RefCell<String>>);

impl InputCallback for TypedCollector {
    fn add_char(&mut self, uni_char: u32) {
        if let Some(c) = char::from_u32(uni_char) {
            self.0.borrow_mut().push(c);
        }
    }
}

fn to_minifb(key: Key) -> MKey {
    match key {
        Key::A => MKey::A,
        Key::B => MKey::B,
        Key::C => MKey::C,
        Key::D => MKey::D,
        Key::E => MKey::E,
        Key::F => MKey::F,
        Key::G => MKey::G,
        Key::H => MKey::H,
        Key::I => MKey::I,
        Key::J => MKey::J,
        Key::K => MKey::K,
        Key::L => MKey::L,
        Key::M => MKey::M,
        Key::N => MKey::N,
        Key::O => MKey::O,
        Key::P => MKey::P,
        Key::Q => MKey::Q,
        Key::R => MKey::R,
        Key::S => MKey::S,
        Key::T => MKey::T,
        Key::U => MKey::U,
        Key::V => MKey::V,
        Key::W => MKey::W,
        Key::X => MKey::X,
        Key::Y => MKey::Y,
        Key::Z => MKey::Z,
        Key::Num0 => MKey::Key0,
        Key::Num1 => MKey::Key1,
        Key::Num2 => MKey::Key2,
        Key::Num3 => MKey::Key3,
        Key::Num4 => MKey::Key4,
        Key::Num5 => MKey::Key5,
        Key::Num6 => MKey::Key6,
        Key::Num7 => MKey::Key7,
        Key::Num8 => MKey::Key8,
        Key::Num9 => MKey::Key9,
        Key::F1 => MKey::F1,
        Key::F2 => MKey::F2,
        Key::F3 => MKey::F3,
        Key::F4 => MKey::F4,
        Key::F5 => MKey::F5,
        Key::F6 => MKey::F6,
        Key::F7 => MKey::F7,
        Key::F8 => MKey::F8,
        Key::F9 => MKey::F9,
        Key::F10 => MKey::F10,
        Key::F11 => MKey::F11,
        Key::F12 => MKey::F12,
        Key::Left => MKey::Left,
        Key::Right => MKey::Right,
        Key::Up => MKey::Up,
        Key::Down => MKey::Down,
        Key::Space => MKey::Space,
        Key::Enter => MKey::Enter,
        Key::Escape => MKey::Escape,
        Key::Tab => MKey::Tab,
        Key::Backspace => MKey::Backspace,
        Key::Delete => MKey::Delete,
        Key::Home => MKey::Home,
        Key::End => MKey::End,
        Key::LeftShift => MKey::LeftShift,
        Key::RightShift => MKey::RightShift,
        Key::LeftCtrl => MKey::LeftCtrl,
        Key::RightCtrl => MKey::RightCtrl,
        Key::LeftAlt => MKey::LeftAlt,
        Key::RightAlt => MKey::RightAlt,
    }
}

/// the core key for a minifb key, if the core knows it
fn from_minifb(key: MKey) -> Option<Key> {
    Key::ALL.iter().copied().find(|&k| to_minifb(k) == key)
}

impl Backend for MinifbBackend {
    fn open(&mut self, title: &str, width: usize, height: usize) -> Result<(), String> {
        // resizable: the program can react to the size with onResize
        let options = WindowOptions { resize: true, ..WindowOptions::default() };
        let window = Window::new(title, width, height, options).map_err(|e| e.to_string())?;
        self.size = (width, height);
        let mut window = window;
        window.set_input_callback(Box::new(TypedCollector(Rc::clone(&self.typed))));
        self.window = Some(window);
        Ok(())
    }

    fn show(&mut self, pixels: &[u32], width: usize, height: usize) -> Result<(), String> {
        let window = self.window.as_mut().ok_or("the window is not open")?;
        window
            .update_with_buffer(pixels, width, height)
            .map_err(|e| e.to_string())?;
        self.pumped = true;
        Ok(())
    }

    fn poll(&mut self) -> bool {
        let Some(window) = self.window.as_mut() else { return false };
        if !self.pumped {
            window.update();
        }
        self.pumped = false;
        self.pressed = window
            .get_keys_pressed(KeyRepeat::No)
            .into_iter()
            .filter_map(from_minifb)
            .collect();
        self.released = window.get_keys_released().into_iter().filter_map(from_minifb).collect();
        self.focused = window.is_active();
        self.size = window.get_size();
        // `Discard`: a position only while the pointer is over the window
        self.mouse = window
            .get_mouse_pos(MouseMode::Discard)
            .map(|(x, y)| (x as f64, y as f64));
        self.buttons = [
            window.get_mouse_down(MouseButton::Left),
            window.get_mouse_down(MouseButton::Right),
            window.get_mouse_down(MouseButton::Middle),
        ];
        window.is_open()
    }

    fn is_down(&self, key: Key) -> bool {
        self.window
            .as_ref()
            .map_or(false, |window| window.is_key_down(to_minifb(key)))
    }

    fn was_pressed(&self, key: Key) -> bool {
        self.pressed.contains(&key)
    }

    fn was_released(&self, key: Key) -> bool {
        self.released.contains(&key)
    }

    fn is_focused(&self) -> bool {
        self.focused
    }

    fn size(&self) -> (usize, usize) {
        self.size
    }

    fn mouse_position(&self) -> Option<(f64, f64)> {
        self.mouse
    }

    fn take_typed(&mut self) -> String {
        std::mem::take(&mut *self.typed.borrow_mut())
    }

    fn is_button_down(&self, button: Button) -> bool {
        self.buttons[match button {
            Button::Left => 0,
            Button::Right => 1,
            Button::Middle => 2,
        }]
    }

    fn close(&mut self) {
        self.window = None;
    }
}
