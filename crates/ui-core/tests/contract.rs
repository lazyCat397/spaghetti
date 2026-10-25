//! the contract of the core, against the null backend.
//! covered here: the scene, call order, dt, held keys, closing and errors.
//! not covered: a real window (checked by hand on Windows).

use ui_core::color;
use ui_core::null::NullBackend;
use ui_core::{Backend, Key, Session, State, MAX_DT};

const RED: u32 = 0x00FF_3300;
const GREEN: u32 = 0x0000_FF00;
const BLUE: u32 = 0x0000_00FF;
const BG: u32 = 0x0010_1010;

fn session(w: usize, h: usize) -> Session<NullBackend> {
    Session::open(NullBackend::new(), "test", w, h).unwrap()
}

/// a session with its first frame already started, so the scene is "live"
fn running(w: usize, h: usize) -> Session<NullBackend> {
    let mut s = session(w, h);
    s.begin_frame(0.0).unwrap();
    s
}

/// draws one more frame and returns the pixel that frame ended up with at (x, y)
fn drawn(s: &mut Session<NullBackend>, now: f64, x: usize, y: usize) -> u32 {
    s.begin_frame(now).unwrap();
    s.backend().last_frame[y * s.width() + x]
}

fn approx(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-9
}

// --- colors and keys ---------------------------------------------------------

#[test]
fn hex_accepts_only_rrggbb() {
    assert_eq!(color::hex("#ff3300"), Ok(RED));
    assert!(color::hex("ff3300").is_err(), "missing #");
    assert!(color::hex("#fff").is_err(), "short form");
    assert!(color::hex("#gg0000").is_err(), "not hexadecimal");
}

#[test]
fn rgb_checks_each_channel() {
    assert_eq!(color::rgb(255, 51, 0), Ok(RED));
    assert!(color::rgb(256, 0, 0).is_err());
    assert!(color::rgb(0, -1, 0).is_err());
}

#[test]
fn a_color_number_must_fit_in_three_bytes() {
    assert_eq!(color::check(0), Ok(0));
    assert_eq!(color::check(0xFF_FFFF), Ok(0xFF_FFFF));
    assert!(color::check(-1).is_err());
    assert!(color::check(0x100_0000).is_err());
}

#[test]
fn key_names_map_to_keys() {
    assert_eq!(Key::from_name("Left"), Some(Key::Left));
    assert_eq!(Key::from_name("Escape"), Some(Key::Escape));
    assert_eq!(Key::from_name("Num7"), Some(Key::Num7));
    assert_eq!(Key::from_name("left"), None, "names are the variant names, case included");
    assert_eq!(Key::from_name("Enter2"), None);
}

#[test]
fn every_key_has_a_distinct_name() {
    let mut names: Vec<String> = Key::ALL.iter().map(|k| format!("{k:?}")).collect();
    let total = names.len();
    names.sort();
    names.dedup();
    assert_eq!(names.len(), total);
    for key in Key::ALL {
        assert_eq!(Key::from_name(&format!("{key:?}")), Some(*key));
    }
}

// --- opening and states --------------------------------------------------------

#[test]
fn window_size_must_be_positive() {
    assert!(Session::open(NullBackend::new(), "x", 0, 10).is_err());
    assert!(Session::open(NullBackend::new(), "x", 10, 0).is_err());
}

#[test]
fn window_starts_created_and_opens_the_backend() {
    let s = session(8, 6);
    assert_eq!(s.state(), State::Created);
    assert_eq!((s.width(), s.height()), (8, 6));
    assert!(s.backend().is_open());
}

#[test]
fn shapes_can_be_made_before_the_loop_starts() {
    let mut s = session(8, 8);
    let id = s.add_rect(1.0, 1.0, 2.0, 2.0, RED).unwrap();
    assert_eq!(s.shape(id).unwrap().color(), RED);
    assert_eq!(s.state(), State::Created);
}

// --- dt --------------------------------------------------------------------------

#[test]
fn first_frame_has_dt_zero() {
    let mut s = session(8, 8);
    assert_eq!(s.begin_frame(10.0).unwrap(), Some(0.0));
}

#[test]
fn later_frames_use_elapsed_time() {
    let mut s = session(8, 8);
    s.begin_frame(10.0).unwrap();
    let dt = s.begin_frame(10.05).unwrap().unwrap();
    assert!(approx(dt, 0.05), "0.05 s elapsed must arrive as 0.05, got {dt}");
}

#[test]
fn dt_is_capped_after_a_long_pause() {
    let mut s = session(8, 8);
    s.begin_frame(10.0).unwrap();
    let dt = s.begin_frame(15.0).unwrap().unwrap();
    assert!(approx(dt, MAX_DT), "a 5 s pause must arrive as {MAX_DT} s, got {dt}");
}

