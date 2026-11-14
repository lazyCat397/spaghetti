//! `cmd`: run another program and get back its code and its output, and check that a program
//! exists (`attach #cmd`). the program is started directly, with the arguments as given: no
//! shell reads the text, so an argument can not run anything it was not meant to. `run` gives
//! its input as nothing, so it never waits for a keyboard; `runWith` can give it text, a folder
//! and environment variables. `#> safe-party` (or `spg run --safe-party`) turns every run off,
//! because a run can start any program; `exists` still works, since it starts nothing.

use std::env;
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus, Stdio};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use crate::{CallCtx, Kind, LibError, LibValue, NativeFn, NativeModule};

pub static MODULE: NativeModule = NativeModule {
    kind: Kind::Attach,
    name: "cmd",
    doc: "run another program and get its code and output, check it exists (attach #cmd)",
    structs: &["struct CmdResult {\n    success: bool,\n    code: int,\n    output: string,\n    error: string\n}\n"],
    objects: &[],
    functions: &[
        NativeFn { name: "run", sig: "(string, array<string>) -> CmdResult", doc: "run a program with its arguments and wait for it. code -1 means it was stopped by a signal", call: run },
        NativeFn { name: "run", sig: "(string, array<string>, int) -> CmdResult", doc: "the same, but gives up after the given number of milliseconds (an error)", call: run_with_timeout },
        NativeFn { name: "runWith", sig: "(string, array<string>, map<string, string>) -> CmdResult", doc: "the same, with options: \"dir\" (folder to run in), \"input\" (text for its input), \"timeout\" (milliseconds), \"env.NAME\" (a variable to set)", call: run_with },
        NativeFn { name: "exists", sig: "(string) -> bool", doc: "true if a program with that name (or path) is there to run. looks in PATH, and on Windows tries the usual extensions", call: exists },
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

/// what the options of `runWith` say. every key is checked: a typo is an error, not ignored
#[derive(Default)]
struct Options {
    dir: Option<String>,
    input: Option<String>,
    timeout_ms: Option<u64>,
    env: Vec<(String, String)>,
}

/// the options map of `runWith`, read key by key
fn options(function: &'static str, map: Option<&LibValue>) -> Result<Options, LibError> {
    let mut opts = Options::default();
    let pairs = match map {
        Some(LibValue::Map(pairs)) => pairs.as_slice(),
        _ => &[],
    };
    for (key, value) in pairs {
        let (key, value) = match (key, value) {
            (LibValue::Str(k), LibValue::Str(v)) => (k.as_str(), v.as_str()),
            _ => return Err(LibError::new(function, "the options must be text keys with text values")),
        };
        match key {
            "dir" => opts.dir = Some(value.to_string()),
            "input" => opts.input = Some(value.to_string()),
            "timeout" => {
                let ms: i64 = value
                    .trim()
                    .parse()
                    .map_err(|_| LibError::new(function, format!("timeout '{}' is not a number of milliseconds", value)))?;
                if ms <= 0 {
                    return Err(LibError::new(function, "the timeout must be more than 0 milliseconds"));
                }
                opts.timeout_ms = Some(ms as u64);
            }
            _ => match key.strip_prefix("env.") {
                Some(name) if !name.is_empty() && !name.contains('=') && !name.contains('\0') => {
                    opts.env.push((name.to_string(), value.to_string()))
                }
                _ => {
                    return Err(LibError::new(
                        function,
                        format!("unknown option '{}' (use \"dir\", \"input\", \"timeout\" or \"env.NAME\")", key),
                    ))
                }
            },
        }
    }
    Ok(opts)
}

/// why a program could not be started, in words a person reads
fn start_error(function: &'static str, program: &str, e: std::io::Error) -> LibError {
    let why = match e.kind() {
        std::io::ErrorKind::NotFound => "no such program".to_string(),
        std::io::ErrorKind::PermissionDenied => "permission denied".to_string(),
        _ => e.to_string(),
    };
    LibError::new(function, format!("could not start '{}': {}", program, why))
}

/// the one place a program is started. every `run` form goes through here
fn execute(function: &'static str, program: &str, args: Vec<String>, opts: Options) -> Result<LibValue, LibError> {
    let mut cmd = Command::new(program);
    cmd.args(args)
        .stdin(if opts.input.is_some() { Stdio::piped() } else { Stdio::null() })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(dir) = &opts.dir {
        cmd.current_dir(dir);
    }
    for (name, value) in &opts.env {
        cmd.env(name, value);
    }
    let mut child = cmd.spawn().map_err(|e| start_error(function, program, e))?;

    // the input is written on its own thread, and the pipe is closed after it, so the program
    // sees the end of its input. a program that never reads it just makes the write fail
    if let Some(input) = opts.input {
        if let Some(mut pipe) = child.stdin.take() {
            std::thread::spawn(move || {
                let _ = pipe.write_all(input.as_bytes());
            });
        }
    }
    // the output is read on its own threads, so a program that fills a pipe can not stall the wait
    let out_reader = reader(child.stdout.take());
    let err_reader = reader(child.stderr.take());

    let status = match opts.timeout_ms {
        None => child.wait().map_err(|e| LibError::new(function, e.to_string()))?,
        Some(ms) => wait_with_deadline(function, &mut child, ms)?,
    };
    // the readers are joined only when the program ended on its own. after a timeout they are
    // left to finish by themselves: a program that started its own programs may keep the pipes
    // open, and waiting for them would hang
    let out = out_reader.join().unwrap_or_default();
    let err = err_reader.join().unwrap_or_default();
    Ok(outcome(status, out, err))
}

fn wait_with_deadline(function: &'static str, child: &mut std::process::Child, ms: u64) -> Result<ExitStatus, LibError> {
    let deadline = Instant::now() + Duration::from_millis(ms);
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return Ok(status),
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(5)),
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(LibError::new(function, format!("timed out after {} ms", ms)));
            }
            Err(e) => return Err(LibError::new(function, e.to_string())),
        }
    }
}

