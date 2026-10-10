//! `file://` URI <-> filesystem path, including the Windows shapes VSCode
//! sends (`file:///c%3A/dir/a.spg`). Pure string work, so the Windows
//! cases are unit-tested on any OS.

use std::path::{Path, PathBuf};

pub fn uri_to_path(uri: &str) -> Option<PathBuf> {
    let rest = uri.strip_prefix("file://")?;
    // authority (empty, or `localhost`) sits before the first '/'
    let rest = if rest.starts_with('/') {
        rest
    } else {
        &rest[rest.find('/')?..]
    };
    let decoded = percent_decode(rest);
    let b = decoded.as_bytes();
    // "/c:/dir" -> "c:/dir"  (drive letter form)
    if b.len() >= 3 && b[0] == b'/' && b[1].is_ascii_alphabetic() && b[2] == b':' {
        return Some(PathBuf::from(decoded[1..].to_string()));
    }
    Some(PathBuf::from(decoded))
}

pub fn path_to_uri(path: &Path) -> String {
    let s = path.to_string_lossy().replace('\\', "/");
    let b = s.as_bytes();
    if b.len() >= 2 && b[0].is_ascii_alphabetic() && b[1] == b':' {
        // VSCode writes the drive lowercase with an encoded colon
        let drive = (b[0] as char).to_ascii_lowercase();
        format!("file:///{}%3A{}", drive, percent_encode(&s[2..]))
    } else if s.starts_with('/') {
        format!("file://{}", percent_encode(&s))
    } else {
        format!("file:///{}", percent_encode(&s))
    }
}

fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            let h = std::str::from_utf8(&b[i + 1..i + 3]).ok().and_then(|h| u8::from_str_radix(h, 16).ok());
            if let Some(v) = h {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn percent_encode(s: &str) -> String {
    let mut out = String::new();
    for &b in s.as_bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~' | b'/') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{:02X}", b));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn windows_uri_to_path() {
        let p = uri_to_path("file:///c%3A/spaghetti-prot/a%20b.spg").unwrap();
        assert_eq!(p, PathBuf::from("c:/spaghetti-prot/a b.spg"));
    }

    #[test]
    fn unix_uri_to_path() {
        assert_eq!(uri_to_path("file:///home/x/a.spg").unwrap(), PathBuf::from("/home/x/a.spg"));
        assert_eq!(uri_to_path("file://localhost/home/x").unwrap(), PathBuf::from("/home/x"));
        assert!(uri_to_path("untitled:Untitled-1").is_none());
    }

    #[test]
    fn windows_path_to_uri() {
        let u = path_to_uri(Path::new(r"C:\spaghetti-prot\a b.spg"));
        assert_eq!(u, "file:///c%3A/spaghetti-prot/a%20b.spg");
    }

    #[test]
    fn roundtrips_with_accents() {
        for p in ["/home/u/ação/olá.spg", "c:/spaghetti-prot/ção.spg"] {
            let back = uri_to_path(&path_to_uri(Path::new(p))).unwrap();
            assert_eq!(back, PathBuf::from(p), "roundtrip {}", p);
        }
    }

    #[test]
    fn stray_percent_is_kept() {
        assert_eq!(uri_to_path("file:///a/100%25/b%").unwrap(), PathBuf::from("/a/100%/b%"));
    }
}