#[test]
fn dt_is_never_negative() {
    let mut s = session(8, 8);
    s.begin_frame(10.0).unwrap();
    assert_eq!(s.begin_frame(9.0).unwrap().unwrap(), 0.0);
}

// --- the scene is drawn by the frame -----------------------------------------

#[test]
fn the_first_frame_already_shows_the_scene() {
    let mut s = session(10, 10);
    s.scene_mut().unwrap().set_background(BG);
    s.add_rect(2.0, 2.0, 3.0, 3.0, RED).unwrap();
    assert_eq!(s.backend().frames_shown, 0);
    s.begin_frame(0.0).unwrap();
    assert_eq!(s.backend().frames_shown, 1, "no empty first frame");
    let shown = &s.backend().last_frame;
    assert_eq!(shown.len(), 100);
    assert_eq!(shown[2 * 10 + 2], RED, "corner of the rectangle");
    assert_eq!(shown[4 * 10 + 4], RED, "last pixel of the rectangle");
    assert_eq!(shown[5 * 10 + 5], BG, "outside the rectangle");
}

#[test]
fn every_frame_shows_one_frame() {
    let mut s = session(4, 4);
    for i in 0..5 {
        s.begin_frame(i as f64 * 0.016).unwrap();
    }
    assert_eq!(s.backend().frames_shown, 5);
}

#[test]
fn a_change_shows_up_on_the_next_frame_and_not_before() {
    let mut s = running(10, 10);
    let id = s.add_rect(0.0, 0.0, 4.0, 4.0, RED).unwrap();
    // changed after the frame started: the frame already shown does not have it
    assert_eq!(s.backend().last_frame[0], 0);
    s.shape_mut(id).unwrap().set_color(GREEN);
    assert_eq!(drawn(&mut s, 0.016, 0, 0), GREEN);
}

#[test]
fn the_background_is_painted_under_everything() {
    let mut s = running(6, 6);
    s.scene_mut().unwrap().set_background(BG);
    assert_eq!(drawn(&mut s, 0.016, 5, 5), BG);
}

#[test]
fn a_shape_outside_the_window_is_cut_not_an_error() {
    let mut s = session(10, 10);
    s.scene_mut().unwrap().set_background(BG);
    s.add_rect(-5.0, -5.0, 8.0, 8.0, RED).unwrap();
    s.add_rect(8.0, 8.0, 50.0, 50.0, BLUE).unwrap();
    s.add_rect(100.0, 100.0, 5.0, 5.0, GREEN).unwrap();
    s.begin_frame(0.0).unwrap();
    // the first covers x and y in [-5, 3): pixels 0..=2
    assert_eq!(s.pixel(0, 0), Some(RED));
    assert_eq!(s.pixel(2, 2), Some(RED));
    assert_eq!(s.pixel(3, 3), Some(BG), "the end of the rectangle is exclusive");
    assert_eq!(s.pixel(9, 9), Some(BLUE));
}

#[test]
fn a_shape_of_size_zero_draws_nothing() {
    let mut s = session(6, 6);
    s.scene_mut().unwrap().set_background(BG);
    s.add_rect(2.0, 2.0, 0.0, 3.0, RED).unwrap();
    s.add_rect(3.0, 3.0, 3.0, 0.0, RED).unwrap();
    s.begin_frame(0.0).unwrap();
    assert!(s.backend().last_frame.iter().all(|&p| p == BG));
}

#[test]
fn an_invisible_shape_is_kept_but_not_drawn() {
    let mut s = session(6, 6);
    s.scene_mut().unwrap().set_background(BG);
    let id = s.add_rect(0.0, 0.0, 6.0, 6.0, RED).unwrap();
    s.shape_mut(id).unwrap().set_visible(false);
    assert_eq!(drawn(&mut s, 0.0, 3, 3), BG);
    assert!(!s.shape(id).unwrap().visible(), "still there, and still readable");
    s.shape_mut(id).unwrap().set_visible(true);
    assert_eq!(drawn(&mut s, 0.016, 3, 3), RED);
}

#[test]
fn the_last_shape_made_is_on_top() {
    let mut s = session(6, 6);
    s.add_rect(0.0, 0.0, 6.0, 6.0, RED).unwrap();
    s.add_rect(0.0, 0.0, 6.0, 6.0, GREEN).unwrap();
    assert_eq!(drawn(&mut s, 0.0, 3, 3), GREEN);
}

#[test]
fn a_higher_layer_is_on_top_whatever_the_creation_order() {
    let mut s = session(6, 6);
    let high = s.add_rect(0.0, 0.0, 6.0, 6.0, RED).unwrap();
    s.add_rect(0.0, 0.0, 6.0, 6.0, GREEN).unwrap(); // made later, but layer 0
    s.shape_mut(high).unwrap().set_layer(5);
    assert_eq!(drawn(&mut s, 0.0, 3, 3), RED);
}