/// reads a pipe to its end on another thread: empty when the stream was not piped
fn reader<R: Read + Send + 'static>(pipe: Option<R>) -> JoinHandle<Vec<u8>> {
    std::thread::spawn(move || {
        let mut buf = Vec::new();
        if let Some(mut p) = pipe {
            let _ = p.read_to_end(&mut buf);
        }
        buf
    })
}

/// the result as a struct: success, the code, then the output and the error text
fn outcome(status: ExitStatus, out: Vec<u8>, err: Vec<u8>) -> LibValue {
    let code = status.code();
    LibValue::record(
        "CmdResult",
        vec![
            ("success", LibValue::Bool(code == Some(0))),
            ("code", LibValue::Int(code.map(i64::from).unwrap_or(-1))),
            ("output", LibValue::Str(String::from_utf8_lossy(&out).into_owned())),
            ("error", LibValue::Str(String::from_utf8_lossy(&err).into_owned())),
        ],
    )
}

/// safe-party: a run can start any program, so every run is off. shared with `ps.run`
fn refuse_under_safe_party(function: &'static str) -> Result<(), LibError> {
    if crate::ps::safe_party() {
        return Err(LibError::new(
            function,
            "blocked by safe-party: cmd runs other programs, so it is off (cmd.exists still works)",
        ));
    }
    Ok(())
}

fn run(_: &CallCtx, a: Vec<LibValue>) -> Result<LibValue, LibError> {
    refuse_under_safe_party("cmd.run")?;
    execute("cmd.run", text(&a, 0), texts(&a, 1), Options::default())
}

fn run_with_timeout(_: &CallCtx, a: Vec<LibValue>) -> Result<LibValue, LibError> {
    refuse_under_safe_party("cmd.run")?;
    let ms = int(&a, 2);
    if ms <= 0 {
        return Err(LibError::new("cmd.run", "the timeout must be more than 0 milliseconds"));
    }
    let opts = Options { timeout_ms: Some(ms as u64), ..Options::default() };
    execute("cmd.run", text(&a, 0), texts(&a, 1), opts)
}

fn run_with(_: &CallCtx, a: Vec<LibValue>) -> Result<LibValue, LibError> {
    refuse_under_safe_party("cmd.runWith")?;
    let opts = options("cmd.runWith", a.get(2))?;
    execute("cmd.runWith", text(&a, 0), texts(&a, 1), opts)
}

fn exists(_: &CallCtx, a: Vec<LibValue>) -> Result<LibValue, LibError> {
    let name = text(&a, 0);
    Ok(LibValue::Bool(!name.is_empty() && find_program(name).is_some()))
}

/// the file a program name stands for: a path as given, or the first match along PATH.
/// on Windows each place is also tried with the usual extensions (.EXE, .CMD, ...)
fn find_program(name: &str) -> Option<PathBuf> {
    let places: Vec<PathBuf> = if name.contains('/') || name.contains('\\') {
        vec![PathBuf::from(name)]
    } else {
        env::var_os("PATH")
            .map(|path| env::split_paths(&path).map(|dir| dir.join(name)).collect())
            .unwrap_or_default()
    };
    let exts: Vec<String> = if cfg!(windows) {
        env::var("PATHEXT")
            .unwrap_or_else(|_| ".COM;.EXE;.BAT;.CMD".to_string())
            .split(';')
            .filter(|e| !e.is_empty())
            .map(str::to_string)
            .collect()
    } else {
        Vec::new()
    };
    places
        .into_iter()
        .flat_map(|base| {
            let mut tries = vec![base.clone()];
            tries.extend(exts.iter().map(|e| PathBuf::from(format!("{}{}", base.display(), e))));
            tries
        })
        .find(|path| is_program(path))
}

