//! Embeds a build id in the binary, shown by `spg --version`.
//! The short git hash, with `-dirty` when the tree has local changes.
//! Falls back to `unknown` when git is not available (e.g. a source zip).

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
}
