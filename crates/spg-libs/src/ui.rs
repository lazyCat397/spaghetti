//! `ui`: a window to draw rectangles in, and the keyboard (`install #ui`). the core is in
//! `ui-core`; this file is the bridge to the program's values.
//!
//! a library can not call the program's functions, so there is no `run(callback)`: the
//! program writes its own loop and asks the window for the time of each frame:
//!
//! ```text
//! install #ui
//!
//! let win = ui.window("demo", 800, 600)
//! let x = 100.0
//! while (true) {
//!     let dt = ui.nextFrame(win)
//!     if (dt < 0.0) { break }              // closed: Esc, the X button or ui.close(win)
//!     if (ui.isDown(win, Key.Right)) { x = x + 200.0 * dt }
//!     if (ui.wasPressed(win, Key.Space)) { print("jump") }
//!     ui.clear(win, ui.hex("#101010"))
//!     ui.rect(win, x, 100.0, 40.0, 40.0, ui.hex("#ff3300"))
//! }
//! ```
//!
//! the windows live here, one set per thread, and the program holds the id in its `Window`.

use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::OnceLock;
use std::time::Instant;

use ui_core::{color, Backend, Key, Session};

use crate::{CallCtx, Kind, LibError, LibValue, NativeFn, NativeModule};

/// the largest side a window may have, in pixels
const MAX_SIDE: i64 = 8192;

/// the largest color value: `0xFFFFFF`
const MAX_COLOR: i64 = 0xFF_FFFF;

/// the `Key` enum the program sees. it lists the same names as `ui_core::Key`; a test checks
/// that the two stay equal
const KEY_ENUM: &str = "enum Key { A, B, C, D, E, F, G, H, I, J, K, L, M, N, O, P, Q, R, S, T, U, V, W, X, Y, Z, Num0, Num1, Num2, Num3, Num4, Num5, Num6, Num7, Num8, Num9, F1, F2, F3, F4, F5, F6, F7, F8, F9, F10, F11, F12, Left, Right, Up, Down, Space, Enter, Escape, Tab, Backspace, Delete, Home, End, LeftShift, RightShift, LeftCtrl, RightCtrl, LeftAlt, RightAlt }";

pub static MODULE: NativeModule = NativeModule {
    kind: Kind::Install,
    name: "ui",
    doc: "a window to draw rectangles in, and the keyboard (install #ui)",
    structs: &["struct Window {\n    id: int\n}\n", KEY_ENUM],
    functions: &[
        NativeFn { name: "window", sig: "(string, int, int) -> Window", doc: "opens a window of the given size, in pixels. it shows when the loop starts", call: window },
        NativeFn { name: "nextFrame", sig: "(Window) -> float", doc: "starts the next frame: shows the last one, reads the keys and gives the seconds since the last frame (at most 0.1). gives -1.0 when the window is closing, which ends the loop", call: next_frame },
        NativeFn { name: "clear", sig: "(Window, int) -> void", doc: "paints the whole window with a color (ui.hex or ui.rgb)", call: clear },
        NativeFn { name: "rect", sig: "(Window, float, float, float, float, int) -> void", doc: "draws a filled rectangle: x, y, width, height in pixels, then a color. the part outside the window is cut off", call: rect },
        NativeFn { name: "isDown", sig: "(Window, Key) -> bool", doc: "true on every frame the key is held", call: is_down },
        NativeFn { name: "wasPressed", sig: "(Window, Key) -> bool", doc: "true on the one frame the key went down. a tap shorter than one frame can be missed", call: was_pressed },
        NativeFn { name: "wasReleased", sig: "(Window, Key) -> bool", doc: "true on the one frame the key went up", call: was_released },
        NativeFn { name: "close", sig: "(Window) -> void", doc: "asks the window to close. the frame in progress finishes, then the loop ends", call: close },
        NativeFn { name: "hex", sig: "(string) -> int", doc: "a color from \"#rrggbb\"", call: hex },
        NativeFn { name: "rgb", sig: "(int, int, int) -> int", doc: "a color from red, green and blue, each from 0 to 255", call: rgb },
    ],
};

/// an open window: the core session, over whichever backend this build uses
type OpenWindow = Session<Box<dyn Backend>>;

