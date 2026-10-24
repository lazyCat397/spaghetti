//! the contract between the core and a window library. no type here exposes the library.

/// every key the core knows. the variant names are the ones a program writes: `Key.Space`
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Key {
    A,
    B,
    C,
    D,
    E,
    F,
    G,
    H,
    I,
    J,
    K,
    L,
    M,
    N,
    O,
    P,
    Q,
    R,
    S,
    T,
    U,
    V,
    W,
    X,
    Y,
    Z,
    Num0,
    Num1,
    Num2,
    Num3,
    Num4,
    Num5,
    Num6,
    Num7,
    Num8,
    Num9,
    F1,
    F2,
    F3,
    F4,
    F5,
    F6,
    F7,
    F8,
    F9,
    F10,
    F11,
    F12,
    Left,
    Right,
    Up,
    Down,
    Space,
    Enter,
    Escape,
    Tab,
    Backspace,
    Delete,
    Home,
    End,
    LeftShift,
    RightShift,
    LeftCtrl,
    RightCtrl,
    LeftAlt,
    RightAlt,
}

impl Key {
    /// every key, in declaration order
    pub const ALL: &'static [Key] = &[Key::A, Key::B, Key::C, Key::D, Key::E, Key::F, Key::G, Key::H, Key::I, Key::J, Key::K, Key::L, Key::M, Key::N, Key::O, Key::P, Key::Q, Key::R, Key::S, Key::T, Key::U, Key::V, Key::W, Key::X, Key::Y, Key::Z, Key::Num0, Key::Num1, Key::Num2, Key::Num3, Key::Num4, Key::Num5, Key::Num6, Key::Num7, Key::Num8, Key::Num9, Key::F1, Key::F2, Key::F3, Key::F4, Key::F5, Key::F6, Key::F7, Key::F8, Key::F9, Key::F10, Key::F11, Key::F12, Key::Left, Key::Right, Key::Up, Key::Down, Key::Space, Key::Enter, Key::Escape, Key::Tab, Key::Backspace, Key::Delete, Key::Home, Key::End, Key::LeftShift, Key::RightShift, Key::LeftCtrl, Key::RightCtrl, Key::LeftAlt, Key::RightAlt];

    /// the key with this variant name, if there is one
    pub fn from_name(name: &str) -> Option<Key> {
        Key::ALL.iter().copied().find(|key| format!("{key:?}") == name)
    }
}

/// what a backend must do: open, show pixels, read the keys, close
pub trait Backend {
    /// opens the window. called once
    fn open(&mut self, title: &str, width: usize, height: usize) -> Result<(), String>;

    /// shows the framebuffer (`0x00RRGGBB`, row by row, top first)
    fn show(&mut self, pixels: &[u32], width: usize, height: usize) -> Result<(), String>;

    /// handles the system events. after this call, `was_pressed` and `was_released` describe
    /// the events since the previous call. returns `false` when the user closed the window
    fn poll(&mut self) -> bool;

    /// whether the key is down right now
    fn is_down(&self, key: Key) -> bool;

    /// whether the key went down since the previous `poll`. a backend that reads events
    /// reports every press; one that reads the key state can miss a tap shorter than a frame
    /// (the minifb backend does: see the spec)
    fn was_pressed(&self, key: Key) -> bool;

    /// whether the key went up since the previous `poll`
    fn was_released(&self, key: Key) -> bool;

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
    fn was_pressed(&self, key: Key) -> bool {
        (**self).was_pressed(key)
    }
    fn was_released(&self, key: Key) -> bool {
        (**self).was_released(key)
    }
    fn close(&mut self) {
        (**self).close()
    }
}
