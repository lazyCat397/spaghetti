//! `ui`: a window with a scene of rectangles, and the keyboard (`install #ui`). the core is in
//! `ui-core`; this file is the bridge to the program's values.
//!
//! the program holds two small handles, `Window` and `Shape`. their fields and methods are this
//! library's (see `objects` below): `win.background = ...`, `red.x`, `red.kill()`. the engine
//! turns each use into a call of the functions here (`red.x = 1.0` is `ui._setX(red, 1.0)`), so
//! every read gets the current value and every write lands at once.
//!
//! a library can not call the program's functions, so there is no `run(callback)`: the
//! program writes its own loop and asks the window for the time of each frame:
//!
//! ```text
//! install #ui
//!
//! let win = ui.window("demo", 800, 600)
//! win.background = ui.hex("#101010")
//! let red = win.rect(100.0, 100.0, 40.0, 40.0, ui.hex("#ff3300"))
//! while (true) {
//!     let dt = win.nextFrame()
//!     if (dt < 0.0) { break }              // closed: Esc, the X button or win.close()
//!     if (win.isDown(Key.Right)) { red.x = red.x + 200.0 * dt }
//!     if (win.wasPressed(Key.Space)) { red.color = ui.hex("#00ff00") }
//! }
//! ```
//!
//! the windows live here, one set per thread, and the program holds the id in its `Window`.
//! `SPG_UI_BACKEND=null` opens no real window (for tests and machines with no display).

use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::OnceLock;
use std::time::Instant;

use ui_core::{color, Backend, Key, Session, Shape};

use crate::{CallCtx, Kind, LibError, LibValue, NativeField, NativeFn, NativeModule, NativeObject};

/// the largest side a window may have, in pixels
const MAX_SIDE: i64 = 8192;

/// the `Key` enum the program sees. it lists the same names as `ui_core::Key`; a test checks
/// that the two stay equal
const MOUSE_ENUM: &str = "enum Mouse { Left, Right, Middle }";

const ALIGN_ENUM: &str = "enum Align { Left, Center, Right }";

const KEY_ENUM: &str = "enum Key { A, B, C, D, E, F, G, H, I, J, K, L, M, N, O, P, Q, R, S, T, U, V, W, X, Y, Z, Num0, Num1, Num2, Num3, Num4, Num5, Num6, Num7, Num8, Num9, F1, F2, F3, F4, F5, F6, F7, F8, F9, F10, F11, F12, Left, Right, Up, Down, Space, Enter, Escape, Tab, Backspace, Delete, Home, End, LeftShift, RightShift, LeftCtrl, RightCtrl, LeftAlt, RightAlt }";

