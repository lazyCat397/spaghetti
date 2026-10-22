//! `test`: checks inside a program (`install #test`). `test.expect(actual, expected)` counts a
//! pass, or records a failure with the line that made the check. nothing stops at a failure:
//! the whole file runs, and the CLI lists every failure with its place when it ends. floats are
//! compared exactly. both values must have the same type (int with int, string with string...).

use std::sync::Mutex;

use crate::{CallCtx, Kind, LibError, LibValue, NativeFn, NativeModule};

const EXPECT_DOC: &str = "check that the actual value equals the expected one. a mismatch is reported at the end of the run";

pub static MODULE: NativeModule = NativeModule {
    kind: Kind::Install,
    name: "test",
    doc: "checks: test.expect(actual, expected) (install #test)",
    structs: &[],
    functions: &[
        NativeFn { name: "expect", sig: "(int, int) -> void", doc: EXPECT_DOC, call: expect },
        NativeFn { name: "expect", sig: "(float, float) -> void", doc: EXPECT_DOC, call: expect },
        NativeFn { name: "expect", sig: "(bool, bool) -> void", doc: EXPECT_DOC, call: expect },
        NativeFn { name: "expect", sig: "(string, string) -> void", doc: EXPECT_DOC, call: expect },
    ],
};

/// one check that did not hold
#[derive(Debug, Clone, PartialEq)]
pub struct TestFailure {
    /// the script file of the check (None when unknown)
    pub file: Option<String>,
    pub line: u32,
    pub message: String,
}

/// everything the checks of the run counted
#[derive(Debug, Default, Clone, PartialEq)]
pub struct TestReport {
    pub passed: usize,
    pub failures: Vec<TestFailure>,
}

static REPORT: Mutex<TestReport> = Mutex::new(TestReport { passed: 0, failures: Vec::new() });

/// hands over the report and starts a new one
pub fn take_report() -> TestReport {
    let mut report = REPORT.lock().unwrap_or_else(|e| e.into_inner());
    std::mem::take(&mut *report)
}

/// counts one check into `report`
fn compare(report: &mut TestReport, file: Option<&str>, line: u32, actual: &LibValue, expected: &LibValue) {
    if actual == expected {
        report.passed += 1;
    } else {
        report.failures.push(TestFailure {
            file: file.map(str::to_string),
            line,
            message: format!("expected {}, got {}", show(expected), show(actual)),
        });
    }
}

/// a value the way a person reads it: text in quotes, numbers and bools as they are
fn show(v: &LibValue) -> String {
    match v {
        LibValue::Str(s) => format!("{:?}", s),
        LibValue::Int(n) => n.to_string(),
        LibValue::Float(x) => x.to_string(),
        LibValue::Bool(b) => b.to_string(),
        other => format!("{:?}", other),
    }
}

fn expect(ctx: &CallCtx, args: Vec<LibValue>) -> Result<LibValue, LibError> {
    let (Some(actual), Some(expected)) = (args.first(), args.get(1)) else {
        return Err(LibError::new("test.expect", "needs two values: the actual one and the expected one"));
    };
    let mut report = REPORT.lock().unwrap_or_else(|e| e.into_inner());
    compare(&mut report, ctx.caller_file, ctx.caller_line, actual, expected);
    Ok(LibValue::Void)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_match_counts_a_pass_and_a_mismatch_keeps_the_place() {
        let mut report = TestReport::default();
        compare(&mut report, Some("a.spg"), 7, &LibValue::Int(3), &LibValue::Int(3));
        compare(&mut report, Some("a.spg"), 9, &LibValue::Int(4), &LibValue::Int(3));
        assert_eq!(report.passed, 1);
        assert_eq!(report.failures.len(), 1);
        assert_eq!(report.failures[0].line, 9);
        assert_eq!(report.failures[0].message, "expected 3, got 4");
    }

    #[test]
    fn text_is_quoted_in_the_message() {
        let mut report = TestReport::default();
        compare(&mut report, None, 1, &LibValue::Str("ola".into()), &LibValue::Str("oi".into()));
        assert_eq!(report.failures[0].message, "expected \"oi\", got \"ola\"");
        assert_eq!(report.failures[0].file, None);
    }
}