#[test]
fn a_negative_layer_goes_under_the_default_one() {
    let mut s = session(6, 6);
    s.add_rect(0.0, 0.0, 6.0, 6.0, RED).unwrap();
    let low = s.add_rect(0.0, 0.0, 6.0, 6.0, GREEN).unwrap();
    s.shape_mut(low).unwrap().set_layer(-1);
    assert_eq!(drawn(&mut s, 0.0, 3, 3), RED);
}

#[test]
fn shapes_on_one_layer_keep_creation_order() {
    let mut s = session(6, 6);
    let a = s.add_rect(0.0, 0.0, 6.0, 6.0, RED).unwrap();
    let b = s.add_rect(0.0, 0.0, 6.0, 6.0, GREEN).unwrap();
    let c = s.add_rect(0.0, 0.0, 6.0, 6.0, BLUE).unwrap();
    for id in [a, b, c] {
        s.shape_mut(id).unwrap().set_layer(2);
    }
    assert_eq!(drawn(&mut s, 0.0, 3, 3), BLUE);
}

// --- killing -------------------------------------------------------------------

#[test]
fn a_killed_shape_is_gone_from_the_screen() {
    let mut s = session(6, 6);
    s.scene_mut().unwrap().set_background(BG);
    let id = s.add_rect(0.0, 0.0, 6.0, 6.0, RED).unwrap();
    assert_eq!(drawn(&mut s, 0.0, 3, 3), RED);
    s.kill_shape(id);
    assert_eq!(drawn(&mut s, 0.016, 3, 3), BG);
}

#[test]
fn using_a_killed_shape_is_an_error() {
    let mut s = session(6, 6);
    let id = s.add_rect(0.0, 0.0, 1.0, 1.0, RED).unwrap();
    s.kill_shape(id);
    assert_eq!(s.shape(id).unwrap_err(), "this shape was killed");
    assert_eq!(s.shape_mut(id).unwrap_err(), "this shape was killed");
}

#[test]
fn killing_twice_does_nothing() {
    let mut s = session(6, 6);
    let id = s.add_rect(0.0, 0.0, 1.0, 1.0, RED).unwrap();
    s.kill_shape(id);
    s.kill_shape(id);
    s.kill_shape(999);
    assert!(s.scene().unwrap().is_empty());
}

#[test]
fn a_shape_id_is_never_reused() {
    let mut s = session(6, 6);
    let first = s.add_rect(0.0, 0.0, 1.0, 1.0, RED).unwrap();
    s.kill_shape(first);
    let second = s.add_rect(0.0, 0.0, 1.0, 1.0, GREEN).unwrap();
    assert_ne!(first, second);
    assert!(s.shape(first).is_err(), "the old handle must not reach the new shape");
    assert_eq!(s.shape(second).unwrap().color(), GREEN);
}

// --- values are checked --------------------------------------------------------------

#[test]
fn bad_numbers_are_refused_when_a_shape_is_made() {
    let mut s = session(6, 6);
    assert!(s.add_rect(f64::NAN, 0.0, 1.0, 1.0, RED).is_err());
    assert!(s.add_rect(0.0, f64::INFINITY, 1.0, 1.0, RED).is_err());
    assert!(s.add_rect(0.0, 0.0, -1.0, 1.0, RED).is_err(), "negative width");
    assert!(s.add_rect(0.0, 0.0, 1.0, -0.5, RED).is_err(), "negative height");
    assert!(s.add_rect(0.0, 0.0, f64::NAN, 1.0, RED).is_err());
    assert!(s.scene().unwrap().is_empty(), "nothing was added");
}

#[test]
fn bad_numbers_are_refused_by_the_setters_and_the_old_value_stays() {
    let mut s = session(6, 6);
    let id = s.add_rect(1.0, 2.0, 3.0, 4.0, RED).unwrap();
    let shape = s.shape_mut(id).unwrap();
    assert!(shape.set_x(f64::NAN).is_err());
    assert!(shape.set_y(f64::NEG_INFINITY).is_err());
    assert!(shape.set_w(-1.0).is_err());
    assert!(shape.set_h(f64::NAN).is_err());
    assert_eq!((shape.x(), shape.y(), shape.w(), shape.h()), (1.0, 2.0, 3.0, 4.0));
    assert!(shape.set_w(0.0).is_ok(), "zero is fine");
}

// --- held keys -------------------------------------------------------------------

#[test]
fn held_key_stays_down_across_frames() {
    let mut s = session(8, 8);
    s.backend_mut().press(Key::Right);
    s.begin_frame(0.0).unwrap();
    assert!(s.is_down(Key::Right));
    s.begin_frame(0.016).unwrap();
    assert!(s.is_down(Key::Right), "a held key is down on every frame");
}