pub static MODULE: NativeModule = NativeModule {
    kind: Kind::Install,
    name: "ui",
    doc: "a window with a scene of rectangles, and the keyboard (install #ui)",
    structs: &["struct Window {\n    id: int\n}\n", "struct Shape {\n    id: int,\n    win: int\n}\n", "struct Label {\n    id: int,\n    win: int\n}\n", KEY_ENUM, MOUSE_ENUM, ALIGN_ENUM],
    objects: &[
        NativeObject {
            name: "Window",
            fields: &[
                NativeField { name: "background", get: "_getBackground", set: Some("_setBackground") },
                NativeField { name: "width", get: "_getWidth", set: None },
                NativeField { name: "height", get: "_getHeight", set: None },
                NativeField { name: "mouseX", get: "_getMouseX", set: None },
                NativeField { name: "mouseY", get: "_getMouseY", set: None },
                NativeField { name: "mouseInside", get: "_getMouseInside", set: None },
                NativeField { name: "typed", get: "_getTyped", set: None },
            ],
            methods: &[("rect", "rect"), ("nextFrame", "nextFrame"), ("isDown", "isDown"), ("wasPressed", "wasPressed"), ("wasReleased", "wasReleased"), ("close", "close"), ("onClose", "onClose"), ("onFocus", "onFocus"), ("onBlur", "onBlur"), ("onResize", "onResize"), ("mouseDown", "mouseDown"), ("mousePressed", "mousePressed"), ("mouseReleased", "mouseReleased"), ("text", "text")],
        },
        NativeObject {
            name: "Label",
            fields: &[
                NativeField { name: "x", get: "_getLabelX", set: Some("_setLabelX") },
                NativeField { name: "y", get: "_getLabelY", set: Some("_setLabelY") },
                NativeField { name: "text", get: "_getLabelText", set: Some("_setLabelText") },
                NativeField { name: "font", get: "_getLabelFont", set: Some("_setLabelFont") },
                NativeField { name: "color", get: "_getLabelColor", set: Some("_setLabelColor") },
                NativeField { name: "scale", get: "_getLabelScale", set: Some("_setLabelScale") },
                NativeField { name: "visible", get: "_getLabelVisible", set: Some("_setLabelVisible") },
                NativeField { name: "layer", get: "_getLabelLayer", set: Some("_setLabelLayer") },
                NativeField { name: "align", get: "_getLabelAlign", set: Some("_setLabelAlign") },
                NativeField { name: "wrap", get: "_getLabelWrap", set: Some("_setLabelWrap") },
                NativeField { name: "width", get: "_getLabelWidth", set: None },
                NativeField { name: "height", get: "_getLabelHeight", set: None },
            ],
            methods: &[("kill", "labelKill")],
        },
        NativeObject {
            name: "Shape",
            fields: &[
                NativeField { name: "x", get: "_getX", set: Some("_setX") },
                NativeField { name: "y", get: "_getY", set: Some("_setY") },
                NativeField { name: "w", get: "_getW", set: Some("_setW") },
                NativeField { name: "h", get: "_getH", set: Some("_setH") },
                NativeField { name: "color", get: "_getColor", set: Some("_setColor") },
                NativeField { name: "visible", get: "_getVisible", set: Some("_setVisible") },
                NativeField { name: "layer", get: "_getLayer", set: Some("_setLayer") },
                NativeField { name: "clickThrough", get: "_getClickThrough", set: Some("_setClickThrough") },
                NativeField { name: "hovered", get: "_getHovered", set: None },
            ],
            methods: &[
                ("kill", "kill"),
                ("mouseDown", "shapeMouseDown"),
                ("mousePressed", "shapeMousePressed"),
                ("mouseReleased", "shapeMouseReleased"),
            ],
        },
    ],
    functions: &[
        NativeFn { name: "text", sig: "(Window, float, float, string, int, int) -> Label", doc: "adds a line of text to the scene: x, y, the text, a color, and a scale from 1 to 8 (a character is 8 pixels at scale 1). a \\n starts a new line. the window draws it on every frame until it is killed", call: add_text },
        NativeFn { name: "labelKill", sig: "(Label) -> void", doc: "removes the label from the scene. killing it again does nothing", call: label_kill },
        NativeFn { name: "_getTyped", sig: "(Window) -> string", doc: "internal: win.typed", call: get_typed },
        NativeFn { name: "_getLabelFont", sig: "(Label) -> string", doc: "internal: label.font", call: get_label_font },
        NativeFn { name: "_setLabelFont", sig: "(Label, string) -> void", doc: "internal: label.font = value", call: set_label_font },
        NativeFn { name: "_getLabelWidth", sig: "(Label) -> float", doc: "internal: label.width", call: get_label_width },
        NativeFn { name: "_getLabelAlign", sig: "(Label) -> Align", doc: "internal: label.align", call: get_label_align },
        NativeFn { name: "_setLabelAlign", sig: "(Label, Align) -> void", doc: "internal: label.align = value", call: set_label_align },
        NativeFn { name: "_getLabelWrap", sig: "(Label) -> float", doc: "internal: label.wrap", call: get_label_wrap },
        NativeFn { name: "_setLabelWrap", sig: "(Label, float) -> void", doc: "internal: label.wrap = value", call: set_label_wrap },
        NativeFn { name: "_getLabelHeight", sig: "(Label) -> float", doc: "internal: label.height", call: get_label_height },
        NativeFn { name: "_getLabelX", sig: "(Label) -> float", doc: "internal: label.x", call: get_label_x },
        NativeFn { name: "_setLabelX", sig: "(Label, float) -> void", doc: "internal: label.x = value", call: set_label_x },
        NativeFn { name: "_getLabelY", sig: "(Label) -> float", doc: "internal: label.y", call: get_label_y },
        NativeFn { name: "_setLabelY", sig: "(Label, float) -> void", doc: "internal: label.y = value", call: set_label_y },
        NativeFn { name: "_getLabelText", sig: "(Label) -> string", doc: "internal: label.text", call: get_label_text },
        NativeFn { name: "_setLabelText", sig: "(Label, string) -> void", doc: "internal: label.text = value", call: set_label_text },
        NativeFn { name: "_getLabelColor", sig: "(Label) -> int", doc: "internal: label.color", call: get_label_color },
        NativeFn { name: "_setLabelColor", sig: "(Label, int) -> void", doc: "internal: label.color = value", call: set_label_color },
        NativeFn { name: "_getLabelScale", sig: "(Label) -> int", doc: "internal: label.scale", call: get_label_scale },
        NativeFn { name: "_setLabelScale", sig: "(Label, int) -> void", doc: "internal: label.scale = value", call: set_label_scale },
        NativeFn { name: "_getLabelVisible", sig: "(Label) -> bool", doc: "internal: label.visible", call: get_label_visible },
        NativeFn { name: "_setLabelVisible", sig: "(Label, bool) -> void", doc: "internal: label.visible = value", call: set_label_visible },
        NativeFn { name: "_getLabelLayer", sig: "(Label) -> int", doc: "internal: label.layer", call: get_label_layer },
        NativeFn { name: "_setLabelLayer", sig: "(Label, int) -> void", doc: "internal: label.layer = value", call: set_label_layer },
        NativeFn { name: "window", sig: "(string, int, int) -> Window", doc: "opens a window of the given size, in pixels. it shows when the loop starts", call: window },
        NativeFn { name: "nextFrame", sig: "(Window) -> float", doc: "draws the scene and shows it, reads the keys and gives the seconds since the last frame (at most 0.1). gives -1.0 when the window is closing, which ends the loop", call: next_frame },
        NativeFn { name: "rect", sig: "(Window, float, float, float, float, int) -> Shape", doc: "adds a filled rectangle to the scene: x, y, width, height in pixels, then a color. the window draws it on every frame until it is killed", call: rect },
        NativeFn { name: "kill", sig: "(Shape) -> void", doc: "removes the shape from the scene. killing it again does nothing; using it afterwards is an error", call: kill },
        NativeFn { name: "isDown", sig: "(Window, Key) -> bool", doc: "true on every frame the key is held", call: is_down },
        NativeFn { name: "wasPressed", sig: "(Window, Key) -> bool", doc: "true on the one frame the key went down. a tap shorter than one frame can be missed", call: was_pressed },
        NativeFn { name: "wasReleased", sig: "(Window, Key) -> bool", doc: "true on the one frame the key went up", call: was_released },
        NativeFn { name: "close", sig: "(Window) -> void", doc: "asks the window to close. the frame in progress finishes, then the loop ends", call: close },
        NativeFn { name: "onClose", sig: "(Window, function() -> void) -> void", doc: "runs the function when the window closes (the X, Esc, or win.close()). replaces the one set before", call: on_close },
        NativeFn { name: "onFocus", sig: "(Window, function() -> void) -> void", doc: "runs the function when the window gains the focus of the system. replaces the one set before", call: on_focus },
        NativeFn { name: "onBlur", sig: "(Window, function() -> void) -> void", doc: "runs the function when the window loses the focus of the system. replaces the one set before", call: on_blur },
        NativeFn { name: "onResize", sig: "(Window, function(int, int) -> void) -> void", doc: "runs the function with the new width and height of the window, in pixels, when the user resizes it. replaces the one set before", call: on_resize },
        NativeFn { name: "hex", sig: "(string) -> int", doc: "a color from \"#rrggbb\"", call: hex },
        NativeFn { name: "textWidth", sig: "(string, int, float) -> float", doc: "the width in pixels of a text at a scale from 1 to 8, wrapped to the given width (0 means no wrap). the widest line counts. the same numbers a label would report", call: text_width },
        NativeFn { name: "textHeight", sig: "(string, int, float) -> float", doc: "the height in pixels of a text at a scale from 1 to 8, wrapped to the given width (0 means no wrap). counts every line after wrapping", call: text_height },
        NativeFn { name: "rgb", sig: "(int, int, int) -> int", doc: "a color from red, green and blue, each from 0 to 255", call: rgb },
        // what `win.field` and `shape.field` call. not meant to be written by hand
        NativeFn { name: "mouseDown", sig: "(Window, Mouse) -> bool", doc: "true on every frame the mouse button is held", call: mouse_down },
        NativeFn { name: "mousePressed", sig: "(Window, Mouse) -> bool", doc: "true on the one frame the mouse button went down", call: mouse_pressed },
        NativeFn { name: "mouseReleased", sig: "(Window, Mouse) -> bool", doc: "true on the one frame the mouse button went up", call: mouse_released },
        NativeFn { name: "shapeMouseDown", sig: "(Shape, Mouse) -> bool", doc: "true on every frame the shape holds the mouse button down", call: shape_mouse_down },
        NativeFn { name: "shapeMousePressed", sig: "(Shape, Mouse) -> bool", doc: "true on the one frame the shape takes a press of the mouse button", call: shape_mouse_pressed },
        NativeFn { name: "shapeMouseReleased", sig: "(Shape, Mouse) -> bool", doc: "true on the one frame the shape gives back the mouse button", call: shape_mouse_released },
        NativeFn { name: "_getClickThrough", sig: "(Shape) -> bool", doc: "internal: shape.clickThrough", call: get_click_through },
        NativeFn { name: "_setClickThrough", sig: "(Shape, bool) -> void", doc: "internal: shape.clickThrough = value", call: set_click_through },
        NativeFn { name: "_getHovered", sig: "(Shape) -> bool", doc: "internal: shape.hovered", call: get_hovered },
        NativeFn { name: "_getMouseX", sig: "(Window) -> float", doc: "internal: win.mouseX", call: get_mouse_x },
        NativeFn { name: "_getMouseY", sig: "(Window) -> float", doc: "internal: win.mouseY", call: get_mouse_y },
        NativeFn { name: "_getMouseInside", sig: "(Window) -> bool", doc: "internal: win.mouseInside", call: get_mouse_inside },
        NativeFn { name: "_getBackground", sig: "(Window) -> int", doc: "internal: win.background", call: get_background },
        NativeFn { name: "_setBackground", sig: "(Window, int) -> void", doc: "internal: win.background = color", call: set_background },
        NativeFn { name: "_getWidth", sig: "(Window) -> int", doc: "internal: win.width", call: get_width },
        NativeFn { name: "_getHeight", sig: "(Window) -> int", doc: "internal: win.height", call: get_height },
        NativeFn { name: "_getX", sig: "(Shape) -> float", doc: "internal: shape.x", call: get_x },
        NativeFn { name: "_setX", sig: "(Shape, float) -> void", doc: "internal: shape.x = value", call: set_x },
        NativeFn { name: "_getY", sig: "(Shape) -> float", doc: "internal: shape.y", call: get_y },
        NativeFn { name: "_setY", sig: "(Shape, float) -> void", doc: "internal: shape.y = value", call: set_y },
        NativeFn { name: "_getW", sig: "(Shape) -> float", doc: "internal: shape.w", call: get_w },
        NativeFn { name: "_setW", sig: "(Shape, float) -> void", doc: "internal: shape.w = value", call: set_w },
        NativeFn { name: "_getH", sig: "(Shape) -> float", doc: "internal: shape.h", call: get_h },
        NativeFn { name: "_setH", sig: "(Shape, float) -> void", doc: "internal: shape.h = value", call: set_h },
        NativeFn { name: "_getColor", sig: "(Shape) -> int", doc: "internal: shape.color", call: get_color },
        NativeFn { name: "_setColor", sig: "(Shape, int) -> void", doc: "internal: shape.color = color", call: set_color },
        NativeFn { name: "_getVisible", sig: "(Shape) -> bool", doc: "internal: shape.visible", call: get_visible },
        NativeFn { name: "_setVisible", sig: "(Shape, bool) -> void", doc: "internal: shape.visible = value", call: set_visible },
        NativeFn { name: "_getLayer", sig: "(Shape) -> int", doc: "internal: shape.layer", call: get_layer },
        NativeFn { name: "_setLayer", sig: "(Shape, int) -> void", doc: "internal: shape.layer = value", call: set_layer },
    ],
};