fn is_program(path: &Path) -> bool {
    match fs::metadata(path) {
        Ok(meta) if meta.is_file() => {
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                meta.permissions().mode() & 0o111 != 0
            }
            #[cfg(not(unix))]
            {
                true
            }
        }
        _ => false,
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use crate::{LibFn, LibValue};

    fn ctx() -> CallCtx<'static> {
        CallCtx { caller_file: None, caller_line: 0 }
    }

    fn s(t: &str) -> LibValue {
        LibValue::Str(t.to_string())
    }

    fn sh(script: &str) -> Vec<LibValue> {
        vec![s("sh"), LibValue::Array(vec![s("-c"), s(script)])]
    }

    fn with_opts(mut args: Vec<LibValue>, pairs: &[(&str, &str)]) -> Vec<LibValue> {
        args.push(LibValue::Map(pairs.iter().map(|(k, v)| (s(k), s(v))).collect()));
        args
    }

    fn field(result: &LibValue, name: &str) -> LibValue {
        match result {
            LibValue::Struct { fields, .. } => fields.iter().find(|(n, _)| n == name).map(|(_, v)| v.clone()).unwrap(),
            other => panic!("not a struct: {:?}", other),
        }
    }

    fn call(f: LibFn, args: Vec<LibValue>) -> Result<LibValue, LibError> {
        f(&ctx(), args)
    }

    #[test]
    fn code_and_both_streams_come_back() {
        let r = call(run, sh("echo ola; echo erro 1>&2; exit 3")).unwrap();
        assert_eq!(field(&r, "code"), LibValue::Int(3));
        assert_eq!(field(&r, "success"), LibValue::Bool(false));
        assert_eq!(field(&r, "output"), s("ola\n"));
        assert_eq!(field(&r, "error"), s("erro\n"));
    }

    #[test]
    fn success_is_true_only_for_exit_zero() {
        let r = call(run, sh("exit 0")).unwrap();
        assert_eq!(field(&r, "success"), LibValue::Bool(true));
    }

    #[test]
    fn a_slow_program_is_stopped_by_the_timeout() {
        let mut args = sh("sleep 5");
        args.push(LibValue::Int(200));
        let err = call(run_with_timeout, args).unwrap_err();
        assert!(err.reason.contains("timed out"), "{}", err.reason);
    }

    #[test]
    fn a_missing_program_says_it_could_not_start() {
        let err = call(run, vec![s("programa-que-nao-existe-xyz"), LibValue::Array(vec![])]).unwrap_err();
        assert!(err.reason.contains("could not start"), "{}", err.reason);
    }

    #[test]
    fn input_reaches_the_program() {
        let args = with_opts(vec![s("cat"), LibValue::Array(vec![])], &[("input", "entrada")]);
        let r = call(run_with, args).unwrap();
        assert_eq!(field(&r, "output"), s("entrada"));
    }

    #[test]
    fn the_folder_option_sets_where_it_runs() {
        let args = with_opts(vec![s("pwd"), LibValue::Array(vec![])], &[("dir", "/")]);
        let r = call(run_with, args).unwrap();
        assert_eq!(field(&r, "output"), s("/\n"));
    }

    #[test]
    fn env_options_set_variables_for_the_program() {
        let mut args = sh("echo $SPG_TEST_VALOR");
        args.push(LibValue::Map(vec![(s("env.SPG_TEST_VALOR"), s("ola"))]));
        let r = call(run_with, args).unwrap();
        assert_eq!(field(&r, "output"), s("ola\n"));
    }

    #[test]
    fn the_timeout_option_stops_a_slow_program() {
        let args = with_opts(sh("sleep 5"), &[("timeout", "200")]);
        let err = call(run_with, args).unwrap_err();
        assert!(err.reason.contains("timed out"), "{}", err.reason);
    }

    #[test]
    fn a_misspelled_option_is_an_error_not_ignored() {
        let args = with_opts(sh("true"), &[("dri", "/")]);
        let err = call(run_with, args).unwrap_err();
        assert!(err.reason.contains("unknown option 'dri'"), "{}", err.reason);
    }

    #[test]
    fn a_bad_timeout_value_is_an_error() {
        let err = call(run_with, with_opts(sh("true"), &[("timeout", "dez")])).unwrap_err();
        assert!(err.reason.contains("not a number"), "{}", err.reason);
        let err = call(run_with, with_opts(sh("true"), &[("timeout", "0")])).unwrap_err();
        assert!(err.reason.contains("more than 0"), "{}", err.reason);
    }

    #[test]
    fn exists_finds_a_program_on_path_and_nothing_else() {
        assert_eq!(call(exists, vec![s("sh")]).unwrap(), LibValue::Bool(true));
        assert_eq!(call(exists, vec![s("programa-que-nao-existe-xyz")]).unwrap(), LibValue::Bool(false));
        assert_eq!(call(exists, vec![s("")]).unwrap(), LibValue::Bool(false));
        assert_eq!(call(exists, vec![s("/etc/passwd")]).unwrap(), LibValue::Bool(false), "a file that is not a program");
    }
}
