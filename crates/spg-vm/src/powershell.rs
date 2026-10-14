//! `ps.*`: PowerShell, run as a child process. The Spaghetti side lives in
//! `stdlib/powershell.spg`; this file only starts processes and reports what
//! they did, so the .spg wrapper stays a thin layer over these builtins.

use std::io::Read;
use std::process::{Command, Stdio};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use crate::error::RuntimeError;

/// every run is forced to UTF-8, both for what PowerShell prints and for
/// what it reads from the script text, so accents survive on Windows
const UTF8_PRELUDE: &str = "[Console]::OutputEncoding = [Text.Encoding]::UTF8; $OutputEncoding = [Text.Encoding]::UTF8; ";

/// the first executable that starts and exits cleanly: `pwsh` (7+) before
/// `powershell.exe` (5.1, always present on Windows). Found once per process.
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

/// what a finished (or killed) run reported
pub struct Outcome {
    pub code: i64,
    pub output: String,
    pub error: String,
}

/// `mode` is "cmd" (`arg` is script text) or "file" (`arg` is a .ps1 path).
/// Returns `Err` only when PowerShell itself cannot run; a command that runs
/// and fails, or that times out, comes back as an `Outcome`.
pub fn exec(function: &'static str, mode: &str, arg: &str, timeout_secs: i64) -> Result<Outcome, RuntimeError> {
    if timeout_secs <= 0 {
        return Err(RuntimeError::Process { function, reason: "the timeout must be greater than 0 seconds".to_string() });
    }
    let exe = find_exe().ok_or_else(|| RuntimeError::Process {
        function,
        reason: "PowerShell not found (tried pwsh and powershell)".to_string(),
    })?;
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
        .map_err(|e| RuntimeError::Process { function, reason: format!("could not start {exe}: {e}") })?;

    // read both pipes on their own threads, or a chatty script could fill
    // a pipe and block forever while we wait for it to exit
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
            Err(e) => return Err(RuntimeError::Process { function, reason: format!("lost the process: {e}") }),
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

/// true only when PowerShell starts and answers a trivial command with exit 0
pub fn is_available() -> bool {
    match exec("ps.isAvailable", "cmd", "$PSVersionTable.PSVersion.ToString()", 10) {
        Ok(done) => done.code == 0 && !done.output.trim().is_empty(),
        Err(_) => false,
    }
}

/// `#> safe-party`: set once by the CLI from the entry file's header
static SAFE_PARTY: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

pub fn set_safe_party(on: bool) {
    SAFE_PARTY.store(on, std::sync::atomic::Ordering::Relaxed);
}

pub fn safe_party() -> bool {
    SAFE_PARTY.load(std::sync::atomic::Ordering::Relaxed)
}

/// the version of the PowerShell that `exec` would use
pub fn version() -> Result<String, RuntimeError> {
    let outcome = exec("ps.version", "cmd", "$PSVersionTable.PSVersion.ToString()", 10)?;
    let text = outcome.output.trim().to_string();
    if outcome.code == 0 && !text.is_empty() {
        Ok(text)
    } else {
        Err(RuntimeError::Process {
            function: "ps.version",
            reason: format!("PowerShell did not report a version (exit code {})", outcome.code),
        })
    }
}

/// a PowerShell single-quoted literal: quotes are doubled, and the typographic
/// quotes PowerShell also accepts are doubled too, so the path is never split
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