/// an open window: the core session, over whichever backend this build uses
type OpenWindow = Session<Box<dyn Backend>>;

/// the handlers a window was given (`win.onClose(...)`), by the handler numbers of the VM
#[derive(Default, Clone, Copy)]
struct Handlers {
    close: Option<u64>,
    focus: Option<u64>,
    blur: Option<u64>,
    resize: Option<u64>,
}

#[derive(Default)]
struct Registry {
    last_id: i64,
    windows: HashMap<i64, OpenWindow>,
    handlers: HashMap<i64, Handlers>,
}

thread_local! {
    static REGISTRY: RefCell<Registry> = RefCell::new(Registry::default());
}

#[cfg(test)]
thread_local! {
    /// the focus and size of the last test window opened on this thread
    static TEST_HANDLES: std::cell::RefCell<Option<TestHandles>> = const { std::cell::RefCell::new(None) };
}

#[cfg(test)]
struct TestHandles {
    focus: std::rc::Rc<std::cell::Cell<bool>>,
    size: std::rc::Rc<std::cell::Cell<(usize, usize)>>,
    mouse: std::rc::Rc<std::cell::RefCell<ui_core::null::NullMouse>>,
}

/// lets a test change the focus of the window it opened
#[cfg(test)]
fn share_focus(backend: &ui_core::null::NullBackend) {
    TEST_HANDLES.with(|handles| {
        *handles.borrow_mut() = Some(TestHandles {
            focus: backend.focus_handle(),
            size: backend.size_handle(),
            mouse: backend.mouse_handle(),
        })
    });
}

#[cfg(not(test))]
fn share_focus(_backend: &ui_core::null::NullBackend) {}

/// the real window, unless the tests (or `SPG_UI_BACKEND=null`) ask for none
fn new_backend() -> Box<dyn Backend> {
    if cfg!(test) || std::env::var("SPG_UI_BACKEND").as_deref() == Ok("null") {
        let backend = ui_core::null::NullBackend::new();
        share_focus(&backend);
        Box::new(backend)
    } else {
        Box::new(ui_core::minifb_backend::MinifbBackend::default())
    }
}

/// seconds since the first call. the clock of the frames
fn now() -> f64 {
    static START: OnceLock<Instant> = OnceLock::new();
    START.get_or_init(Instant::now).elapsed().as_secs_f64()
}

// --- the window -----------------------------------------------------------------

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
    Ok(LibValue::record("Window", vec![("id", LibValue::Int(id))]))
}

fn next_frame(_: &CallCtx, args: Vec<LibValue>) -> Result<LibValue, LibError> {
    const F: &str = "ui.nextFrame";
    let id = window_id(F, &args)?;
    let (dt, focus_changes, resizes) = with_window(F, id, |w| {
        let dt = w.begin_frame(now())?;
        Ok((dt, w.take_focus_changes(), w.take_resizes()))
    })?;
    // the handlers the frame earned run after this call returns, in this order: focus, resize, close
    REGISTRY.with(|registry| {
        let mut registry = registry.borrow_mut();
        let Some(handlers) = registry.handlers.get_mut(&id) else { return };
        for gained in focus_changes {
            let slot = if gained { &mut handlers.focus } else { &mut handlers.blur };
            if let Some(handler) = *slot {
                crate::queue_handler(handler, Vec::new());
            }
        }
        if let Some(handler) = handlers.resize {
            for (width, height) in resizes {
                crate::queue_handler(handler, vec![LibValue::Int(width as i64), LibValue::Int(height as i64)]);
            }
        }
        if dt.is_none() {
            // taken once: the window is closed for good, so its close handler runs once
            if let Some(handler) = handlers.close.take() {
                crate::queue_handler(handler, Vec::new());
            }
        }
    });
    Ok(LibValue::Float(dt.unwrap_or(-1.0)))
}

/// the handler a program passed to `win.onClose(...)`, `onFocus` or `onBlur`, kept for the window
fn set_handler(
    function: &'static str,
    args: &[LibValue],
    choose: fn(&mut Handlers) -> &mut Option<u64>,
) -> Result<LibValue, LibError> {
    let id = window_id(function, args)?;
    let handler = match args.get(1) {
        Some(LibValue::Handler(handler)) => *handler,
        _ => return Err(LibError::new(function, "argument 2 must be a function with no parameters that returns nothing")),
    };
    with_window(function, id, |_| Ok(()))?;
    REGISTRY.with(|registry| {
        let mut registry = registry.borrow_mut();
        *choose(registry.handlers.entry(id).or_default()) = Some(handler);
    });
    Ok(LibValue::Void)
}

fn on_close(_: &CallCtx, args: Vec<LibValue>) -> Result<LibValue, LibError> {
    set_handler("ui.onClose", &args, |h| &mut h.close)
}

fn on_focus(_: &CallCtx, args: Vec<LibValue>) -> Result<LibValue, LibError> {
    set_handler("ui.onFocus", &args, |h| &mut h.focus)
}

fn on_blur(_: &CallCtx, args: Vec<LibValue>) -> Result<LibValue, LibError> {
    set_handler("ui.onBlur", &args, |h| &mut h.blur)
}

fn on_resize(_: &CallCtx, args: Vec<LibValue>) -> Result<LibValue, LibError> {
    set_handler("ui.onResize", &args, |h| &mut h.resize)
}

fn close(_: &CallCtx, args: Vec<LibValue>) -> Result<LibValue, LibError> {
    const F: &str = "ui.close";
    let id = window_id(F, &args)?;
    with_window(F, id, |w| {
        w.request_close();
        Ok(())
    })?;
    Ok(LibValue::Void)
}

fn is_down(_: &CallCtx, args: Vec<LibValue>) -> Result<LibValue, LibError> {
    const F: &str = "ui.isDown";
    let id = window_id(F, &args)?;
    let key = key_arg(F, &args, 1)?;
    Ok(LibValue::Bool(with_window(F, id, |w| Ok(w.is_down(key)))?))
}

fn was_pressed(_: &CallCtx, args: Vec<LibValue>) -> Result<LibValue, LibError> {
    const F: &str = "ui.wasPressed";
    let id = window_id(F, &args)?;
    let key = key_arg(F, &args, 1)?;
    Ok(LibValue::Bool(with_window(F, id, |w| Ok(w.was_pressed(key)))?))
}

fn was_released(_: &CallCtx, args: Vec<LibValue>) -> Result<LibValue, LibError> {
    const F: &str = "ui.wasReleased";
    let id = window_id(F, &args)?;
    let key = key_arg(F, &args, 1)?;
    Ok(LibValue::Bool(with_window(F, id, |w| Ok(w.was_released(key)))?))
}

fn mouse_down(_: &CallCtx, args: Vec<LibValue>) -> Result<LibValue, LibError> {
    const F: &str = "Window.mouseDown";
    let id = window_id(F, &args)?;
    let button = button_arg(F, &args, 1)?;
    Ok(LibValue::Bool(with_window(F, id, |w| Ok(w.button_down(button)))?))
}

fn mouse_pressed(_: &CallCtx, args: Vec<LibValue>) -> Result<LibValue, LibError> {
    const F: &str = "Window.mousePressed";
    let id = window_id(F, &args)?;
    let button = button_arg(F, &args, 1)?;
    Ok(LibValue::Bool(with_window(F, id, |w| Ok(w.button_pressed(button)))?))
}