#[test]
fn released_key_is_no_longer_down() {
    let mut s = session(8, 8);
    s.backend_mut().press(Key::Up);
    s.begin_frame(0.0).unwrap();
    s.backend_mut().release(Key::Up);
    s.begin_frame(0.016).unwrap();
    assert!(!s.is_down(Key::Up));
}

// --- key events --------------------------------------------------------------------

#[test]
fn a_press_is_seen_on_one_frame_only() {
    let mut s = session(8, 8);
    s.begin_frame(0.0).unwrap();
    s.backend_mut().press(Key::Space);
    s.begin_frame(0.016).unwrap();
    assert!(s.was_pressed(Key::Space), "pressed during the previous frame");
    assert!(s.is_down(Key::Space));
    s.begin_frame(0.032).unwrap();
    assert!(!s.was_pressed(Key::Space), "still held, but not a new press");
    assert!(s.is_down(Key::Space));
}

/// the contract for a backend that reports every event. the minifb backend can miss taps
/// shorter than a frame (see the spec), so this runs on the null backend only
#[test]
fn a_tap_shorter_than_a_frame_is_still_a_press() {
    let mut s = session(8, 8);
    s.begin_frame(0.0).unwrap();
    s.backend_mut().press(Key::Space);
    s.backend_mut().release(Key::Space); // down and up before the next frame starts
    s.begin_frame(0.016).unwrap();
    assert!(s.was_pressed(Key::Space), "the tap must not be lost");
    assert!(s.was_released(Key::Space));
    assert!(!s.is_down(Key::Space));
}

#[test]
fn a_release_is_seen_on_one_frame_only() {
    let mut s = session(8, 8);
    s.begin_frame(0.0).unwrap();
    s.backend_mut().press(Key::Up);
    s.begin_frame(0.016).unwrap();
    s.backend_mut().release(Key::Up);
    s.begin_frame(0.032).unwrap();
    assert!(s.was_released(Key::Up));
    s.begin_frame(0.048).unwrap();
    assert!(!s.was_released(Key::Up));
}

#[test]
fn keys_are_a_snapshot_for_the_whole_frame() {
    let mut s = session(8, 8);
    s.begin_frame(0.0).unwrap();
    s.backend_mut().press(Key::Left);
    // the user presses a key in the middle of the frame: the frame does not see it yet
    assert!(!s.is_down(Key::Left));
    s.begin_frame(0.016).unwrap();
    assert!(s.is_down(Key::Left));
}

#[test]
fn escape_pressed_and_released_inside_one_frame_still_closes() {
    let mut s = session(8, 8);
    s.begin_frame(0.0).unwrap();
    s.backend_mut().press(Key::Escape);
    s.backend_mut().release(Key::Escape);
    assert_eq!(s.begin_frame(0.016).unwrap(), None);
}

// --- closing ---------------------------------------------------------------------

#[test]
fn escape_closes_the_window() {
    let mut s = running(8, 8);
    s.backend_mut().press(Key::Escape);
    assert_eq!(s.begin_frame(0.016).unwrap(), None);
    assert_eq!(s.state(), State::Closed);
    assert!(!s.backend().is_open(), "the backend is closed");
}

#[test]
fn closing_the_window_with_the_button_ends_the_loop() {
    let mut s = running(8, 8);
    s.backend_mut().close_by_user();
    assert_eq!(s.begin_frame(0.016).unwrap(), None);
    assert_eq!(s.state(), State::Closed);
}

#[test]
fn request_close_lets_the_current_frame_finish() {
    let mut s = running(8, 8);
    let id = s.add_rect(0.0, 0.0, 1.0, 1.0, RED).unwrap();
    s.request_close();
    // everything still works until the next frame starts
    s.shape_mut(id).unwrap().set_x(3.0).unwrap();
    s.add_rect(0.0, 0.0, 1.0, 1.0, GREEN).unwrap();
    assert_eq!(s.state(), State::Running, "nothing is released in the middle of a frame");
    assert_eq!(s.begin_frame(0.016).unwrap(), None);
    assert_eq!(s.state(), State::Closed);
}

#[test]
fn request_close_before_the_first_frame_ends_the_loop_at_once() {
    let mut s = session(8, 8);
    s.request_close();
    assert_eq!(s.begin_frame(0.0).unwrap(), None);
}

#[test]
fn the_shapes_die_with_the_window() {
    let mut s = running(8, 8);
    let id = s.add_rect(0.0, 0.0, 1.0, 1.0, RED).unwrap();
    s.request_close();
    s.begin_frame(0.016).unwrap();
    assert_eq!(s.shape(id).unwrap_err(), "the window is closed");
    assert_eq!(s.shape_mut(id).unwrap_err(), "the window is closed");
    assert!(s.add_rect(0.0, 0.0, 1.0, 1.0, RED).is_err());
    assert!(s.scene().is_err());
}

