# The `ui` library

`ui` opens windows and draws shapes, text and images. It also reads the keyboard and the mouse. A program is a loop: each frame, you read the input, change the objects, and let the window draw.

```javascript
attach #ui

let win = ui.window("hello", 320, 180)
win.background = ui.hex("#202636")

win.label(16.0, 16.0, "Hello, ui!", ui.hex("#ffffff"), 16)

while (true) {
    let dt = win.nextFrame()
    if (dt < 0.0) { break }
}
```

<!-- screenshot: the hello window -->

`win.nextFrame()` draws the frame and returns the time since the last frame, in seconds. It returns a negative number when the window closes, so the loop checks for that and stops.

The examples in `examples/ui/` cover each feature on this page, and they are a good place to start.

## Windows and colors

```javascript
let win = ui.window("title", 400, 300)
win.background = ui.hex("#1e1e28")
```

`ui.window(title, width, height)` opens the window. `ui.hex("#rrggbb")` makes a color from text, and `ui.rgb(r, g, b)` makes one from numbers from `0` to `255`.

The window has `width` and `height` fields, which are whole numbers.

## Shapes

A shape is a rectangle, an ellipse, a line or a polygon. Each one is created on the window and returned as an object you can change:

```javascript
let box = win.rect(20.0, 20.0, 80.0, 40.0, ui.hex("#ff3300"))
let disc = win.ellipse(120.0, 20.0, 40.0, 40.0, ui.hex("#b388ff"))
let wire = win.line(10.0, 100.0, 150.0, 90.0, 6.0, ui.hex("#ffd23f"))
let star = win.polygon([40.0, 150.0, 110.0, 150.0, 75.0, 60.0], ui.hex("#33ccff"))
```

- `rect(x, y, width, height, color)`
- `ellipse(x, y, width, height, color)`. A square box makes a circle.
- `line(x1, y1, x2, y2, thickness, color)`. The ends are round.
- `polygon([x0, y0, x1, y1, ...], color)`. It needs at least three points.

Positions and sizes are in pixels, and they are floats, so write `20.0` and not `20`.

Each shape has fields you can read and write:

```javascript
box.x = box.x + 2.0
box.color = ui.rgb(0, 255, 120)
```

| Field | What it does |
|---|---|
| `x`, `y` | Position. |
| `width`, `height` | Size in pixels. Set them to resize the shape. |
| `color` | Fill color. |
| `rotation` | Angle in degrees, clockwise, around the pivot. |
| `pivotX`, `pivotY` | The pivot as a fraction of the size. `0.5` is the centre. |
| `alpha` | Transparency, from `0.0` (invisible) to `1.0` (solid). |
| `fill` | `false` draws only the outline. |
| `stroke` | Outline thickness in pixels, drawn inside the edges. |
| `strokeColor` | Outline color. |
| `radius` | Rounded corners, in pixels. Capped at half the smaller side. |
| `topLeftRadius`, `topRightRadius`, `bottomRightRadius`, `bottomLeftRadius` | One corner each. |

A rotated shape turns around its pivot. To spin a square around its centre, keep the default pivot and change the rotation each frame:

```javascript
let spin = win.rect(60.0, 60.0, 80.0, 80.0, ui.hex("#ff3300"))

while (true) {
    let dt = win.nextFrame()
    if (dt < 0.0) { break }

    spin.rotation = spin.rotation + 45.0 * dt
}
```

Use `dt` to move or turn things. A value per second times `dt` gives the same speed on any computer.

Call `shape.kill()` to remove a shape from the window.

## Antialiasing

Turned, transparent and outlined shapes can look jagged. Set `win.antialias = true` to smooth their edges. It costs some speed, so it is off by default. Shapes that are not turned, transparent or outlined always look sharp, and they are not affected.

## Text

```javascript
let label = win.label(16.0, 16.0, "Hello", ui.hex("#ffffff"), 16)
label.text = "Hello again"
```

`win.label(x, y, text, color, size)` draws text. `size` is in pixels, from 1 to 512. The bitmap font works without any file. For a nicer font, load a `.ttf` or `.otf` file. The path is relative to the `.spg` file:

```javascript
label.font = "../assets/fonts/LiberationSans-Regular.ttf"
```

Text has these fields:

| Field | What it does |
|---|---|
| `text` | The text shown. |
| `font` | Path to a `.ttf` or `.otf` file. `""` goes back to the bitmap font. |
| `wrap` | Width in pixels. The text breaks into lines by words to fit. |
| `align` | `Align.Left`, `Align.Center` or `Align.Right`, inside the wrap width. |
| `italic` | Slant in degrees, from `-45` to `45`. Positive leans right. |
| `stroke`, `strokeColor` | An outline around the letters, and its color. It works as bold. |
| `letterSpacing`, `wordSpacing`, `lineSpacing` | Space in pixels. Negative values are allowed. |

Use `\n` in the text to start a new line. To color part of a text, wrap it in tags:

```javascript
win.label(16.0, 20.0, "Life: %{#ff3300}100%{/} of 100", ui.hex("#ffffff"), 16)
```

`%{#rrggbb}` starts a color, and `%{/}` goes back to the label's color.

To measure a text before drawing it, use `ui.textWidth(text, size, spacing)` and `ui.textHeight(text, size, spacing)`. This is useful for centering.

## Images

```javascript
let sprite = win.image(20.0, 100.0, "../assets/images/sprite.png", 32, 32)
sprite.x = sprite.x + 1.0
sprite.visible = false
```

`win.image(x, y, path, width, height)` loads a PNG file. Transparent pixels stay transparent. Use `0` for a width or height to keep the size of the image on that side. Only PNG is supported for now.

Images have `x`, `y`, `width`, `height` and `visible` fields, and `kill()` removes them.

## Keyboard and mouse

```javascript
if (win.isDown(Key.Right)) { box.x = box.x + 200.0 * dt }
if (win.wasPressed(Key.Space)) { box.color = ui.rgb(0, 255, 120) }
```

- `win.isDown(key)` is `true` while the key is held down.
- `win.wasPressed(key)` is `true` only on the frame the key goes down.
- `win.wasReleased(key)` is `true` only on the frame the key goes up.
- `win.typed` holds the text typed during the frame.

Keys are in the `Key` enum, such as `Key.Space`, `Key.Left` and `Key.C`. The editor suggests the names after `Key.`.

The mouse is read through the window:

```javascript
if (win.mouseDown(Mouse.Left) && win.mouseInside) {
    print(win.mouseX, win.mouseY)
}
```

`mouseX` and `mouseY` are the position in the window, `mouseInside` tells whether the pointer is over the window, and `mouseDown`, `mousePressed` and `mouseReleased` take a button from the `Mouse` enum.

Shapes and text can also be clicked. A shape answers `shape.mousePressed(Mouse.Left)`, which is `true` on the frame it was clicked. Text does not take clicks, so a click on a text goes to the shape under it.

A very short tap that lasts less than one frame can be missed. This is a known limit of the library.

## Full demos

Two complete programs live in `examples/ui/`. Run them from a terminal:

```javascript
spg examples/ui/clock/main.spg
spg examples/ui/shooter/main.spg
```

- **`clock/`** draws an analog clock. The hour comes from the system. On Windows, the program asks PowerShell for the UTC offset, DST included. Anywhere else, or if PowerShell fails, it uses `FALLBACK_OFFSET_MIN` in `helper/local-time.spg`, which is -180 (Brasília). Change it to your own offset in minutes.
- **`shooter/`** is a small shoot 'em up with sprites, a TTF font, and a title and game over screen. Arrows move the ship, hold Space to fire, Enter or Space starts, Enter or R plays again, and Esc quits. Each sprite's hitbox matches the painted part of its PNG.

Both programs load their images and fonts with paths relative to `main.spg`, so run them from any folder.

## Limits

- Dragging the window by its title bar freezes the frames on Windows. This is how Windows handles window dragging, and the library does not change it yet.
- Images are only PNG. Text has no kerning, and there is no cache for glyphs.
- Positions are in pixels.