fn mouse_released(_: &CallCtx, args: Vec<LibValue>) -> Result<LibValue, LibError> {
    const F: &str = "Window.mouseReleased";
    let id = window_id(F, &args)?;
    let button = button_arg(F, &args, 1)?;
    Ok(LibValue::Bool(with_window(F, id, |w| Ok(w.button_released(button)))?))
}

/// the `Label` at argument 0 (its window and its id), checked: the label must be alive
fn label_handle(function: &'static str, args: &[LibValue]) -> Result<(i64, u64), LibError> {
    let win = handle_field(function, args, "Label", "win")?;
    let id = handle_field(function, args, "Label", "id")?;
    Ok((win, id as u64))
}

/// runs `f` on the label at argument 0
fn with_label<T>(
    function: &'static str,
    args: &[LibValue],
    f: impl FnOnce(&mut ui_core::Label) -> Result<T, String>,
) -> Result<T, LibError> {
    let (win, id) = label_handle(function, args)?;
    with_window(function, win, |w| f(w.label_mut(id)?))
}

fn add_text(_: &CallCtx, args: Vec<LibValue>) -> Result<LibValue, LibError> {
    const F: &str = "ui.text";
    let win = window_id(F, &args)?;
    let x = float(F, &args, 1)?;
    let y = float(F, &args, 2)?;
    let content = text_arg(F, &args, 3)?;
    let color = color_arg(F, &args, 4)?;
    let scale = int(F, &args, 5)?;
    let id = with_window(F, win, |w| w.add_label(x, y, &content, color, scale))?;
    Ok(LibValue::record("Label", vec![("id", LibValue::Int(id as i64)), ("win", LibValue::Int(win))]))
}

fn label_kill(_: &CallCtx, args: Vec<LibValue>) -> Result<LibValue, LibError> {
    const F: &str = "ui.labelKill";
    let (win, id) = label_handle(F, &args)?;
    with_window(F, win, |w| {
        w.kill_shape(id);
        Ok(())
    })?;
    Ok(LibValue::Void)
}

fn get_typed(_: &CallCtx, args: Vec<LibValue>) -> Result<LibValue, LibError> {
    const F: &str = "Window.typed";
    let id = window_id(F, &args)?;
    Ok(LibValue::Str(with_window(F, id, |w| Ok(w.typed().to_string()))?))
}

fn get_label_align(_: &CallCtx, args: Vec<LibValue>) -> Result<LibValue, LibError> {
    const F: &str = "Label.align";
    with_label(F, &args, |l| {
        let variant = match l.align() {
            ui_core::Align::Left => "Left",
            ui_core::Align::Center => "Center",
            ui_core::Align::Right => "Right",
        };
        Ok(LibValue::Variant("Align".to_string(), variant.to_string()))
    })
}

fn set_label_align(_: &CallCtx, args: Vec<LibValue>) -> Result<LibValue, LibError> {
    const F: &str = "Label.align";
    let value = match args.get(1) {
        Some(LibValue::Enum(name)) => match name.as_str() {
            "Left" => ui_core::Align::Left,
            "Center" => ui_core::Align::Center,
            "Right" => ui_core::Align::Right,
            other => return Err(LibError::new(F, format!("unknown alignment {other}"))),
        },
        _ => return Err(LibError::new(F, "argument 2 must be an Align, like Align.Center")),
    };
    with_label(F, &args, |l| {
        l.set_align(value);
        Ok(())
    })?;
    Ok(LibValue::Void)
}

fn get_label_wrap(_: &CallCtx, args: Vec<LibValue>) -> Result<LibValue, LibError> {
    const F: &str = "Label.wrap";
    with_label(F, &args, |l| Ok(LibValue::Float(l.wrap())))
}

fn set_label_wrap(_: &CallCtx, args: Vec<LibValue>) -> Result<LibValue, LibError> {
    const F: &str = "Label.wrap";
    let value = float(F, &args, 1)?;
    with_label(F, &args, |l| l.set_wrap(value))?;
    Ok(LibValue::Void)
}

/// the path of the font file a label draws with, or `""` for the bitmap font
fn get_label_font(_: &CallCtx, args: Vec<LibValue>) -> Result<LibValue, LibError> {
    const F: &str = "Label.font";
    with_label(F, &args, |l| Ok(LibValue::Str(l.font().path().to_string())))
}

/// `label.font = "./file.ttf"` loads an outline font, relative to the script that sets it.
/// `label.font = ""` goes back to the bitmap font
fn set_label_font(ctx: &CallCtx, args: Vec<LibValue>) -> Result<LibValue, LibError> {
    const F: &str = "Label.font";
    let path = text_arg(F, &args, 1)?;
    let font = if path.is_empty() {
        ui_core::text::Font::Bitmap
    } else {
        let path_buf = std::path::Path::new(&path);
        let resolved = if path_buf.is_absolute() {
            path_buf.to_path_buf()
        } else {
            ctx.caller_file
                .map(|file| std::path::Path::new(file).parent().unwrap_or(std::path::Path::new(".")).join(path_buf))
                .unwrap_or_else(|| path_buf.to_path_buf())
        };
        let bytes = std::fs::read(&resolved)
            .map_err(|e| LibError::new(F, format!("cannot read the font \"{path}\": {e}")))?;
        ui_core::text::Font::load(&path, &bytes).map_err(|e| LibError::new(F, e))?
    };
    with_label(F, &args, |l| {
        l.set_font(font);
        Ok(())
    })?;
    Ok(LibValue::Void)
}

fn get_label_width(_: &CallCtx, args: Vec<LibValue>) -> Result<LibValue, LibError> {
    const F: &str = "Label.width";
    with_label(F, &args, |l| Ok(LibValue::Float(l.size()?.0)))
}

fn get_label_height(_: &CallCtx, args: Vec<LibValue>) -> Result<LibValue, LibError> {
    const F: &str = "Label.height";
    with_label(F, &args, |l| Ok(LibValue::Float(l.size()?.1)))
}

fn get_label_x(_: &CallCtx, args: Vec<LibValue>) -> Result<LibValue, LibError> {
    const F: &str = "Label.x";
    with_label(F, &args, |l| Ok(LibValue::Float(l.x())))
}

fn set_label_x(_: &CallCtx, args: Vec<LibValue>) -> Result<LibValue, LibError> {
    const F: &str = "Label.x";
    let value = float(F, &args, 1)?;
    with_label(F, &args, |l| l.set_x(value))?;
    Ok(LibValue::Void)
}

fn get_label_y(_: &CallCtx, args: Vec<LibValue>) -> Result<LibValue, LibError> {
    const F: &str = "Label.y";
    with_label(F, &args, |l| Ok(LibValue::Float(l.y())))
}

fn set_label_y(_: &CallCtx, args: Vec<LibValue>) -> Result<LibValue, LibError> {
    const F: &str = "Label.y";
    let value = float(F, &args, 1)?;
    with_label(F, &args, |l| l.set_y(value))?;
    Ok(LibValue::Void)
}

fn get_label_text(_: &CallCtx, args: Vec<LibValue>) -> Result<LibValue, LibError> {
    const F: &str = "Label.text";
    with_label(F, &args, |l| Ok(LibValue::Str(l.text().to_string())))
}

fn set_label_text(_: &CallCtx, args: Vec<LibValue>) -> Result<LibValue, LibError> {
    const F: &str = "Label.text";
    let value = text_arg(F, &args, 1)?;
    with_label(F, &args, |l| l.set_text(&value))?;
    Ok(LibValue::Void)
}

fn get_label_color(_: &CallCtx, args: Vec<LibValue>) -> Result<LibValue, LibError> {
    const F: &str = "Label.color";
    with_label(F, &args, |l| Ok(LibValue::Int(l.color() as i64)))
}

fn set_label_color(_: &CallCtx, args: Vec<LibValue>) -> Result<LibValue, LibError> {
    const F: &str = "Label.color";
    let value = color_arg(F, &args, 1)?;
    with_label(F, &args, |l| {
        l.set_color(value);
        Ok(())
    })?;
    Ok(LibValue::Void)
}

fn get_label_scale(_: &CallCtx, args: Vec<LibValue>) -> Result<LibValue, LibError> {
    const F: &str = "Label.scale";
    with_label(F, &args, |l| Ok(LibValue::Int(l.scale() as i64)))
}