#[test]
fn killing_after_the_window_closed_is_not_an_error() {
    let mut s = running(8, 8);
    let id = s.add_rect(0.0, 0.0, 1.0, 1.0, RED).unwrap();
    s.request_close();
    s.begin_frame(0.016).unwrap();
    s.kill_shape(id); // the shape is already gone: nothing to do
    s.request_close(); // so is closing a closed window
}

#[test]
fn begin_frame_after_close_keeps_returning_none() {
    let mut s = running(8, 8);
    s.request_close();
    assert_eq!(s.begin_frame(0.016).unwrap(), None);
    assert_eq!(s.begin_frame(0.032).unwrap(), None);
    assert_eq!(s.backend().frames_shown, 2, "no frame is shown after the close");
}

#[test]
fn backend_is_closed_exactly_once() {
    let mut s = running(8, 8);
    let closes = s.backend().close_counter();
    s.request_close();
    s.begin_frame(0.016).unwrap();
    s.begin_frame(0.032).unwrap();
    drop(s);
    assert_eq!(closes.get(), 1, "closing twice must not free the window twice");
}

#[test]
fn dropping_an_open_window_closes_it() {
    let s = running(8, 8);
    let closes = s.backend().close_counter();
    assert_eq!(closes.get(), 0, "still open");
    drop(s); // no closing begin_frame: the drop must close it
    assert_eq!(closes.get(), 1);
}

#[test]
fn a_boxed_backend_works_the_same() {
    let boxed: Box<dyn Backend> = Box::new(NullBackend::new());
    let mut s = Session::open(boxed, "boxed", 4, 4).unwrap();
    s.add_rect(0.0, 0.0, 4.0, 4.0, RED).unwrap();
    assert_eq!(s.begin_frame(0.0).unwrap(), Some(0.0));
    assert_eq!(s.pixel(1, 1), Some(RED));
    s.request_close();
    assert_eq!(s.begin_frame(0.016).unwrap(), None);
}

// --- focus --------------------------------------------------------------------------

#[test]
fn the_first_frame_is_not_a_focus_change() {
    let mut s = session(4, 4);
    s.begin_frame(0.0).unwrap();
    assert!(s.take_focus_changes().is_empty(), "opening is not a change");
}

#[test]
fn losing_and_gaining_the_focus_are_reported_once_each() {
    let mut s = session(4, 4);
    let focus = s.backend().focus_handle();
    s.begin_frame(0.0).unwrap();
    focus.set(false);
    s.begin_frame(0.016).unwrap();
    s.begin_frame(0.032).unwrap(); // still unfocused: nothing new
    focus.set(true);
    s.begin_frame(0.048).unwrap();
    assert_eq!(s.take_focus_changes(), vec![false, true]);
    assert!(s.take_focus_changes().is_empty(), "taking them empties the list");
}

#[test]
fn a_window_that_starts_unfocused_reports_its_first_gain_only_when_it_changes() {
    let mut s = session(4, 4);
    let focus = s.backend().focus_handle();
    focus.set(false);
    s.begin_frame(0.0).unwrap();
    assert!(s.take_focus_changes().is_empty());
    focus.set(true);
    s.begin_frame(0.016).unwrap();
    assert_eq!(s.take_focus_changes(), vec![true]);
}

#[test]
fn closing_does_not_report_a_focus_change() {
    let mut s = session(4, 4);
    let focus = s.backend().focus_handle();
    s.begin_frame(0.0).unwrap();
    focus.set(false);
    s.request_close();
    assert_eq!(s.begin_frame(0.016).unwrap(), None);
    assert_eq!(s.take_focus_changes(), vec![false], "the change of this frame is still reported");
    assert_eq!(s.begin_frame(0.032).unwrap(), None);
    assert!(s.take_focus_changes().is_empty());
}

// --- resize -------------------------------------------------------------------------

#[test]
fn the_first_frame_is_not_a_resize() {
    let mut s = session(8, 6);
    s.begin_frame(0.0).unwrap();
    assert!(s.take_resizes().is_empty(), "opening at the given size is not a change");
}

#[test]
fn each_new_size_is_reported_once_in_order() {
    let mut s = session(8, 6);
    let size = s.backend().size_handle();
    s.begin_frame(0.0).unwrap();
    size.set((640, 480));
    s.begin_frame(0.016).unwrap();
    s.begin_frame(0.032).unwrap(); // same size again: nothing new
    size.set((1024, 768));
    s.begin_frame(0.048).unwrap();
    assert_eq!(s.take_resizes(), vec![(640, 480), (1024, 768)]);
    assert!(s.take_resizes().is_empty());
}