#[derive(Default)]
struct Registry {
    last_id: i64,
    windows: HashMap<i64, OpenWindow>,
}

thread_local! {
    static REGISTRY: RefCell<Registry> = RefCell::new(Registry::default());
}

#[cfg(not(test))]
fn new_backend() -> Box<dyn Backend> {
    Box::new(ui_core::minifb_backend::MinifbBackend::default())
}

#[cfg(test)]
fn new_backend() -> Box<dyn Backend> {
    Box::new(ui_core::null::NullBackend::new())
}

/// seconds since the first call. the clock of the frames
fn now() -> f64 {
    static START: OnceLock<Instant> = OnceLock::new();
    START.get_or_init(Instant::now).elapsed().as_secs_f64()
}

fn window(_: &CallCtx, args: Vec<LibValue>) -> Result<LibValue, LibError> {
    const F: &str = "ui.window";
    let title = text(F, &args, 0)?;
    let width = int(F, &args, 1)?;
    let height = int(F, &args, 2)?;
    if !(1..=MAX_SIDE).contains(&width) || !(1..=MAX_SIDE).contains(&height) {
        return Err(LibError::new(F, format!("each side must be between 1 and {MAX_SIDE} pixels, got {width}x{height}")));
    }
    let session = Session::open(new_backend(), &title, width as usize, height as usize)
        .map_err(|e| LibError::new(F, format!("could not open the window ({e})")))?;
    let id = REGISTRY.with(|registry| {
        let mut registry = registry.borrow_mut();
        registry.last_id += 1;
        let id = registry.last_id;
        registry.windows.insert(id, session);
        id
    });
    Ok(window_value(id))
}

fn next_frame(_: &CallCtx, args: Vec<LibValue>) -> Result<LibValue, LibError> {
    const F: &str = "ui.nextFrame";
    let id = window_id(F, &args)?;
    let dt = with_window(F, id, |w| w.begin_frame(now()))?;
    Ok(LibValue::Float(dt.unwrap_or(-1.0)))
}

fn clear(_: &CallCtx, args: Vec<LibValue>) -> Result<LibValue, LibError> {
    const F: &str = "ui.clear";
    let id = window_id(F, &args)?;
    let color = color_arg(F, &args, 1)?;
    with_window(F, id, |w| w.clear(color))?;
    Ok(LibValue::Void)
}

fn rect(_: &CallCtx, args: Vec<LibValue>) -> Result<LibValue, LibError> {
    const F: &str = "ui.rect";
    let id = window_id(F, &args)?;
    let x = float(F, &args, 1)?;
    let y = float(F, &args, 2)?;
    let w = float(F, &args, 3)?;
    let h = float(F, &args, 4)?;
    let color = color_arg(F, &args, 5)?;
    with_window(F, id, |win| win.rect(x, y, w, h, color))?;
    Ok(LibValue::Void)
}

fn is_down(_: &CallCtx, args: Vec<LibValue>) -> Result<LibValue, LibError> {
    const F: &str = "ui.isDown";
    let id = window_id(F, &args)?;
    let key = key_arg(F, &args, 1)?;
    let down = with_window(F, id, |w| Ok(w.is_down(key)))?;
    Ok(LibValue::Bool(down))
}

fn was_pressed(_: &CallCtx, args: Vec<LibValue>) -> Result<LibValue, LibError> {
    const F: &str = "ui.wasPressed";
    let id = window_id(F, &args)?;
    let key = key_arg(F, &args, 1)?;
    let pressed = with_window(F, id, |w| Ok(w.was_pressed(key)))?;
    Ok(LibValue::Bool(pressed))
}

fn was_released(_: &CallCtx, args: Vec<LibValue>) -> Result<LibValue, LibError> {
    const F: &str = "ui.wasReleased";
    let id = window_id(F, &args)?;
    let key = key_arg(F, &args, 1)?;
    let released = with_window(F, id, |w| Ok(w.was_released(key)))?;
    Ok(LibValue::Bool(released))
}

fn close(_: &CallCtx, args: Vec<LibValue>) -> Result<LibValue, LibError> {
    const F: &str = "ui.close";
    let id = window_id(F, &args)?;
    with_window(F, id, |w| w.request_close())?;
    Ok(LibValue::Void)
}