fn set_label_scale(_: &CallCtx, args: Vec<LibValue>) -> Result<LibValue, LibError> {
    const F: &str = "Label.scale";
    let value = int(F, &args, 1)?;
    with_label(F, &args, |l| l.set_scale(value))?;
    Ok(LibValue::Void)
}

fn get_label_visible(_: &CallCtx, args: Vec<LibValue>) -> Result<LibValue, LibError> {
    const F: &str = "Label.visible";
    with_label(F, &args, |l| Ok(LibValue::Bool(l.visible())))
}

fn set_label_visible(_: &CallCtx, args: Vec<LibValue>) -> Result<LibValue, LibError> {
    const F: &str = "Label.visible";
    let value = boolean(F, &args, 1)?;
    with_label(F, &args, |l| {
        l.set_visible(value);
        Ok(())
    })?;
    Ok(LibValue::Void)
}

fn get_label_layer(_: &CallCtx, args: Vec<LibValue>) -> Result<LibValue, LibError> {
    const F: &str = "Label.layer";
    with_label(F, &args, |l| Ok(LibValue::Int(l.layer())))
}

fn set_label_layer(_: &CallCtx, args: Vec<LibValue>) -> Result<LibValue, LibError> {
    const F: &str = "Label.layer";
    let value = int(F, &args, 1)?;
    with_label(F, &args, |l| {
        l.set_layer(value);
        Ok(())
    })?;
    Ok(LibValue::Void)
}

/// a text argument: a string
fn text_arg(function: &'static str, args: &[LibValue], i: usize) -> Result<String, LibError> {
    match args.get(i) {
        Some(LibValue::Str(t)) => Ok(t.clone()),
        _ => Err(LibError::new(function, format!("argument {} must be a string", i + 1))),
    }
}

/// the shape at argument 0 asked about a mouse button: the shape must be alive, and its window open
fn shape_button(
    function: &'static str,
    args: &[LibValue],
    query: fn(&OpenWindow, ui_core::ShapeId, ui_core::Button) -> bool,
) -> Result<LibValue, LibError> {
    let (win, id) = shape_handle(function, args)?;
    let button = button_arg(function, args, 1)?;
    let value = with_window(function, win, |w| {
        w.shape(id)?; // a killed shape is an error, not a quiet false
        Ok(query(w, id, button))
    })?;
    Ok(LibValue::Bool(value))
}

fn shape_mouse_down(_: &CallCtx, args: Vec<LibValue>) -> Result<LibValue, LibError> {
    shape_button("ui.shapeMouseDown", &args, |w, id, b| w.shape_button_down(id, b))
}

fn shape_mouse_pressed(_: &CallCtx, args: Vec<LibValue>) -> Result<LibValue, LibError> {
    shape_button("ui.shapeMousePressed", &args, |w, id, b| w.shape_button_pressed(id, b))
}

fn shape_mouse_released(_: &CallCtx, args: Vec<LibValue>) -> Result<LibValue, LibError> {
    shape_button("ui.shapeMouseReleased", &args, |w, id, b| w.shape_button_released(id, b))
}

fn get_click_through(_: &CallCtx, args: Vec<LibValue>) -> Result<LibValue, LibError> {
    const F: &str = "Shape.clickThrough";
    with_shape(F, &args, |s| Ok(LibValue::Bool(s.click_through())))
}

fn set_click_through(_: &CallCtx, args: Vec<LibValue>) -> Result<LibValue, LibError> {
    const F: &str = "Shape.clickThrough";
    let value = boolean(F, &args, 1)?;
    with_shape(F, &args, |s| {
        s.set_click_through(value);
        Ok(())
    })?;
    Ok(LibValue::Void)
}

fn get_hovered(_: &CallCtx, args: Vec<LibValue>) -> Result<LibValue, LibError> {
    const F: &str = "Shape.hovered";
    let (win, id) = shape_handle(F, &args)?;
    let hovered = with_window(F, win, |w| {
        w.shape(id)?;
        Ok(w.shape_hovered(id))
    })?;
    Ok(LibValue::Bool(hovered))
}

fn get_mouse_x(_: &CallCtx, args: Vec<LibValue>) -> Result<LibValue, LibError> {
    const F: &str = "Window.mouseX";
    let id = window_id(F, &args)?;
    Ok(LibValue::Float(with_window(F, id, |w| Ok(w.mouse_x()))?))
}

fn get_mouse_y(_: &CallCtx, args: Vec<LibValue>) -> Result<LibValue, LibError> {
    const F: &str = "Window.mouseY";
    let id = window_id(F, &args)?;
    Ok(LibValue::Float(with_window(F, id, |w| Ok(w.mouse_y()))?))
}

fn get_mouse_inside(_: &CallCtx, args: Vec<LibValue>) -> Result<LibValue, LibError> {
    const F: &str = "Window.mouseInside";
    let id = window_id(F, &args)?;
    Ok(LibValue::Bool(with_window(F, id, |w| Ok(w.mouse_inside()))?))
}

/// a `Mouse` argument: the enum value, which is the variant's name
fn button_arg(function: &'static str, args: &[LibValue], i: usize) -> Result<ui_core::Button, LibError> {
    use ui_core::Button;
    match args.get(i) {
        Some(LibValue::Enum(name)) => match name.as_str() {
            "Left" => Ok(Button::Left),
            "Right" => Ok(Button::Right),
            "Middle" => Ok(Button::Middle),
            other => Err(LibError::new(function, format!("unknown mouse button {other}"))),
        },
        _ => Err(LibError::new(function, format!("argument {} must be a Mouse, like Mouse.Left", i + 1))),
    }
}

fn get_background(_: &CallCtx, args: Vec<LibValue>) -> Result<LibValue, LibError> {
    const F: &str = "Window.background";
    let id = window_id(F, &args)?;
    let value = with_window(F, id, |w| Ok(w.scene()?.background()))?;
    Ok(LibValue::Int(value as i64))
}

fn set_background(_: &CallCtx, args: Vec<LibValue>) -> Result<LibValue, LibError> {
    const F: &str = "Window.background";
    let id = window_id(F, &args)?;
    let value = color_arg(F, &args, 1)?;
    with_window(F, id, |w| {
        w.scene_mut()?.set_background(value);
        Ok(())
    })?;
    Ok(LibValue::Void)
}

fn get_width(_: &CallCtx, args: Vec<LibValue>) -> Result<LibValue, LibError> {
    const F: &str = "Window.width";
    let id = window_id(F, &args)?;
    Ok(LibValue::Int(with_window(F, id, |w| Ok(w.width()))? as i64))
}

fn get_height(_: &CallCtx, args: Vec<LibValue>) -> Result<LibValue, LibError> {
    const F: &str = "Window.height";
    let id = window_id(F, &args)?;
    Ok(LibValue::Int(with_window(F, id, |w| Ok(w.height()))? as i64))
}

// --- shapes -----------------------------------------------------------------------

fn rect(_: &CallCtx, args: Vec<LibValue>) -> Result<LibValue, LibError> {
    const F: &str = "ui.rect";
    let win = window_id(F, &args)?;
    let x = float(F, &args, 1)?;
    let y = float(F, &args, 2)?;
    let w = float(F, &args, 3)?;
    let h = float(F, &args, 4)?;
    let color = color_arg(F, &args, 5)?;
    let shape = with_window(F, win, |window| window.add_rect(x, y, w, h, color))?;
    Ok(LibValue::record("Shape", vec![("id", LibValue::Int(shape as i64)), ("win", LibValue::Int(win))]))
}

fn kill(_: &CallCtx, args: Vec<LibValue>) -> Result<LibValue, LibError> {
    const F: &str = "ui.kill";
    let (win, id) = shape_handle(F, &args)?;
    with_window(F, win, |w| {
        w.kill_shape(id);
        Ok(())
    })?;
    Ok(LibValue::Void)
}