#[test]
fn the_framebuffer_keeps_its_size_when_the_window_is_resized() {
    let mut s = running(4, 4);
    let size = s.backend().size_handle();
    s.backend_mut().press(Key::Right); // keep the backend busy
    size.set((400, 400));
    s.begin_frame(0.016).unwrap();
    assert_eq!((s.width(), s.height()), (4, 4));
    assert_eq!(s.backend().last_frame.len(), 16, "the frame shown is still 4x4");
}

#[test]
fn closing_does_not_report_a_resize() {
    let mut s = session(8, 6);
    let size = s.backend().size_handle();
    s.begin_frame(0.0).unwrap();
    size.set((100, 100));
    s.request_close();
    assert_eq!(s.begin_frame(0.016).unwrap(), None);
    assert_eq!(s.take_resizes(), vec![(100, 100)], "the change of the closing frame is reported");
}

// --- mouse -------------------------------------------------------------------------

use ui_core::Button;

#[test]
fn the_pointer_starts_outside_the_window() {
    let mut s = session(8, 8);
    s.begin_frame(0.0).unwrap();
    assert!(!s.mouse_inside());
    assert_eq!(s.mouse_x(), -1.0, "no position was ever seen");
}

#[test]
fn the_position_is_read_each_frame_and_kept_after_the_pointer_leaves() {
    let mut s = session(8, 8);
    let mouse = s.backend().mouse_handle();
    mouse.borrow_mut().position = Some((3.0, 4.0));
    s.begin_frame(0.0).unwrap();
    assert!(s.mouse_inside());
    assert_eq!((s.mouse_x(), s.mouse_y()), (3.0, 4.0));
    mouse.borrow_mut().position = None;
    s.begin_frame(0.016).unwrap();
    assert!(!s.mouse_inside());
    assert_eq!((s.mouse_x(), s.mouse_y()), (3.0, 4.0), "the last position stays");
}

#[test]
fn the_position_is_in_framebuffer_pixels_when_the_window_is_shown_bigger() {
    let mut s = session(8, 8);
    s.backend().size_handle().set((16, 16));
    s.backend().mouse_handle().borrow_mut().position = Some((8.0, 12.0));
    s.begin_frame(0.0).unwrap();
    assert_eq!((s.mouse_x(), s.mouse_y()), (4.0, 6.0));
}

#[test]
fn a_held_button_is_pressed_once_and_released_once() {
    let mut s = session(8, 8);
    let mouse = s.backend().mouse_handle();
    s.begin_frame(0.0).unwrap(); // the first frame: nothing went down or up yet
    mouse.borrow_mut().down.insert(Button::Left);
    s.begin_frame(0.016).unwrap();
    assert!(s.button_pressed(Button::Left), "the frame it went down");
    assert!(s.button_down(Button::Left));
    s.begin_frame(0.032).unwrap();
    assert!(!s.button_pressed(Button::Left), "still held: no new press");
    assert!(s.button_down(Button::Left));
    mouse.borrow_mut().down.remove(&Button::Left);
    s.begin_frame(0.048).unwrap();
    assert!(s.button_released(Button::Left));
    assert!(!s.button_down(Button::Left));
    s.begin_frame(0.064).unwrap();
    assert!(!s.button_released(Button::Left), "one frame only");
}

#[test]
fn a_button_held_when_the_window_opens_is_not_a_press() {
    let mut s = session(8, 8);
    s.backend().mouse_handle().borrow_mut().down.insert(Button::Right);
    s.begin_frame(0.0).unwrap();
    assert!(s.button_down(Button::Right));
    assert!(!s.button_pressed(Button::Right), "it was already down when the program started");
}

#[test]
fn buttons_are_told_apart() {
    let mut s = session(8, 8);
    let mouse = s.backend().mouse_handle();
    s.begin_frame(0.0).unwrap();
    mouse.borrow_mut().down.insert(Button::Middle);
    s.begin_frame(0.016).unwrap();
    assert!(s.button_pressed(Button::Middle));
    assert!(!s.button_pressed(Button::Left) && !s.button_pressed(Button::Right));
}

// --- clicks on shapes ------------------------------------------------------------------

/// a session with one mouse frame done, and the pointer at (x, y)
fn pointer_at(x: f64, y: f64) -> Session<NullBackend> {
    let mut s = session(20, 20);
    s.begin_frame(0.0).unwrap(); // the first frame reads no press or release
    s.backend().mouse_handle().borrow_mut().position = Some((x, y));
    s
}