fn hex(_: &CallCtx, args: Vec<LibValue>) -> Result<LibValue, LibError> {
    const F: &str = "ui.hex";
    let text = text(F, &args, 0)?;
    let value = color::hex(&text).map_err(|e| LibError::new(F, e))?;
    Ok(LibValue::Int(value as i64))
}

fn rgb(_: &CallCtx, args: Vec<LibValue>) -> Result<LibValue, LibError> {
    const F: &str = "ui.rgb";
    let r = int(F, &args, 0)?;
    let g = int(F, &args, 1)?;
    let b = int(F, &args, 2)?;
    let value = color::rgb(r, g, b).map_err(|e| LibError::new(F, e))?;
    Ok(LibValue::Int(value as i64))
}

// --- helpers ---------------------------------------------------------------------

/// the `Window` value the program holds
fn window_value(id: i64) -> LibValue {
    LibValue::record("Window", vec![("id", LibValue::Int(id))])
}

/// the id inside the `Window` at argument 0
fn window_id(function: &'static str, args: &[LibValue]) -> Result<i64, LibError> {
    match args.first() {
        Some(LibValue::Struct { name, fields }) if name == "Window" => fields
            .iter()
            .find_map(|(field, value)| match (field.as_str(), value) {
                ("id", LibValue::Int(id)) => Some(*id),
                _ => None,
            })
            .ok_or_else(|| LibError::new(function, "the Window has no id")),
        _ => Err(LibError::new(function, "expected a Window, made by ui.window")),
    }
}

/// a `Key` argument: the enum value, which is the variant's name
fn key_arg(function: &'static str, args: &[LibValue], i: usize) -> Result<Key, LibError> {
    match args.get(i) {
        Some(LibValue::Enum(name)) => Key::from_name(name)
            .ok_or_else(|| LibError::new(function, format!("unknown key {name}"))),
        _ => Err(LibError::new(function, format!("argument {} must be a Key, like Key.Space", i + 1))),
    }
}

/// runs `f` on the open window `id`. an unknown id is an error, a closed window is one too
fn with_window<T>(
    function: &'static str,
    id: i64,
    f: impl FnOnce(&mut OpenWindow) -> Result<T, String>,
) -> Result<T, LibError> {
    REGISTRY.with(|registry| {
        let mut registry = registry.borrow_mut();
        let window = registry
            .windows
            .get_mut(&id)
            .ok_or_else(|| LibError::new(function, "this window is not open"))?;
        f(window).map_err(|e| LibError::new(function, e))
    })
}

/// a color argument: a whole number from 0 to 0xFFFFFF
fn color_arg(function: &'static str, args: &[LibValue], i: usize) -> Result<u32, LibError> {
    let value = int(function, args, i)?;
    if !(0..=MAX_COLOR).contains(&value) {
        return Err(LibError::new(function, format!("a color must be between 0 and {MAX_COLOR}: make one with ui.hex or ui.rgb")));
    }
    Ok(value as u32)
}

fn text(function: &'static str, args: &[LibValue], i: usize) -> Result<String, LibError> {
    match args.get(i) {
        Some(LibValue::Str(t)) => Ok(t.clone()),
        _ => Err(LibError::new(function, format!("argument {} must be a string", i + 1))),
    }
}

fn int(function: &'static str, args: &[LibValue], i: usize) -> Result<i64, LibError> {
    match args.get(i) {
        Some(LibValue::Int(v)) => Ok(*v),
        _ => Err(LibError::new(function, format!("argument {} must be an int", i + 1))),
    }
}

