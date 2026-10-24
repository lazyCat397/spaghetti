//! the real backend, on `minifb`. checked by hand on Windows: the CI has no display

use std::collections::HashSet;

use minifb::{Key as MKey, KeyRepeat, Window, WindowOptions};

use crate::backend::{Backend, Key};

#[derive(Default)]
pub struct MinifbBackend {
    window: Option<Window>,
    /// `show` already pumped the events for this frame, so `poll` must not pump again: a
    /// second pump would clear the key presses the first one collected
    pumped: bool,
    pressed: HashSet<Key>,
    released: HashSet<Key>,
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
        let window = Window::new(title, width, height, WindowOptions::default())
            .map_err(|e| e.to_string())?;
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

    fn close(&mut self) {
        self.window = None;
    }
}