#[test]
fn a_click_goes_to_the_topmost_shape_under_the_pointer() {
    let mut s = pointer_at(5.0, 5.0);
    let below = s.add_rect(0.0, 0.0, 10.0, 10.0, RED).unwrap();
    let above = s.add_rect(4.0, 4.0, 10.0, 10.0, GREEN).unwrap(); // made later: on top
    s.backend().mouse_handle().borrow_mut().down.insert(Button::Left);
    s.begin_frame(0.016).unwrap();
    assert!(s.shape_button_pressed(above, Button::Left));
    assert!(!s.shape_button_pressed(below, Button::Left), "the shape under it does not get the click");
    assert!(s.shape_hovered(above) && !s.shape_hovered(below));
}

#[test]
fn a_layer_decides_which_shape_is_on_top() {
    let mut s = pointer_at(5.0, 5.0);
    let high = s.add_rect(0.0, 0.0, 10.0, 10.0, RED).unwrap();
    let low = s.add_rect(0.0, 0.0, 10.0, 10.0, GREEN).unwrap();
    s.shape_mut(high).unwrap().set_layer(3);
    s.backend().mouse_handle().borrow_mut().down.insert(Button::Left);
    s.begin_frame(0.016).unwrap();
    assert!(s.shape_button_pressed(high, Button::Left));
    assert!(!s.shape_button_pressed(low, Button::Left));
}

#[test]
fn a_click_through_shape_lets_the_click_reach_what_is_under_it() {
    let mut s = pointer_at(5.0, 5.0);
    let target = s.add_rect(0.0, 0.0, 10.0, 10.0, RED).unwrap();
    let bullet = s.add_rect(4.0, 4.0, 10.0, 10.0, GREEN).unwrap();
    s.shape_mut(bullet).unwrap().set_click_through(true);
    s.backend().mouse_handle().borrow_mut().down.insert(Button::Left);
    s.begin_frame(0.016).unwrap();
    assert!(s.shape_button_pressed(target, Button::Left), "the bullet is passed through");
    assert!(!s.shape_hovered(bullet));
}

#[test]
fn an_invisible_shape_takes_no_click() {
    let mut s = pointer_at(5.0, 5.0);
    let target = s.add_rect(0.0, 0.0, 10.0, 10.0, RED).unwrap();
    let hidden = s.add_rect(4.0, 4.0, 10.0, 10.0, GREEN).unwrap();
    s.shape_mut(hidden).unwrap().set_visible(false);
    s.backend().mouse_handle().borrow_mut().down.insert(Button::Left);
    s.begin_frame(0.016).unwrap();
    assert!(s.shape_button_pressed(target, Button::Left));
}

#[test]
fn a_killed_shape_takes_no_click() {
    let mut s = pointer_at(5.0, 5.0);
    let target = s.add_rect(0.0, 0.0, 10.0, 10.0, RED).unwrap();
    let gone = s.add_rect(4.0, 4.0, 10.0, 10.0, GREEN).unwrap();
    s.kill_shape(gone);
    s.backend().mouse_handle().borrow_mut().down.insert(Button::Left);
    s.begin_frame(0.016).unwrap();
    assert!(s.shape_button_pressed(target, Button::Left));
}

#[test]
fn a_click_outside_every_shape_reaches_no_shape() {
    let mut s = pointer_at(15.0, 15.0);
    let a = s.add_rect(0.0, 0.0, 10.0, 10.0, RED).unwrap();
    s.backend().mouse_handle().borrow_mut().down.insert(Button::Left);
    s.begin_frame(0.016).unwrap();
    assert!(!s.shape_button_pressed(a, Button::Left));
    assert!(s.mouse_inside(), "the pointer is in the window, just over nothing");
}

#[test]
fn a_shape_keeps_the_button_it_took_even_when_the_pointer_leaves() {
    let mut s = pointer_at(5.0, 5.0);
    let mouse = s.backend().mouse_handle();
    let a = s.add_rect(0.0, 0.0, 10.0, 10.0, RED).unwrap();
    let b = s.add_rect(10.0, 0.0, 10.0, 10.0, GREEN).unwrap();
    mouse.borrow_mut().down.insert(Button::Left);
    s.begin_frame(0.016).unwrap();
    assert!(s.shape_button_pressed(a, Button::Left));
    // dragged over `b` while held: `a` still holds the button, `b` gets nothing
    mouse.borrow_mut().position = Some((15.0, 5.0));
    s.begin_frame(0.032).unwrap();
    assert!(s.shape_button_down(a, Button::Left));
    assert!(!s.shape_button_down(b, Button::Left));
    assert!(s.shape_hovered(b), "the hover still follows the pointer");
    // released over `b`: `a` is the one that gets the release
    mouse.borrow_mut().down.remove(&Button::Left);
    s.begin_frame(0.048).unwrap();
    assert!(s.shape_button_released(a, Button::Left));
    assert!(!s.shape_button_released(b, Button::Left));
}

// --- text -------------------------------------------------------------------------------

use ui_core::text;

