//! the contract between the core and a window library. no type here exposes the library.

/// the keys the core knows. a program names them: "left", "escape", "w"...
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Key {
    Left,
    Right,
    Up,
    Down,
    Escape,
    W,
    A,
    S,
    D,
}

/// the name a program writes -> the key. `None` when the core does not know it
pub fn key_from_name(name: &str) -> Option<Key> {
    Some(match name {
        "left" => Key::Left,
        "right" => Key::Right,
        "up" => Key::Up,
        "down" => Key::Down,
        "escape" => Key::Escape,
        "w" => Key::W,
        "a" => Key::A,
        "s" => Key::S,
        "d" => Key::D,
        _ => return None,
    })
}

/// what a backend must do: open, show pixels, read the keys, close
pub trait Backend {
    /// opens the window. called once
    fn open(&mut self, title: &str, width: usize, height: usize) -> Result<(), String>;

    /// shows the framebuffer (`0x00RRGGBB`, row by row, top first)
    fn show(&mut self, pixels: &[u32], width: usize, height: usize) -> Result<(), String>;

    /// handles the system events and updates the key state.
    /// returns `false` when the user closed the window
    fn poll(&mut self) -> bool;

    /// whether the key is down right now (true on every frame it stays down)
    fn is_down(&self, key: Key) -> bool;

    /// closes the window and frees it. may be called more than once
    fn close(&mut self);
}

/// a boxed backend is a backend, so the core can pick one at run time
impl<T: Backend + ?Sized> Backend for Box<T> {
    fn open(&mut self, title: &str, width: usize, height: usize) -> Result<(), String> {
        (**self).open(title, width, height)
    }
    fn show(&mut self, pixels: &[u32], width: usize, height: usize) -> Result<(), String> {
        (**self).show(pixels, width, height)
    }
    fn poll(&mut self) -> bool {
        (**self).poll()
    }
    fn is_down(&self, key: Key) -> bool {
        (**self).is_down(key)
    }
    fn close(&mut self) {
        (**self).close()
    }
}
