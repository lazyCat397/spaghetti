//! the real backend, on `minifb`. checked by hand on Windows: the CI has no display

use minifb::{Key as MKey, Window, WindowOptions};

use crate::backend::{Backend, Key};

#[derive(Default)]
pub struct MinifbBackend {
    window: Option<Window>,
}

fn to_minifb(key: Key) -> MKey {
    match key {
        Key::Left => MKey::Left,
        Key::Right => MKey::Right,
        Key::Up => MKey::Up,
        Key::Down => MKey::Down,
        Key::Escape => MKey::Escape,
        Key::W => MKey::W,
        Key::A => MKey::A,
        Key::S => MKey::S,
        Key::D => MKey::D,
    }
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
            .map_err(|e| e.to_string())
    }

    fn poll(&mut self) -> bool {
        match self.window.as_mut() {
            Some(window) => {
                window.update();
                window.is_open()
            }
            None => false,
        }
    }

    fn is_down(&self, key: Key) -> bool {
        self.window
            .as_ref()
            .map_or(false, |window| window.is_key_down(to_minifb(key)))
    }

    fn close(&mut self) {
        self.window = None;
    }
}
