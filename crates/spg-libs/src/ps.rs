//! `ps`: PowerShell, run as a child process (`install #ps`). this file starts the
//! processes and reports what they did, and declares the library for the registry.

use std::io::Read;
use std::process::{Command, Stdio};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use crate::{CallCtx, Kind, LibError, LibValue, NativeFn, NativeModule};

pub static MODULE: NativeModule = NativeModule {
    kind: Kind::Install,
    name: "ps",
    doc: "PowerShell (install #ps)",
    structs: &["struct PsResult {\n    success: bool,\n    code: int,\n    output: string,\n    error: string\n}\n"],
    objects: &[],
    functions: &[
        NativeFn { name: "run", sig: "(string) -> PsResult", doc: "run a PowerShell command. gives back a PsResult", call: run },
        NativeFn { name: "run", sig: "(string, int) -> PsResult", doc: "run a PowerShell command. gives back a PsResult", call: run },
        NativeFn { name: "runFile", sig: "(string) -> PsResult", doc: "run a .ps1 script. gives back a PsResult", call: run_file },
        NativeFn { name: "runFile", sig: "(string, int) -> PsResult", doc: "run a .ps1 script. gives back a PsResult", call: run_file },
        NativeFn { name: "version", sig: "() -> string", doc: "the PowerShell version, as text", call: version_call },
        NativeFn { name: "isAvailable", sig: "() -> bool", doc: "true if PowerShell can be found on this machine", call: is_available_call },
    ],
};

fn run(ctx: &CallCtx, args: Vec<LibValue>) -> Result<LibValue, LibError> {
    run_script(ctx, args, false)
}

fn run_file(ctx: &CallCtx, args: Vec<LibValue>) -> Result<LibValue, LibError> {
    run_script(ctx, args, true)
}

fn version_call(_: &CallCtx, _: Vec<LibValue>) -> Result<LibValue, LibError> {
    version().map(LibValue::Str)
}

fn is_available_call(_: &CallCtx, _: Vec<LibValue>) -> Result<LibValue, LibError> {
    Ok(LibValue::Bool(is_available()))
}

/// `ps.run(cmd [, timeout])` and `ps.runFile(path [, timeout])`: builds the `PsResult`
/// (success, code, output, error). in that order, don't swap.
fn run_script(ctx: &CallCtx, args: Vec<LibValue>, file: bool) -> Result<LibValue, LibError> {
    let function = if file { "ps.runFile" } else { "ps.run" };
    let text = match args.first() {
        Some(LibValue::Str(s)) => s.clone(),
        _ => return Err(LibError::new(function, "expected a string")),
    };
    let timeout = match args.get(1) {
        Some(LibValue::Int(n)) => *n,
        _ => 30,
    };
    // safe-party: `ps.run` runs any command, so it is off entirely. `runFile` stays,
    // but only for a .ps1 inside the script's folder (checked below)
    if safe_party() && !file {
        return Err(LibError::new(
            function,
            "blocked by safe-party: ps.run is off (use ps.runFile with a .ps1 inside the script's folder)",
        ));
    }
    let (mode, arg) = if file {
        let path = std::path::Path::new(&text);
        let escapes = path.components().any(|c| {
            matches!(c, std::path::Component::ParentDir | std::path::Component::RootDir | std::path::Component::Prefix(_))
        });
        if safe_party() && escapes {
            return Err(LibError::new(
                function,
                "blocked by safe-party: runFile only takes a .ps1 inside the script's folder (no absolute paths, no ..)",
            ));
        }
        // the folder of the script that made the call. relative paths are a pain.
        let resolved = if path.is_absolute() {
            path.to_path_buf()
        } else {
            ctx.caller_file
                .map(|file| std::path::Path::new(file).parent().unwrap_or(std::path::Path::new(".")).join(path))
                .unwrap_or_else(|| path.to_path_buf())
        };
        ("file", resolved.to_string_lossy().into_owned())
    } else {
        ("cmd", text)
    };
    let done = exec(function, mode, &arg, timeout)?;
    Ok(LibValue::record(
        "PsResult",
        vec![
            ("success", LibValue::Bool(done.code == 0)),
            ("code", LibValue::Int(done.code)),
            ("output", LibValue::Str(done.output)),
            ("error", LibValue::Str(done.error)),
        ],
    ))
}

/// every run is forced to UTF-8, both for what PowerShell prints and
/// what it reads from the script, so accents survive on Windows. (ç, ã, you know.)
const UTF8_PRELUDE: &str = "[Console]::OutputEncoding = [Text.Encoding]::UTF8; $OutputEncoding = [Text.Encoding]::UTF8; ";

/// the first executable that starts and exits cleanly: `pwsh` (7+) before
/// `powershell.exe` (5.1, always there on Windows). found once per process. cached.
static EXE: OnceLock<Option<&'static str>> = OnceLock::new();

