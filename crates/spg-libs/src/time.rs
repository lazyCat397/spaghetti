//! `@time`: the clock, and sleep. part of the core (always there, written with `@`).
//! `sleep` is the one call a library can not do alone: it has to give the baton away so other
//! tasks keep running, and only the VM loop can. so the VM runs it and this file just declares it.

use std::sync::OnceLock;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use crate::util::iso_utc;
use crate::{CallCtx, Kind, LibError, LibValue, NativeFn, NativeModule};

pub static MODULE: NativeModule = NativeModule {
    kind: Kind::Core,
    name: "time",
    doc: "clock and time, and sleep",
    structs: &[],
    functions: &[
        NativeFn { name: "now", sig: "() -> string", doc: "the current date and time as text", call: now },
        NativeFn { name: "unix", sig: "() -> int", doc: "milliseconds since 1970 (the Unix epoch), as an int", call: unix },
        NativeFn { name: "elapsed", sig: "() -> string", doc: "time since the program started, as text", call: elapsed },
        NativeFn { name: "sleep", sig: "(int) -> void", doc: "pause for n milliseconds. other tasks keep running meanwhile", call: sleep },
    ],
};

static PROGRAM_START: OnceLock<Instant> = OnceLock::new();

/// fixes the instant `@time.elapsed()` counts from. `Vm::new` calls this so the
/// clock starts when the program does (not at the first elapsed call).
/// later calls do nothing.
pub fn mark_program_start() {
    PROGRAM_START.get_or_init(Instant::now);
}

fn program_start() -> Instant {
    *PROGRAM_START.get_or_init(Instant::now)
}

/// whole ms since the Unix epoch. if the system clock is set before 1970,
/// you get a negative number instead of a panic. nice of it.
fn unix_millis() -> i64 {
    match SystemTime::now().duration_since(UNIX_EPOCH) {
        Ok(d) => d.as_millis() as i64,
        Err(e) => -(e.duration().as_millis() as i64),
    }
}

/// @time.now(): current UTC date-time, ISO-8601 string. the format nobody remembers.
fn now(_: &CallCtx, _: Vec<LibValue>) -> Result<LibValue, LibError> {
    Ok(LibValue::Str(iso_utc(unix_millis() / 1000)))
}

/// @time.unix(): whole ms since 1970-01-01 UTC. wall clock, so it can jump around.
fn unix(_: &CallCtx, _: Vec<LibValue>) -> Result<LibValue, LibError> {
    Ok(LibValue::Int(unix_millis()))
}

/// @time.elapsed(): time since the program started (monotonic clock), already formatted in
/// whole units that adapt to size: "45ms", "3s 250ms", "1h 2m".
fn elapsed(_: &CallCtx, _: Vec<LibValue>) -> Result<LibValue, LibError> {
    Ok(LibValue::Str(format_duration(program_start().elapsed().as_nanos())))
}

/// the VM loop runs `sleep` (see the top of this file). if it ever gets here, that's a bug.
fn sleep(_: &CallCtx, _: Vec<LibValue>) -> Result<LibValue, LibError> {
    Err(LibError::new("@time.sleep", "internal error: sleep is run by the VM loop"))
}

/// `nanos` as whole-number units that adapt to the size. like Rust's own
/// `Duration` output, but integer-only. the largest unit that fits at least once,
/// plus the next smaller unit if it's not zero (that's the whole precision
/// shown, so 2h 3m 4s reads `2h 3m`). units: h m s ms µs ns. no `ps`, because
/// the monotonic clock ticks in ns, so ps would always be 0. don't add it.
/// (it would always be 0. I checked. twice.)
fn format_duration(nanos: u128) -> String {
    const UNITS: [(u128, &str); 6] = [
        (3_600_000_000_000, "h"),
        (60_000_000_000, "m"),
        (1_000_000_000, "s"),
        (1_000_000, "ms"),
        (1_000, "\u{b5}s"),
        (1, "ns"),
    ];
    for (i, (size, name)) in UNITS.iter().enumerate() {
        if nanos >= *size {
            let mut out = format!("{}{}", nanos / size, name);
            if let Some((next_size, next_name)) = UNITS.get(i + 1) {
                let next = (nanos % size) / next_size;
                if next > 0 {
                    out.push_str(&format!(" {}{}", next, next_name));
                }
            }
            return out;
        }
    }
    "0ns".to_string()
}

#[cfg(test)]
mod tests {
    use super::format_duration as fd;

    #[test]
    fn duration_picks_the_unit_that_fits() {
        assert_eq!(fd(0), "0ns");
        assert_eq!(fd(1), "1ns");
        assert_eq!(fd(999), "999ns");
        assert_eq!(fd(1_000), "1\u{b5}s");
        assert_eq!(fd(45_000_000), "45ms");
        assert_eq!(fd(3_000_000_000), "3s");
        assert_eq!(fd(60_000_000_000), "1m");
        assert_eq!(fd(3_600_000_000_000), "1h");
        // far beyond a day stays in hours (there is no "d". don't ask for days.)
        assert_eq!(fd(1_700_000_000_000 * 1_000_000), "472222h 13m");
    }

    #[test]
    fn duration_shows_the_next_unit_only_when_it_is_not_zero() {
        assert_eq!(fd(1_500), "1\u{b5}s 500ns");
        assert_eq!(fd(2_000_300), "2ms"); // 0 \u{b5}s in between: nothing appended
        assert_eq!(fd(3_250_000_000), "3s 250ms");
        assert_eq!(fd(90_000_000_000), "1m 30s");
        assert_eq!(fd(3_725_000_000_000), "1h 2m");
        // two units only: seconds get dropped. 2h 3m 4s becomes 2h 3m. sorry seconds.
        assert_eq!(fd(7_384_000_000_000), "2h 3m");
        assert_eq!(fd(3_605_000_000_000), "1h"); // 1h 0m 5s
    }
}