/// the getter of a `Shape` field: `shape_get!(get_x, "Shape.x", |s| LibValue::Float(s.x()))`
macro_rules! shape_get {
    ($name:ident, $label:literal, |$s:ident| $value:expr) => {
        fn $name(_: &CallCtx, args: Vec<LibValue>) -> Result<LibValue, LibError> {
            const F: &str = $label;
            with_shape(F, &args, |$s| Ok($value))
        }
    };
}

/// the setter of a float field of a `Shape`; the core checks the number
macro_rules! shape_set_float {
    ($name:ident, $label:literal, $setter:ident) => {
        fn $name(_: &CallCtx, args: Vec<LibValue>) -> Result<LibValue, LibError> {
            const F: &str = $label;
            let value = float(F, &args, 1)?;
            with_shape(F, &args, |s| s.$setter(value))?;
            Ok(LibValue::Void)
        }
    };
}

shape_get!(get_x, "Shape.x", |s| LibValue::Float(s.x()));
shape_get!(get_y, "Shape.y", |s| LibValue::Float(s.y()));
shape_get!(get_w, "Shape.w", |s| LibValue::Float(s.w()));
shape_get!(get_h, "Shape.h", |s| LibValue::Float(s.h()));
shape_get!(get_color, "Shape.color", |s| LibValue::Int(s.color() as i64));
shape_get!(get_visible, "Shape.visible", |s| LibValue::Bool(s.visible()));
shape_get!(get_layer, "Shape.layer", |s| LibValue::Int(s.layer()));

shape_set_float!(set_x, "Shape.x", set_x);
shape_set_float!(set_y, "Shape.y", set_y);
shape_set_float!(set_w, "Shape.w", set_w);
shape_set_float!(set_h, "Shape.h", set_h);

fn set_color(_: &CallCtx, args: Vec<LibValue>) -> Result<LibValue, LibError> {
    const F: &str = "Shape.color";
    let value = color_arg(F, &args, 1)?;
    with_shape(F, &args, |s| {
        s.set_color(value);
        Ok(())
    })?;
    Ok(LibValue::Void)
}

fn set_visible(_: &CallCtx, args: Vec<LibValue>) -> Result<LibValue, LibError> {
    const F: &str = "Shape.visible";
    let value = boolean(F, &args, 1)?;
    with_shape(F, &args, |s| {
        s.set_visible(value);
        Ok(())
    })?;
    Ok(LibValue::Void)
}

fn set_layer(_: &CallCtx, args: Vec<LibValue>) -> Result<LibValue, LibError> {
    const F: &str = "Shape.layer";
    let value = int(F, &args, 1)?;
    with_shape(F, &args, |s| {
        s.set_layer(value);
        Ok(())
    })?;
    Ok(LibValue::Void)
}

// --- colors -------------------------------------------------------------------------

/// the size of a text as `ui.textWidth` and `ui.textHeight` take it: the text, its scale and its
/// wrap width, checked the same way a label checks them
fn measure_text(function: &'static str, args: &[LibValue]) -> Result<(f64, f64), LibError> {
    let content = text(function, args, 0)?;
    let scale = ui_core::scene::scale_value(int(function, args, 1)?).map_err(|e| LibError::new(function, e))?;
    let wrap = float(function, args, 2)?;
    if wrap < 0.0 {
        return Err(LibError::new(function, format!("the wrap width must be 0 (no wrap) or more pixels, got {wrap}")));
    }
    ui_core::text::size(&content, &ui_core::text::Font::Bitmap, scale, wrap).map_err(|e| LibError::new(function, e))
}

fn text_width(_: &CallCtx, args: Vec<LibValue>) -> Result<LibValue, LibError> {
    let (width, _) = measure_text("ui.textWidth", &args)?;
    Ok(LibValue::Float(width))
}