fn find_exe() -> Option<&'static str> {
    *EXE.get_or_init(|| {
        ["pwsh", "powershell"].into_iter().find(|exe| {
            Command::new(exe)
                .args(["-NoProfile", "-NonInteractive", "-NoLogo", "-Command", "exit 0"])
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .map(|status| status.success())
                .unwrap_or(false)
        })
    })
}

/// what a finished (or killed) run reported. the receipts.
pub struct Outcome {
    pub code: i64,
    pub output: String,
    pub error: String,
}

/// `mode` is "cmd" (`arg` is script text) or "file" (`arg` is a .ps1 path).
/// returns `Err` only when PowerShell itself can't run. a command that runs
/// and fails, or times out, comes back as an `Outcome`. failing is not an error here.
pub fn exec(function: &'static str, mode: &str, arg: &str, timeout_secs: i64) -> Result<Outcome, LibError> {
    if timeout_secs <= 0 {
        return Err(LibError { function, reason: "the timeout must be greater than 0 seconds".to_string(), path: None });
    }
    let exe = find_exe().ok_or_else(|| LibError {
        function,
        reason: "PowerShell not found (tried pwsh and powershell)".to_string(), path: None })?;
    let script = match mode {
        "file" => format!("{UTF8_PRELUDE}& {}", quote(arg)),
        _ => format!("{UTF8_PRELUDE}{arg}"),
    };
    let mut child = Command::new(exe)
        .args(["-NoProfile", "-NonInteractive", "-NoLogo", "-OutputFormat", "Text", "-Command"])
        .arg(&script)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| LibError { function, reason: format!("could not start {exe}: {e}"), path: None })?;

    // read both pipes on their own threads, or a chatty script fills
    // a pipe and deadlocks us forever while we wait for it. classic.
    let mut out_pipe = child.stdout.take().expect("stdout was piped");
    let mut err_pipe = child.stderr.take().expect("stderr was piped");
    let out_reader = std::thread::spawn(move || {
        let mut bytes = Vec::new();
        let _ = out_pipe.read_to_end(&mut bytes);
        bytes
    });
    let err_reader = std::thread::spawn(move || {
        let mut bytes = Vec::new();
        let _ = err_pipe.read_to_end(&mut bytes);
        bytes
    });

    let deadline = Instant::now() + Duration::from_secs(timeout_secs as u64);
    let (code, timed_out) = loop {
        match child.try_wait() {
            Ok(Some(status)) => break (i64::from(status.code().unwrap_or(-1)), false),
            Ok(None) if Instant::now() >= deadline => {
                let _ = child.kill();
                let _ = child.wait();
                break (-1, true);
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(10)),
            Err(e) => return Err(LibError { function, reason: format!("lost the process: {e}"), path: None }),
        }
    };

    let output = String::from_utf8_lossy(&out_reader.join().unwrap_or_default()).into_owned();
    let error = if timed_out {
        format!("timed out after {timeout_secs}s")
    } else {
        String::from_utf8_lossy(&err_reader.join().unwrap_or_default()).into_owned()
    };
    Ok(Outcome { code, output, error })
}

/// true only when PowerShell starts and answers a trivial command with exit 0. vibes check.
pub fn is_available() -> bool {
    match exec("ps.isAvailable", "cmd", "$PSVersionTable.PSVersion.ToString()", 10) {
        Ok(done) => done.code == 0 && !done.output.trim().is_empty(),
        Err(_) => false,
    }
}

/// `#> safe-party`: set once by the CLI from the entry file's header. party responsibly.
static SAFE_PARTY: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

pub fn set_safe_party(on: bool) {
    SAFE_PARTY.store(on, std::sync::atomic::Ordering::Relaxed);
}

pub fn safe_party() -> bool {
    SAFE_PARTY.load(std::sync::atomic::Ordering::Relaxed)
}

/// the version of the PowerShell that `exec` would pick
pub fn version() -> Result<String, LibError> {
    let outcome = exec("ps.version", "cmd", "$PSVersionTable.PSVersion.ToString()", 10)?;
    let text = outcome.output.trim().to_string();
    if outcome.code == 0 && !text.is_empty() {
        Ok(text)
    } else {
        Err(LibError {
            function: "ps.version",
            reason: format!("PowerShell did not report a version (exit code {})", outcome.code), path: None })
    }
}

/// a PowerShell single-quoted literal: quotes get doubled, and the typographic
/// quotes PowerShell also accepts get doubled too, so the path never splits
fn quote(text: &str) -> String {
    let mut out = String::from("'");
    for c in text.chars() {
        match c {
            '\'' | '\u{2018}' | '\u{2019}' | '\u{201A}' | '\u{201B}' => out.push_str("''"),
            _ => out.push(c),
        }
    }
    out.push('\'');
    out
}

#[cfg(test)]
mod tests {
    use super::quote;

    #[test]
    fn quote_doubles_every_kind_of_single_quote() {
        assert_eq!(quote("C:\\Users\\it's"), "'C:\\Users\\it''s'");
        assert_eq!(quote("a\u{2019}b"), "'a''b'");
    }
}
