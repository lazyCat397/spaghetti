//! `spg build`: a standalone executable is the `spg` binary itself with the
//! program's sources appended to it. At startup `read_own` looks for that
//! payload at the very end of the running executable; when it is there, the
//! program runs from the embedded sources and nothing is read from disk.
//!
//! Layout appended to the executable:
//!
//! ```text
//! payload:  u32 file count, then per file: u32 path length, path bytes,
//!           u32 source length, source bytes   (the entry file comes first)
//! trailer:  u64 payload length (little endian), then the 8 bytes "SPGBUNDL"
//! ```

use std::collections::HashMap;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

const MAGIC: &[u8; 8] = b"SPGBUNDL";
const TRAILER_LEN: u64 = 16;

/// the sources of a bundled program, entry file first
pub struct Bundle {
    pub files: Vec<(PathBuf, String)>,
}

impl Bundle {
    pub fn entry(&self) -> PathBuf {
        self.files[0].0.clone()
    }

    /// every file as an in-memory overlay for the module resolver
    pub fn overlay(&self) -> HashMap<PathBuf, String> {
        self.files.iter().cloned().collect()
    }
}

/// writes `exe` (the spg binary) followed by the payload of `files` to `out`
pub fn write(out: &Path, exe: &[u8], files: &[(PathBuf, String)]) -> std::io::Result<()> {
    let mut payload: Vec<u8> = Vec::new();
    payload.extend_from_slice(&(files.len() as u32).to_le_bytes());
    for (path, source) in files {
        let path = path.to_string_lossy();
        payload.extend_from_slice(&(path.len() as u32).to_le_bytes());
        payload.extend_from_slice(path.as_bytes());
        payload.extend_from_slice(&(source.len() as u32).to_le_bytes());
        payload.extend_from_slice(source.as_bytes());
    }

    let mut file = std::fs::File::create(out)?;
    file.write_all(exe)?;
    file.write_all(&payload)?;
    file.write_all(&(payload.len() as u64).to_le_bytes())?;
    file.write_all(MAGIC)?;
    drop(file);

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(out, std::fs::Permissions::from_mode(0o755))?;
    }
    Ok(())
}

/// the bundle this executable carries, if it is one. Any problem reading the
/// executable simply means "not a bundle": the normal CLI then takes over.
pub fn read_own() -> Option<Bundle> {
    let exe = std::env::current_exe().ok()?;
    let mut file = std::fs::File::open(exe).ok()?;
    let len = file.seek(SeekFrom::End(0)).ok()?;
    if len < TRAILER_LEN {
        return None;
    }

    file.seek(SeekFrom::End(-(TRAILER_LEN as i64))).ok()?;
    let mut trailer = [0u8; TRAILER_LEN as usize];
    file.read_exact(&mut trailer).ok()?;
    if &trailer[8..] != MAGIC {
        return None;
    }
    let payload_len = u64::from_le_bytes(trailer[..8].try_into().ok()?);
    if payload_len > len - TRAILER_LEN {
        return None;
    }

    file.seek(SeekFrom::Start(len - TRAILER_LEN - payload_len)).ok()?;
    let mut payload = vec![0u8; payload_len as usize];
    file.read_exact(&mut payload).ok()?;
    parse(&payload)
}

fn parse(payload: &[u8]) -> Option<Bundle> {
    let mut at = 0usize;
    let count = read_u32(payload, &mut at)?;
    if count == 0 {
        return None;
    }
    let mut files = Vec::with_capacity(count);
    for _ in 0..count {
        let path_len = read_u32(payload, &mut at)?;
        let path = String::from_utf8(read_bytes(payload, &mut at, path_len)?.to_vec()).ok()?;
        let source_len = read_u32(payload, &mut at)?;
        let source = String::from_utf8(read_bytes(payload, &mut at, source_len)?.to_vec()).ok()?;
        files.push((PathBuf::from(path), source));
    }
    Some(Bundle { files })
}

fn read_u32(payload: &[u8], at: &mut usize) -> Option<usize> {
    let bytes = read_bytes(payload, at, 4)?;
    Some(u32::from_le_bytes(bytes.try_into().ok()?) as usize)
}

fn read_bytes<'a>(payload: &'a [u8], at: &mut usize, n: usize) -> Option<&'a [u8]> {
    let end = at.checked_add(n)?;
    let slice = payload.get(*at..end)?;
    *at = end;
    Some(slice)
}
