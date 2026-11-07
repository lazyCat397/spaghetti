//! Embeds a build id in the binary, shown by `spg --version`.
//! The short git hash, with `-dirty` when the tree has local changes.
//! Falls back to `unknown` when git is not available (e.g. a source zip).
//!
//! On Windows, also gives the window build (`spgw`) its icon as a resource, so the file,
//! the shortcut and the taskbar show it without any runtime loading.

use std::path::PathBuf;
use std::process::Command;

fn git(args: &[&str]) -> Option<String> {
    let out = Command::new("git").args(args).output().ok()?;
    if !out.status.success() {
        return None;
    }
    let text = String::from_utf8(out.stdout).ok()?;
    let text = text.trim().to_string();
    if text.is_empty() { None } else { Some(text) }
}

fn main() {
    let mut id = git(&["rev-parse", "--short", "HEAD"]).unwrap_or_else(|| "unknown".to_string());
    if id != "unknown" && git(&["status", "--porcelain"]).is_some() {
        id.push_str("-dirty");
    }
    println!("cargo:rustc-env=SPG_BUILD_ID={id}");
    println!("cargo:rerun-if-changed=../../.git/HEAD");
    println!("cargo:rerun-if-changed=../../.git/index");
    println!("cargo:rerun-if-changed=build.rs");
    embed_window_icon();
}

/// the icon of `spgw.exe`: the `.ico` in assets/ compiled into a resource and linked only into
/// that binary. the MSVC build uses `rc` (Windows SDK, or the RC variable), the GNU one `windres`
/// (or the WINDRES variable). other targets have no icon resource, so nothing happens there.
fn embed_window_icon() {
    println!("cargo:rerun-if-env-changed=RC");
    println!("cargo:rerun-if-env-changed=WINDRES");
    println!("cargo:rerun-if-changed=assets/spgw.ico");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let manifest = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").expect("cargo sets CARGO_MANIFEST_DIR"));
    let icon = manifest.join("assets").join("spgw.ico");
    let out = PathBuf::from(std::env::var("OUT_DIR").expect("cargo sets OUT_DIR"));
    let rc_file = out.join("spgw.rc");
    // the path goes into a resource string: backslashes must be doubled
    let icon_text = icon.display().to_string().replace('\\', "\\\\");
    std::fs::write(&rc_file, format!("1 ICON \"{}\"\n", icon_text)).expect("write the icon resource script");

    let env = std::env::var("CARGO_CFG_TARGET_ENV").unwrap_or_default();
    let object = if env == "msvc" {
        let res = out.join("spgw.res");
        let rc = std::env::var("RC").unwrap_or_else(|_| "rc".to_string());
        run(&rc, &["/nologo".into(), format!("/fo{}", res.display()), rc_file.display().to_string()]);
        res
    } else {
        let obj = out.join("spgw.o");
        let windres = std::env::var("WINDRES").unwrap_or_else(|_| "windres".to_string());
        run(&windres, &[rc_file.display().to_string(), "-O".into(), "coff".into(), "-o".into(), obj.display().to_string()]);
        obj
    };
    // only the window build gets the icon: the console one stays as it is
    println!("cargo:rustc-link-arg-bin=spgw={}", object.display());
}

fn run(tool: &str, args: &[String]) {
    let status = Command::new(tool).args(args).status().unwrap_or_else(|e| {
        panic!(
            "could not run '{}' to build the icon of spgw ({}). Install the Windows SDK \
             (rc.exe, or set RC) or mingw's windres (or set WINDRES)",
            tool, e
        )
    });
    if !status.success() {
        panic!("'{}' failed to compile the icon resource of spgw", tool);
    }
}