#[test]
fn a_glyph_has_its_ink_where_the_font_says_and_the_leftmost_bit_is_the_first_column() {
    // the capital I: its top row is 0b00011110 in the font, and bit 0 is the leftmost column, so the
    // ink is in columns 1 to 4 and the stem in columns 2 and 3
    let mut s = session(10, 10);
    s.scene_mut().unwrap().set_background(BG);
    s.add_label(0.0, 0.0, "I", RED, 1).unwrap();
    s.begin_frame(0.0).unwrap();
    assert_eq!(s.pixel(0, 0), Some(BG), "column 0 of the top row is empty");
    assert_eq!(s.pixel(1, 0), Some(RED));
    assert_eq!(s.pixel(4, 0), Some(RED));
    assert_eq!(s.pixel(5, 0), Some(BG));
    assert_eq!(s.pixel(2, 3), Some(RED), "the stem");
    assert_eq!(s.pixel(4, 3), Some(BG), "beside the stem");
}

#[test]
fn a_scale_draws_each_pixel_of_the_glyph_as_a_block() {
    let mut s = session(20, 20);
    s.scene_mut().unwrap().set_background(BG);
    s.add_label(0.0, 0.0, "I", RED, 2).unwrap();
    s.begin_frame(0.0).unwrap();
    // column 1 at scale 2 is pixels 2 and 3
    assert_eq!(s.pixel(2, 0), Some(RED));
    assert_eq!(s.pixel(3, 1), Some(RED));
    assert_eq!(s.pixel(4, 0), Some(RED));
    assert_eq!(s.pixel(0, 0), Some(BG));
}

#[test]
fn a_label_draws_over_the_shapes_below_and_keeps_its_background_clear() {
    let mut s = session(10, 10);
    s.scene_mut().unwrap().set_background(BG);
    s.add_rect(0.0, 0.0, 10.0, 10.0, GREEN).unwrap();
    let label = s.add_label(0.0, 0.0, "I", RED, 1).unwrap();
    s.begin_frame(0.0).unwrap();
    assert_eq!(s.pixel(1, 0), Some(RED), "the label is on top");
    assert_eq!(s.pixel(0, 0), Some(GREEN), "where the label has no ink, the rectangle shows");
    s.label_mut(label).unwrap().set_layer(-1);
    s.begin_frame(0.016).unwrap();
    assert_eq!(s.pixel(1, 0), Some(GREEN), "a label on a lower layer is under the rectangle");
}

#[test]
fn a_newline_starts_the_next_line_below() {
    assert_eq!(text::size("ab", 2), (32.0, 16.0));
    assert_eq!(text::size("a\nbcd", 1), (24.0, 16.0), "the widest line decides the width");
    let mut s = session(20, 20);
    s.scene_mut().unwrap().set_background(BG);
    s.add_label(0.0, 0.0, "I\nI", RED, 1).unwrap();
    s.begin_frame(0.0).unwrap();
    assert_eq!(s.pixel(1, 0), Some(RED));
    assert_eq!(s.pixel(1, 8), Some(RED), "the second line is 8 pixels lower at scale 1");
}

#[test]
fn accents_have_their_own_glyph_and_unknown_characters_are_a_question_mark() {
    assert_ne!(text::glyph('é'), text::glyph('e'), "é is not e");
    assert_ne!(text::glyph('é'), text::glyph('?'), "é is in the font");
    assert_eq!(text::glyph('\u{20AC}'), text::glyph('?'), "the euro sign is not in the font: a ?");
}

#[test]
fn the_scale_must_be_between_1_and_8() {
    let mut s = session(8, 8);
    assert!(s.add_label(0.0, 0.0, "a", RED, 0).is_err());
    assert!(s.add_label(0.0, 0.0, "a", RED, 9).is_err());
    assert!(s.add_label(0.0, 0.0, "a", RED, 8).is_ok());
    assert!(s.add_label(f64::NAN, 0.0, "a", RED, 1).is_err());
}

#[test]
fn a_label_can_be_changed_and_killed_like_a_shape() {
    let mut s = session(8, 8);
    let label = s.add_label(0.0, 0.0, "a", RED, 1).unwrap();
    s.label_mut(label).unwrap().set_text("bb");
    assert_eq!(s.label(label).unwrap().text(), "bb");
    s.kill_shape(label);
    assert!(s.label(label).is_err(), "killed");
    assert!(s.scene().unwrap().is_empty());
}

#[test]
fn a_label_takes_no_click() {
    let mut s = pointer_at(1.0, 1.0);
    s.add_label(0.0, 0.0, "I", RED, 1).unwrap();
    s.backend().mouse_handle().borrow_mut().down.insert(Button::Left);
    s.begin_frame(0.016).unwrap();
    assert!(!s.shape_hovered(1), "text is not a target");
    assert_eq!(s.hovered(), None);
}
