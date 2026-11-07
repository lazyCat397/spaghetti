//! how the window build (`spgw`) deals with the console. on Windows a window application
//! has none of its own, so these helpers attach to the terminal that started it, or send
//! its output to a log file, and show an error box when it fails without a console.
//! other systems always have a console, so these do nothing there.

#[cfg(windows)]
use std::path::PathBuf;
#[cfg(windows)]
use std::sync::OnceLock;

#[cfg(windows)]
static LOG: OnceLock<PathBuf> = OnceLock::new();

/// true when the output has somewhere to go: a console, a redirect or a pipe.
/// false when there is none, and the output goes to the log file instead
#[cfg(windows)]
pub fn attach_or_log() -> bool {
    use std::ffi::c_void;
    use std::ptr::null_mut;

    type Handle = *mut c_void;
    const STD_OUTPUT_HANDLE: u32 = -11i32 as u32;
    const STD_ERROR_HANDLE: u32 = -12i32 as u32;
    const ATTACH_PARENT_PROCESS: u32 = u32::MAX;
    const GENERIC_READ_WRITE: u32 = 0xC000_0000;
    const FILE_SHARE_READ_WRITE: u32 = 0x3;
    const OPEN_EXISTING: u32 = 3;
    const CREATE_ALWAYS: u32 = 2;
    const FILE_ATTRIBUTE_NORMAL: u32 = 0x80;

    extern "system" {
        fn AttachConsole(process_id: u32) -> i32;
        fn GetStdHandle(id: u32) -> Handle;
        fn SetStdHandle(id: u32, handle: Handle) -> i32;
        fn CreateFileW(
            name: *const u16,
            access: u32,
            share: u32,
            security: *mut c_void,
            disposition: u32,
            flags: u32,
            template: Handle,
        ) -> Handle;
    }

    fn usable(handle: Handle) -> bool {
        !handle.is_null() && handle as isize != -1
    }

    unsafe {
        // the output was redirected (or piped) by whoever started us: keep it as it is
        if usable(GetStdHandle(STD_OUTPUT_HANDLE)) {
            return true;
        }
        // a console is there, but this process was started without its handles
        if AttachConsole(ATTACH_PARENT_PROCESS) != 0 {
            let conout = wide("CONOUT$");
            let out = CreateFileW(
                conout.as_ptr(),
                GENERIC_READ_WRITE,
                FILE_SHARE_READ_WRITE,
                null_mut(),
                OPEN_EXISTING,
                0,
                null_mut(),
            );
            if usable(out) {
                SetStdHandle(STD_OUTPUT_HANDLE, out);
                SetStdHandle(STD_ERROR_HANDLE, out);
            }
            return true;
        }
        // no console at all: the output goes to a log file, so the errors can still be read
        let log = log_path();
        let name = wide(&log.display().to_string());
        let file = CreateFileW(
            name.as_ptr(),
            GENERIC_READ_WRITE,
            FILE_SHARE_READ_WRITE,
            null_mut(),
            CREATE_ALWAYS,
            FILE_ATTRIBUTE_NORMAL,
            null_mut(),
        );
        if usable(file) {
            SetStdHandle(STD_OUTPUT_HANDLE, file);
            SetStdHandle(STD_ERROR_HANDLE, file);
        }
        let _ = LOG.set(log);
        false
    }
}

#[cfg(not(windows))]
pub fn attach_or_log() -> bool {
    true
}

/// shows the error box for a program that failed without a console to show it in
#[cfg(windows)]
pub fn report_failure(code: i32) {
    // user32 is not linked by default, unlike kernel32
    #[link(name = "user32")]
    extern "system" {
        fn MessageBoxW(owner: *mut std::ffi::c_void, text: *const u16, caption: *const u16, kind: u32) -> i32;
    }
    const MB_ICONERROR: u32 = 0x10;
    let log = LOG.get().map(|p| p.display().to_string()).unwrap_or_default();
    let text = wide(&format!("the program ended with error code {}.\n\nDetails: {}", code, log));
    let caption = wide("spg");
    unsafe {
        MessageBoxW(std::ptr::null_mut(), text.as_ptr(), caption.as_ptr(), MB_ICONERROR);
    }
}

#[cfg(not(windows))]
pub fn report_failure(_code: i32) {}

#[cfg(windows)]
fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}

/// `<exe name>.log` in the temp folder: the output of a window program run without a console
#[cfg(windows)]
fn log_path() -> PathBuf {
    let stem = std::env::current_exe()
        .ok()
        .and_then(|p| p.file_stem().map(|s| s.to_string_lossy().into_owned()))
        .unwrap_or_else(|| "spg".to_string());
    std::env::temp_dir().join(format!("{}.log", stem))
}
