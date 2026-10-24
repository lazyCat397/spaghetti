//! `cmd`: run another program and get back its code and its output (`install #cmd`). the
//! program is started directly, with the arguments as given: no shell reads the text, so an
//! argument can not run anything it was not meant to. its input is empty, so it never waits
//! for a keyboard. `run(program, args, ms)` gives up after `ms` milliseconds.

use std::io::{self, Read};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use crate::{CallCtx, Kind, LibError, LibValue, NativeFn, NativeModule};

pub static MODULE: NativeModule = NativeModule {
    kind: Kind::Install,
    name: "cmd",
    doc: "run another program and get its code and output (install #cmd)",
    structs: &["struct CmdResult {\n    code: int,\n    output: string,\n    error: string\n}\n"],
    objects: &[],
    functions: &[
        NativeFn { name: "run", sig: "(string, array<string>) -> CmdResult", doc: "run a program with its arguments and wait for it. code -1 means it was stopped by a signal", call: run },
        NativeFn { name: "run", sig: "(string, array<string>, int) -> CmdResult", doc: "the same, but gives up after the given number of milliseconds (an error)", call: run_with_timeout },
    ],
};

fn text(a: &[LibValue], i: usize) -> &str {
    match a.get(i) {
        Some(LibValue::Str(t)) => t.as_str(),
        _ => "",
    }
}

fn texts(a: &[LibValue], i: usize) -> Vec<String> {
    match a.get(i) {
        Some(LibValue::Array(items)) => items
            .iter()
            .map(|v| match v {
                LibValue::Str(s) => s.clone(),
                _ => String::new(),
            })
            .collect(),
        _ => Vec::new(),
    }
}

fn int(a: &[LibValue], i: usize) -> i64 {
    match a.get(i) {
        Some(LibValue::Int(n)) => *n,
        _ => 0,
    }
}

/// why a program could not be started, in words a person reads
fn start_error(program: &str, e: io::Error) -> LibError {
    let why = match e.kind() {
        io::ErrorKind::NotFound => "no such program".to_string(),
        io::ErrorKind::PermissionDenied => "permission denied".to_string(),
        _ => e.to_string(),
    };
    LibError::new("cmd.run", format!("could not start '{}': {}", program, why))
}

/// the result as a struct: the code, then the output and the error text
fn outcome(code: Option<i32>, out: Vec<u8>, err: Vec<u8>) -> LibValue {
    LibValue::record(
        "CmdResult",
        vec![
            ("code", LibValue::Int(code.map(i64::from).unwrap_or(-1))),
            ("output", LibValue::Str(String::from_utf8_lossy(&out).into_owned())),
            ("error", LibValue::Str(String::from_utf8_lossy(&err).into_owned())),
        ],
    )
}

fn run(_: &CallCtx, a: Vec<LibValue>) -> Result<LibValue, LibError> {
    let program = text(&a, 0);
    let output = Command::new(program)
        .args(texts(&a, 1))
        .stdin(Stdio::null())
        .output()
        .map_err(|e| start_error(program, e))?;
    Ok(outcome(output.status.code(), output.stdout, output.stderr))
}

fn run_with_timeout(_: &CallCtx, a: Vec<LibValue>) -> Result<LibValue, LibError> {
    let ms = int(&a, 2);
    if ms <= 0 {
        return Err(LibError::new("cmd.run", "the timeout must be more than 0 milliseconds"));
    }
    let program = text(&a, 0);
    let mut child = Command::new(program)
        .args(texts(&a, 1))
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| start_error(program, e))?;
    // the output is read on its own threads, so a program that fills a pipe can not stall the wait
    let out_reader = std::thread::spawn({
        let pipe = child.stdout.take();
        move || read_all(pipe)
    });
    let err_reader = std::thread::spawn({
        let pipe = child.stderr.take();
        move || read_all(pipe)
    });
    let deadline = Instant::now() + Duration::from_millis(ms as u64);
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                let out = out_reader.join().unwrap_or_default();
                let err = err_reader.join().unwrap_or_default();
                return Ok(outcome(status.code(), out, err));
            }
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(5)),
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                // the readers are left to finish by themselves: a program that started its own
                // programs may keep the pipes open, and waiting for them would hang
                return Err(LibError::new("cmd.run", format!("timed out after {} ms", ms)));
            }
            Err(e) => return Err(LibError::new("cmd.run", e.to_string())),
        }
    }
}

/// everything a pipe holds, or nothing when the stream was not piped
fn read_all<R: Read>(pipe: Option<R>) -> Vec<u8> {
    let mut buf = Vec::new();
    if let Some(mut p) = pipe {
        let _ = p.read_to_end(&mut buf);
    }
    buf
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    fn ctx() -> CallCtx<'static> {
        CallCtx { caller_file: None, caller_line: 0 }
    }

    fn s(t: &str) -> LibValue {
        LibValue::Str(t.to_string())
    }

    fn sh(script: &str) -> Vec<LibValue> {
        vec![s("sh"), LibValue::Array(vec![s("-c"), s(script)])]
    }

    fn field(result: &LibValue, name: &str) -> LibValue {
        match result {
            LibValue::Struct { fields, .. } => fields.iter().find(|(n, _)| n == name).map(|(_, v)| v.clone()).unwrap(),
            other => panic!("not a struct: {:?}", other),
        }
    }

    #[test]
    fn code_and_both_streams_come_back() {
        let r = run(&ctx(), sh("echo ola; echo erro 1>&2; exit 3")).unwrap();
        assert_eq!(field(&r, "code"), LibValue::Int(3));
        assert_eq!(field(&r, "output"), s("ola\n"));
        assert_eq!(field(&r, "error"), s("erro\n"));
    }

    #[test]
    fn a_slow_program_is_stopped_by_the_timeout() {
        let mut args = sh("sleep 5");
        args.push(LibValue::Int(200));
        let err = run_with_timeout(&ctx(), args).unwrap_err();
        assert!(err.reason.contains("timed out"), "{}", err.reason);
    }

    #[test]
    fn a_missing_program_says_it_could_not_start() {
        let err = run(&ctx(), vec![s("programa-que-nao-existe-xyz"), LibValue::Array(vec![])]).unwrap_err();
        assert!(err.reason.contains("could not start"), "{}", err.reason);
    }
}
