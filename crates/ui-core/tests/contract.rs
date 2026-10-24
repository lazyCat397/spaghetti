//! the slice-1 contract, against the null backend.
//! covered here: call order, dt, held keys, closing and errors.
//! not covered: a real window (checked by hand on Windows).

use ui_core::color;
use ui_core::null::NullBackend;
use ui_core::{Backend, Key, Session, State, MAX_DT};

const RED: u32 = 0x00FF_3300;
const BG: u32 = 0x0010_1010;

fn session(w: usize, h: usize) -> Session<NullBackend> {
    Session::open(NullBackend::new(), "test", w, h).unwrap()
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
    let s = session(8, 8);
    assert_eq!(s.state(), State::Created);
    assert!(s.backend().is_open());
}

#[test]
fn drawing_before_the_first_frame_is_an_error() {
    let mut s = session(8, 8);
    assert!(s.clear(BG).is_err());
    assert!(s.rect(0.0, 0.0, 1.0, 1.0, RED).is_err());
    assert!(s.request_close().is_err());
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
    let dt = s.begin_frame(9.0).unwrap().unwrap();
    assert_eq!(dt, 0.0);
}

// --- order and drawing -----------------------------------------------------------

#[test]
fn clear_then_rect_is_visible_in_the_shown_frame() {
    let mut s = session(10, 10);
    s.begin_frame(0.0).unwrap();
    s.clear(BG).unwrap();
    s.rect(2.0, 2.0, 3.0, 3.0, RED).unwrap();
    s.begin_frame(0.016).unwrap(); // shows the previous frame

    let shown = &s.backend().last_frame;
    assert_eq!(shown.len(), 100);
    assert_eq!(shown[2 * 10 + 2], RED, "corner of the rectangle");
    assert_eq!(shown[4 * 10 + 4], RED, "last pixel of the rectangle");
    assert_eq!(shown[5 * 10 + 5], BG, "outside the rectangle");
    assert_eq!(s.backend().frames_shown, 1);
}

#[test]
fn rect_outside_the_window_is_clipped_not_an_error() {
    let mut s = session(10, 10);
    s.begin_frame(0.0).unwrap();
    s.clear(BG).unwrap();
    assert!(s.rect(-5.0, -5.0, 8.0, 8.0, RED).is_ok());
    // the rectangle covers x in [-5, 3) and y in [-5, 3): pixels 0..=2
    assert_eq!(s.pixel(0, 0), Some(RED));
    assert_eq!(s.pixel(2, 2), Some(RED));
    assert_eq!(s.pixel(3, 3), Some(BG), "the end of the rectangle is exclusive");
}

#[test]
fn rect_with_non_finite_numbers_is_an_error() {
    let mut s = session(10, 10);
    s.begin_frame(0.0).unwrap();
    assert!(s.rect(f64::NAN, 0.0, 1.0, 1.0, RED).is_err());
    assert!(s.rect(0.0, 0.0, f64::INFINITY, 1.0, RED).is_err());
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

// --- closing ---------------------------------------------------------------------

#[test]
fn escape_closes_the_window() {
    let mut s = session(8, 8);
    s.begin_frame(0.0).unwrap();
    s.backend_mut().press(Key::Escape);
    assert_eq!(s.begin_frame(0.016).unwrap(), None);
    assert_eq!(s.state(), State::Closed);
    assert!(!s.backend().is_open(), "the backend is closed");
}

#[test]
fn closing_the_window_with_the_button_ends_the_loop() {
    let mut s = session(8, 8);
    s.begin_frame(0.0).unwrap();
    s.backend_mut().close_by_user();
    assert_eq!(s.begin_frame(0.016).unwrap(), None);
    assert_eq!(s.state(), State::Closed);
}

#[test]
fn request_close_inside_a_frame_finishes_that_frame() {
    let mut s = session(8, 8);
    s.begin_frame(0.0).unwrap();
    s.clear(BG).unwrap();
    s.request_close().unwrap();
    // drawing is still allowed: the frame finishes normally
    s.rect(0.0, 0.0, 1.0, 1.0, RED).unwrap();
    assert_eq!(s.state(), State::Running, "nothing is released in the middle of a frame");
    assert_eq!(s.begin_frame(0.016).unwrap(), None);
    assert_eq!(s.state(), State::Closed);
}

#[test]
fn drawing_after_close_is_an_error() {
    let mut s = session(8, 8);
    s.begin_frame(0.0).unwrap();
    s.request_close().unwrap();
    s.begin_frame(0.016).unwrap();
    assert!(s.clear(BG).is_err());
    assert!(s.rect(0.0, 0.0, 1.0, 1.0, RED).is_err());
}

#[test]
fn begin_frame_after_close_keeps_returning_none() {
    let mut s = session(8, 8);
    s.begin_frame(0.0).unwrap();
    s.request_close().unwrap();
    assert_eq!(s.begin_frame(0.016).unwrap(), None);
    assert_eq!(s.begin_frame(0.032).unwrap(), None);
}

#[test]
fn backend_is_closed_exactly_once() {
    let mut s = session(8, 8);
    let closes = s.backend().close_counter();
    s.begin_frame(0.0).unwrap();
    s.request_close().unwrap();
    s.begin_frame(0.016).unwrap();
    s.begin_frame(0.032).unwrap();
    drop(s);
    assert_eq!(closes.get(), 1, "closing twice must not free the window twice");
}

#[test]
fn dropping_an_open_window_closes_it() {
    let mut s = session(8, 8);
    let closes = s.backend().close_counter();
    s.begin_frame(0.0).unwrap();
    assert_eq!(closes.get(), 0, "still open");
    drop(s); // no closing begin_frame: the drop must close it
    assert_eq!(closes.get(), 1);
}

#[test]
fn a_boxed_backend_works_the_same() {
    let boxed: Box<dyn Backend> = Box::new(NullBackend::new());
    let mut s = Session::open(boxed, "boxed", 4, 4).unwrap();
    assert_eq!(s.begin_frame(0.0).unwrap(), Some(0.0));
    s.clear(BG).unwrap();
    s.request_close().unwrap();
    assert_eq!(s.begin_frame(0.016).unwrap(), None);
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

#[test]
/// the contract for a backend that reports every event. the minifb backend can miss taps
/// shorter than a frame (see the spec), so this runs on the null backend only
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
    // the user presses Right in the middle of the frame: the frame does not see it yet
    assert!(!s.is_down(Key::Left));
    assert!(!s.is_down(Key::Right));
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