fn float(function: &'static str, args: &[LibValue], i: usize) -> Result<f64, LibError> {
    match args.get(i) {
        Some(LibValue::Float(v)) => Ok(*v),
        _ => Err(LibError::new(function, format!("argument {} must be a float", i + 1))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx() -> CallCtx<'static> {
        CallCtx { caller_file: None, caller_line: 0 }
    }

    fn s(v: &str) -> LibValue {
        LibValue::Str(v.to_string())
    }

    fn key(name: &str) -> LibValue {
        LibValue::Enum(name.to_string())
    }

    /// opens a window with the test backend and returns its `Window` value
    fn open(title: &str, w: i64, h: i64) -> LibValue {
        window(&ctx(), vec![s(title), LibValue::Int(w), LibValue::Int(h)]).unwrap()
    }

    fn float_of(v: LibValue) -> f64 {
        match v {
            LibValue::Float(f) => f,
            other => panic!("expected a float, got {other:?}"),
        }
    }

    #[test]
    fn the_declared_key_enum_lists_exactly_the_core_keys() {
        let declared: Vec<&str> = KEY_ENUM
            .trim_start_matches("enum Key { ")
            .trim_end_matches(" }")
            .split(", ")
            .collect();
        let core: Vec<String> = Key::ALL.iter().map(|k| format!("{k:?}")).collect();
        assert_eq!(declared.len(), core.len());
        for (spg, rust) in declared.iter().zip(core.iter()) {
            assert_eq!(spg, rust);
        }
    }

    #[test]
    fn colors_come_back_as_ints() {
        assert_eq!(hex(&ctx(), vec![s("#ff3300")]).unwrap(), LibValue::Int(0xff3300));
        assert_eq!(rgb(&ctx(), vec![LibValue::Int(255), LibValue::Int(51), LibValue::Int(0)]).unwrap(), LibValue::Int(0xff3300));
        assert!(hex(&ctx(), vec![s("nope")]).is_err());
        assert!(rgb(&ctx(), vec![LibValue::Int(300), LibValue::Int(0), LibValue::Int(0)]).is_err());
    }

    #[test]
    fn a_window_opens_and_the_loop_runs_then_ends() {
        let win = open("loop", 32, 32);
        assert_eq!(float_of(next_frame(&ctx(), vec![win.clone()]).unwrap()), 0.0);
        clear(&ctx(), vec![win.clone(), LibValue::Int(0x101010)]).unwrap();
        rect(&ctx(), vec![win.clone(), LibValue::Float(2.0), LibValue::Float(2.0), LibValue::Float(4.0), LibValue::Float(4.0), LibValue::Int(0xff3300)]).unwrap();
        close(&ctx(), vec![win.clone()]).unwrap();
        assert_eq!(float_of(next_frame(&ctx(), vec![win.clone()]).unwrap()), -1.0);
        assert!(clear(&ctx(), vec![win.clone(), LibValue::Int(0)]).is_err());
    }

    #[test]
    fn keys_are_read_as_enum_values() {
        let win = open("keys", 16, 16);
        next_frame(&ctx(), vec![win.clone()]).unwrap();
        assert_eq!(is_down(&ctx(), vec![win.clone(), key("Left")]).unwrap(), LibValue::Bool(false));
        assert_eq!(was_pressed(&ctx(), vec![win.clone(), key("Space")]).unwrap(), LibValue::Bool(false));
        assert_eq!(was_released(&ctx(), vec![win.clone(), key("Space")]).unwrap(), LibValue::Bool(false));
        assert!(is_down(&ctx(), vec![win.clone(), key("Nope")]).is_err(), "not a key of the enum");
        assert!(is_down(&ctx(), vec![win.clone(), s("Left")]).is_err(), "a string is not a Key");
    }

    #[test]
    fn bad_sizes_are_refused_before_a_window_opens() {
        assert!(window(&ctx(), vec![s("x"), LibValue::Int(0), LibValue::Int(10)]).is_err());
        assert!(window(&ctx(), vec![s("x"), LibValue::Int(10), LibValue::Int(MAX_SIDE + 1)]).is_err());
    }

    #[test]
    fn a_color_out_of_range_is_refused() {
        let win = open("color", 8, 8);
        next_frame(&ctx(), vec![win.clone()]).unwrap();
        assert!(clear(&ctx(), vec![win.clone(), LibValue::Int(-1)]).is_err());
        assert!(clear(&ctx(), vec![win.clone(), LibValue::Int(0x1_000_000)]).is_err());
    }

    #[test]
    fn a_value_that_is_not_a_window_is_refused() {
        assert!(next_frame(&ctx(), vec![LibValue::Int(1)]).is_err());
        let fake = LibValue::record("Window", vec![("id", LibValue::Int(999_999))]);
        assert!(next_frame(&ctx(), vec![fake]).is_err(), "no such window");
    }
}