fn text_height(_: &CallCtx, args: Vec<LibValue>) -> Result<LibValue, LibError> {
    let (_, height) = measure_text("ui.textHeight", &args)?;
    Ok(LibValue::Float(height))
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

/// the value of the field `name` (an int) inside the struct at argument 0, if it is a `type_name`
fn handle_field(function: &'static str, args: &[LibValue], type_name: &str, name: &str) -> Result<i64, LibError> {
    match args.first() {
        Some(LibValue::Struct { name: found, fields }) if found == type_name => fields
            .iter()
            .find_map(|(field, value)| match value {
                LibValue::Int(n) if field == name => Some(*n),
                _ => None,
            })
            .ok_or_else(|| LibError::new(function, format!("the {type_name} has no {name}"))),
        _ => Err(LibError::new(function, format!("expected a {type_name}, made by ui.window"))),
    }
}

/// the id inside the `Window` at argument 0
fn window_id(function: &'static str, args: &[LibValue]) -> Result<i64, LibError> {
    handle_field(function, args, "Window", "id")
}

/// `(window id, shape id)` from the `Shape` at argument 0
fn shape_handle(function: &'static str, args: &[LibValue]) -> Result<(i64, u64), LibError> {
    let win = handle_field(function, args, "Shape", "win")?;
    let id = handle_field(function, args, "Shape", "id")?;
    Ok((win, id as u64))
}

/// a `Key` argument: the enum value, which is the variant's name
fn key_arg(function: &'static str, args: &[LibValue], i: usize) -> Result<Key, LibError> {
    match args.get(i) {
        Some(LibValue::Enum(name)) => Key::from_name(name)
            .ok_or_else(|| LibError::new(function, format!("unknown key {name}"))),
        _ => Err(LibError::new(function, format!("argument {} must be a Key, like Key.Space", i + 1))),
    }
}

/// runs `f` on the open window `id`. an unknown id is an error
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

/// runs `f` on the shape at argument 0 (killed shapes and closed windows are errors)
fn with_shape<T>(
    function: &'static str,
    args: &[LibValue],
    f: impl FnOnce(&mut Shape) -> Result<T, String>,
) -> Result<T, LibError> {
    let (win, id) = shape_handle(function, args)?;
    with_window(function, win, |w| f(w.shape_mut(id)?))
}

/// a color argument: a whole number from 0 to 0xFFFFFF
fn color_arg(function: &'static str, args: &[LibValue], i: usize) -> Result<u32, LibError> {
    let value = int(function, args, i)?;
    color::check(value).map_err(|e| LibError::new(function, format!("{e}: make one with ui.hex or ui.rgb")))
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

fn boolean(function: &'static str, args: &[LibValue], i: usize) -> Result<bool, LibError> {
    match args.get(i) {
        Some(LibValue::Bool(v)) => Ok(*v),
        _ => Err(LibError::new(function, format!("argument {} must be a bool", i + 1))),
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

    fn f(v: f64) -> LibValue {
        LibValue::Float(v)
    }

    fn i(v: i64) -> LibValue {
        LibValue::Int(v)
    }

    /// opens a window with the test backend and returns its `Window` value
    fn open(title: &str, w: i64, h: i64) -> LibValue {
        window(&ctx(), vec![s(title), i(w), i(h)]).unwrap()
    }

    /// a red rectangle in the window
    fn shape_in(win: &LibValue) -> LibValue {
        rect(&ctx(), vec![win.clone(), f(2.0), f(3.0), f(4.0), f(5.0), i(0xff3300)]).unwrap()
    }

    fn float_of(v: LibValue) -> f64 {
        match v {
            LibValue::Float(x) => x,
            other => panic!("expected a float, got {other:?}"),
        }
    }

    fn message(r: Result<LibValue, LibError>) -> String {
        r.expect_err("expected an error").reason
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
    fn every_object_field_and_method_is_a_function_of_the_module() {
        let has = |name: &str| MODULE.functions.iter().any(|f| f.name == name);
        for object in MODULE.objects {
            assert!(
                MODULE.structs.iter().any(|source| source.contains(&format!("struct {} {{", object.name))),
                "{} is not a declared struct",
                object.name
            );
            for field in object.fields {
                assert!(has(field.get), "{}.{} has no getter {}", object.name, field.name, field.get);
                if let Some(set) = field.set {
                    assert!(has(set), "{}.{} has no setter {}", object.name, field.name, set);
                }
            }
            for (method, function) in object.methods {
                assert!(has(function), "{}.{} is not a function", object.name, method);
            }
        }
    }

    #[test]
    fn colors_come_back_as_ints() {
        assert_eq!(hex(&ctx(), vec![s("#ff3300")]).unwrap(), i(0xff3300));
        assert_eq!(rgb(&ctx(), vec![i(255), i(51), i(0)]).unwrap(), i(0xff3300));
        assert!(hex(&ctx(), vec![s("nope")]).is_err());
        assert!(rgb(&ctx(), vec![i(300), i(0), i(0)]).is_err());
    }

    #[test]
    fn text_is_measured_the_way_a_label_measures_it() {
        let f = LibValue::Float;
        // at scale 2 a character is 16 pixels: "abc" is 48 wide and 16 tall
        assert_eq!(float_of(text_width(&ctx(), vec![s("abc"), i(2), f(0.0)]).unwrap()), 48.0);
        assert_eq!(float_of(text_height(&ctx(), vec![s("abc"), i(2), f(0.0)]).unwrap()), 16.0);
        // a wrap of 32 fits two characters per line: "ab cd" becomes two lines, 32 wide and 32 tall
        assert_eq!(float_of(text_width(&ctx(), vec![s("ab cd"), i(2), f(32.0)]).unwrap()), 32.0);
        assert_eq!(float_of(text_height(&ctx(), vec![s("ab cd"), i(2), f(32.0)]).unwrap()), 32.0);
        // a label reports the same numbers for the same text
        assert!(text_width(&ctx(), vec![s("a"), i(9), f(0.0)]).is_err());
        assert!(text_height(&ctx(), vec![s("a"), i(0), f(0.0)]).is_err());
        assert!(text_width(&ctx(), vec![s("a"), i(1), f(-1.0)]).is_err());
    }

    #[test]
    fn a_window_opens_and_the_loop_runs_then_ends() {
        let win = open("loop", 32, 32);
        assert_eq!(float_of(next_frame(&ctx(), vec![win.clone()]).unwrap()), 0.0);
        close(&ctx(), vec![win.clone()]).unwrap();
        assert_eq!(float_of(next_frame(&ctx(), vec![win.clone()]).unwrap()), -1.0);
    }

    #[test]
    fn the_window_fields_read_and_write() {
        let win = open("fields", 40, 30);
        assert_eq!(get_width(&ctx(), vec![win.clone()]).unwrap(), i(40));
        assert_eq!(get_height(&ctx(), vec![win.clone()]).unwrap(), i(30));
        assert_eq!(get_background(&ctx(), vec![win.clone()]).unwrap(), i(0));
        set_background(&ctx(), vec![win.clone(), i(0x101010)]).unwrap();
        assert_eq!(get_background(&ctx(), vec![win.clone()]).unwrap(), i(0x101010));
        assert!(set_background(&ctx(), vec![win.clone(), i(-1)]).is_err());
        assert!(set_background(&ctx(), vec![win.clone(), i(0x1_000_000)]).is_err());
        assert_eq!(get_background(&ctx(), vec![win.clone()]).unwrap(), i(0x101010), "a bad value changes nothing");
    }

    #[test]
    fn a_shape_starts_with_what_it_was_made_with() {
        let win = open("shape", 20, 20);
        let shape = shape_in(&win);
        assert_eq!(get_x(&ctx(), vec![shape.clone()]).unwrap(), f(2.0));
        assert_eq!(get_y(&ctx(), vec![shape.clone()]).unwrap(), f(3.0));
        assert_eq!(get_w(&ctx(), vec![shape.clone()]).unwrap(), f(4.0));
        assert_eq!(get_h(&ctx(), vec![shape.clone()]).unwrap(), f(5.0));
        assert_eq!(get_color(&ctx(), vec![shape.clone()]).unwrap(), i(0xff3300));
        assert_eq!(get_visible(&ctx(), vec![shape.clone()]).unwrap(), LibValue::Bool(true));
        assert_eq!(get_layer(&ctx(), vec![shape.clone()]).unwrap(), i(0));
    }

    #[test]
    fn every_shape_field_writes_and_reads_back() {
        let win = open("write", 20, 20);
        let shape = shape_in(&win);
        set_x(&ctx(), vec![shape.clone(), f(7.5)]).unwrap();
        set_y(&ctx(), vec![shape.clone(), f(-1.0)]).unwrap();
        set_w(&ctx(), vec![shape.clone(), f(9.0)]).unwrap();
        set_h(&ctx(), vec![shape.clone(), f(0.0)]).unwrap();
        set_color(&ctx(), vec![shape.clone(), i(0x00ff00)]).unwrap();
        set_visible(&ctx(), vec![shape.clone(), LibValue::Bool(false)]).unwrap();
        set_layer(&ctx(), vec![shape.clone(), i(-3)]).unwrap();
        assert_eq!(get_x(&ctx(), vec![shape.clone()]).unwrap(), f(7.5));
        assert_eq!(get_y(&ctx(), vec![shape.clone()]).unwrap(), f(-1.0));
        assert_eq!(get_w(&ctx(), vec![shape.clone()]).unwrap(), f(9.0));
        assert_eq!(get_h(&ctx(), vec![shape.clone()]).unwrap(), f(0.0));
        assert_eq!(get_color(&ctx(), vec![shape.clone()]).unwrap(), i(0x00ff00));
        assert_eq!(get_visible(&ctx(), vec![shape.clone()]).unwrap(), LibValue::Bool(false));
        assert_eq!(get_layer(&ctx(), vec![shape.clone()]).unwrap(), i(-3));
    }

    #[test]
    fn bad_shape_values_are_refused_and_change_nothing() {
        let win = open("bad", 20, 20);
        let shape = shape_in(&win);
        assert_eq!(message(set_x(&ctx(), vec![shape.clone(), f(f64::NAN)])), "x must be a finite number");
        assert!(set_w(&ctx(), vec![shape.clone(), f(-1.0)]).is_err());
        assert!(set_color(&ctx(), vec![shape.clone(), i(-5)]).is_err());
        assert!(rect(&ctx(), vec![win.clone(), f(0.0), f(0.0), f(-2.0), f(1.0), i(0)]).is_err());
        assert_eq!(get_x(&ctx(), vec![shape.clone()]).unwrap(), f(2.0));
        assert_eq!(get_w(&ctx(), vec![shape.clone()]).unwrap(), f(4.0));
        assert_eq!(get_color(&ctx(), vec![shape.clone()]).unwrap(), i(0xff3300));
    }

    #[test]
    fn a_killed_shape_can_not_be_used_and_killing_again_is_fine() {
        let win = open("kill", 20, 20);
        let shape = shape_in(&win);
        kill(&ctx(), vec![shape.clone()]).unwrap();
        assert_eq!(message(get_x(&ctx(), vec![shape.clone()])), "this shape was killed");
        assert_eq!(message(set_x(&ctx(), vec![shape.clone(), f(1.0)])), "this shape was killed");
        kill(&ctx(), vec![shape.clone()]).unwrap();
    }

    #[test]
    fn a_new_shape_does_not_inherit_a_killed_one() {
        let win = open("ids", 20, 20);
        let old = shape_in(&win);
        kill(&ctx(), vec![old.clone()]).unwrap();
        let fresh = shape_in(&win);
        assert!(get_x(&ctx(), vec![old]).is_err());
        assert_eq!(get_x(&ctx(), vec![fresh]).unwrap(), f(2.0));
    }

    #[test]
    fn the_shapes_die_with_their_window() {
        let win = open("die", 20, 20);
        let shape = shape_in(&win);
        close(&ctx(), vec![win.clone()]).unwrap();
        // the frame in progress finishes: the shape still works
        assert_eq!(get_x(&ctx(), vec![shape.clone()]).unwrap(), f(2.0));
        assert_eq!(float_of(next_frame(&ctx(), vec![win.clone()]).unwrap()), -1.0);
        assert_eq!(message(get_x(&ctx(), vec![shape.clone()])), "the window is closed");
        assert_eq!(message(get_background(&ctx(), vec![win.clone()])), "the window is closed");
        assert!(rect(&ctx(), vec![win.clone(), f(0.0), f(0.0), f(1.0), f(1.0), i(0)]).is_err());
        kill(&ctx(), vec![shape]).unwrap();
        close(&ctx(), vec![win.clone()]).unwrap();
        assert_eq!(get_width(&ctx(), vec![win]).unwrap(), i(20), "the size is still known");
    }

    #[test]
    fn two_windows_keep_their_shapes_apart() {
        let a = open("a", 10, 10);
        let b = open("b", 10, 10);
        let in_a = shape_in(&a);
        let in_b = shape_in(&b);
        set_x(&ctx(), vec![in_a.clone(), f(8.0)]).unwrap();
        assert_eq!(get_x(&ctx(), vec![in_b.clone()]).unwrap(), f(2.0));
        close(&ctx(), vec![a.clone()]).unwrap();
        next_frame(&ctx(), vec![a]).unwrap();
        next_frame(&ctx(), vec![b]).unwrap();
        assert!(get_x(&ctx(), vec![in_a]).is_err(), "a is closed");
        assert_eq!(get_x(&ctx(), vec![in_b]).unwrap(), f(2.0), "b is not");
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
        assert!(window(&ctx(), vec![s("x"), i(0), i(10)]).is_err());
        assert!(window(&ctx(), vec![s("x"), i(10), i(MAX_SIDE + 1)]).is_err());
    }

    #[test]
    fn a_value_that_is_not_the_right_handle_is_refused() {
        assert!(next_frame(&ctx(), vec![i(1)]).is_err());
        let fake_window = LibValue::record("Window", vec![("id", i(999_999))]);
        assert_eq!(message(next_frame(&ctx(), vec![fake_window.clone()])), "this window is not open");
        let win = open("types", 8, 8);
        assert!(get_x(&ctx(), vec![win.clone()]).is_err(), "a Window is not a Shape");
        let shape = shape_in(&win);
        assert!(next_frame(&ctx(), vec![shape]).is_err(), "a Shape is not a Window");
        let forged = LibValue::record("Shape", vec![("id", i(1)), ("win", i(999_999))]);
        assert!(get_x(&ctx(), vec![forged]).is_err(), "no such window");
    }
    fn set_focus(focused: bool) {
        TEST_HANDLES.with(|handles| handles.borrow().as_ref().expect("a test window is open").focus.set(focused));
    }

    /// the handler numbers queued, without their arguments
    fn ids() -> Vec<u64> {
        crate::take_handlers().into_iter().map(|(id, _)| id).collect()
    }

    fn press(button: &str) {
        TEST_HANDLES.with(|handles| {
            let handles = handles.borrow();
            let mouse = &handles.as_ref().expect("a test window is open").mouse;
            let button = match button {
                "Left" => ui_core::Button::Left,
                "Right" => ui_core::Button::Right,
                _ => ui_core::Button::Middle,
            };
            mouse.borrow_mut().down.insert(button);
        });
    }

    fn move_to(x: f64, y: f64) {
        TEST_HANDLES.with(|handles| {
            let handles = handles.borrow();
            handles.as_ref().expect("a test window is open").mouse.borrow_mut().position = Some((x, y));
        });
    }

    #[test]
    fn the_mouse_reads_position_and_buttons_by_name() {
        let win = open("mouse", 8, 8);
        next_frame(&ctx(), vec![win.clone()]).unwrap(); // first frame: nothing pressed yet
        move_to(3.0, 5.0);
        press("Right");
        next_frame(&ctx(), vec![win.clone()]).unwrap();
        assert_eq!(float_of(get_mouse_x(&ctx(), vec![win.clone()]).unwrap()), 3.0);
        assert_eq!(float_of(get_mouse_y(&ctx(), vec![win.clone()]).unwrap()), 5.0);
        assert_eq!(get_mouse_inside(&ctx(), vec![win.clone()]).unwrap(), LibValue::Bool(true));
        let right = LibValue::Enum("Right".to_string());
        assert_eq!(mouse_down(&ctx(), vec![win.clone(), right.clone()]).unwrap(), LibValue::Bool(true));
        assert_eq!(mouse_pressed(&ctx(), vec![win.clone(), right.clone()]).unwrap(), LibValue::Bool(true));
        assert_eq!(mouse_pressed(&ctx(), vec![win.clone(), LibValue::Enum("Left".to_string())]).unwrap(), LibValue::Bool(false));
        assert!(mouse_down(&ctx(), vec![win.clone(), LibValue::Enum("Nope".to_string())]).is_err(), "not a mouse button");
        assert!(mouse_down(&ctx(), vec![win.clone(), LibValue::Int(1)]).is_err(), "not a Mouse");
    }

    fn set_size(width: usize, height: usize) {
        TEST_HANDLES.with(|handles| handles.borrow().as_ref().expect("a test window is open").size.set((width, height)));
    }

    #[test]
    fn the_close_handler_is_queued_once_when_the_window_closes() {
        let win = open("close", 8, 8);
        on_close(&ctx(), vec![win.clone(), LibValue::Handler(5)]).unwrap();
        on_close(&ctx(), vec![win.clone(), LibValue::Handler(6)]).unwrap();
        assert!(crate::take_handlers().is_empty(), "nothing runs before the window closes");
        close(&ctx(), vec![win.clone()]).unwrap();
        next_frame(&ctx(), vec![win.clone()]).unwrap();
        assert_eq!(ids(), vec![6], "the last one set is the one that runs");
        next_frame(&ctx(), vec![win.clone()]).unwrap();
        assert!(crate::take_handlers().is_empty(), "the close handler runs once");
    }

    #[test]
    fn blur_and_focus_handlers_run_on_the_frame_the_focus_changes() {
        let win = open("focus", 8, 8);
        on_focus(&ctx(), vec![win.clone(), LibValue::Handler(10)]).unwrap();
        on_blur(&ctx(), vec![win.clone(), LibValue::Handler(11)]).unwrap();
        next_frame(&ctx(), vec![win.clone()]).unwrap();
        assert!(crate::take_handlers().is_empty(), "opening is not a change of focus");
        set_focus(false);
        next_frame(&ctx(), vec![win.clone()]).unwrap();
        assert_eq!(ids(), vec![11]);
        set_focus(true);
        next_frame(&ctx(), vec![win.clone()]).unwrap();
        assert_eq!(ids(), vec![10]);
        next_frame(&ctx(), vec![win.clone()]).unwrap();
        assert!(crate::take_handlers().is_empty(), "no change, no handler");
    }

    #[test]
    fn a_handler_that_is_not_a_function_is_refused() {
        let win = open("bad", 8, 8);
        assert!(on_close(&ctx(), vec![win.clone(), LibValue::Int(5)]).is_err());
        assert!(on_focus(&ctx(), vec![win.clone()]).is_err(), "a handler is missing");
        assert!(on_blur(&ctx(), vec![LibValue::Int(1), LibValue::Handler(1)]).is_err(), "not a window");
    }

    #[test]
    fn the_resize_handler_gets_the_new_size_once_per_change() {
        let win = open("resize", 8, 8);
        on_resize(&ctx(), vec![win.clone(), LibValue::Handler(20)]).unwrap();
        next_frame(&ctx(), vec![win.clone()]).unwrap();
        assert!(crate::take_handlers().is_empty(), "opening is not a resize");
        set_size(640, 480);
        next_frame(&ctx(), vec![win.clone()]).unwrap();
        assert_eq!(crate::take_handlers(), vec![(20, vec![LibValue::Int(640), LibValue::Int(480)])]);
        next_frame(&ctx(), vec![win.clone()]).unwrap();
        assert!(crate::take_handlers().is_empty(), "the same size again is no change");
    }

    #[test]
    fn the_mouse_fields_are_read_only_in_the_bridge() {
        // the core has no setter for them: the verifier refuses `win.mouseX = ...` through the descriptor
        let object = MODULE.objects.iter().find(|o| o.name == "Window").unwrap();
        for name in ["mouseX", "mouseY", "mouseInside"] {
            let field = object.fields.iter().find(|f| f.name == name).unwrap();
            assert!(field.set.is_none(), "{name} must be read-only");
        }
    }
}
