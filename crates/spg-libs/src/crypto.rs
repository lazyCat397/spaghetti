//! `crypto`: hashes and text encodings (`install #crypto`). a text is hashed and encoded as its
//! UTF-8 bytes. hashes come back as lowercase hex. decoding gives text, so bytes that are not
//! valid UTF-8 are an error, not a garbled string.

use base64::Engine as _;
use md5::Md5;
use sha2::{Digest, Sha256};

use crate::{CallCtx, Kind, LibError, LibValue, NativeFn, NativeModule};

pub static MODULE: NativeModule = NativeModule {
    kind: Kind::Install,
    name: "crypto",
    doc: "hashes and encodings: sha256, md5, base64, hex (install #crypto)",
    structs: &[],
    objects: &[],
    functions: &[
        NativeFn { name: "sha256", sig: "(string) -> string", doc: "SHA-256 of the text, as hex", call: sha256 },
        NativeFn { name: "md5", sig: "(string) -> string", doc: "MD5 of the text, as hex. fine for checksums, never for passwords", call: md5 },
        NativeFn { name: "base64Encode", sig: "(string) -> string", doc: "the text as base64", call: base64_encode },
        NativeFn { name: "base64Decode", sig: "(string) -> string", doc: "the text a base64 string stands for (error if it is not valid base64)", call: base64_decode },
        NativeFn { name: "hexEncode", sig: "(string) -> string", doc: "the text as hex, two digits per byte", call: hex_encode },
        NativeFn { name: "hexDecode", sig: "(string) -> string", doc: "the text a hex string stands for (error if it is not valid hex)", call: hex_decode },
    ],
};

fn text(a: &[LibValue]) -> &str {
    match a.first() {
        Some(LibValue::Str(t)) => t.as_str(),
        _ => "",
    }
}

/// bytes that should be text: valid UTF-8 or an error
fn as_text(function: &'static str, bytes: Vec<u8>) -> Result<LibValue, LibError> {
    String::from_utf8(bytes)
        .map(LibValue::Str)
        .map_err(|_| LibError::new(function, "the decoded bytes are not UTF-8 text"))
}

fn sha256(_: &CallCtx, a: Vec<LibValue>) -> Result<LibValue, LibError> {
    Ok(LibValue::Str(hex::encode(Sha256::digest(text(&a).as_bytes()))))
}

fn md5(_: &CallCtx, a: Vec<LibValue>) -> Result<LibValue, LibError> {
    Ok(LibValue::Str(hex::encode(Md5::digest(text(&a).as_bytes()))))
}

fn base64_encode(_: &CallCtx, a: Vec<LibValue>) -> Result<LibValue, LibError> {
    Ok(LibValue::Str(base64::engine::general_purpose::STANDARD.encode(text(&a).as_bytes())))
}

fn base64_decode(_: &CallCtx, a: Vec<LibValue>) -> Result<LibValue, LibError> {
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(text(&a))
        .map_err(|e| LibError::new("crypto.base64Decode", format!("not valid base64: {}", e)))?;
    as_text("crypto.base64Decode", bytes)
}

fn hex_encode(_: &CallCtx, a: Vec<LibValue>) -> Result<LibValue, LibError> {
    Ok(LibValue::Str(hex::encode(text(&a).as_bytes())))
}

fn hex_decode(_: &CallCtx, a: Vec<LibValue>) -> Result<LibValue, LibError> {
    let bytes = hex::decode(text(&a)).map_err(|e| LibError::new("crypto.hexDecode", format!("not valid hex: {}", e)))?;
    as_text("crypto.hexDecode", bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::LibFn;

    fn call(f: LibFn, s: &str) -> Result<LibValue, LibError> {
        f(&CallCtx { caller_file: None, caller_line: 0 }, vec![LibValue::Str(s.to_string())])
    }

    fn out(f: LibFn, s: &str) -> String {
        match call(f, s) {
            Ok(LibValue::Str(t)) => t,
            other => panic!("unexpected {:?}", other),
        }
    }

    #[test]
    fn hashes_match_the_published_test_vectors() {
        assert_eq!(out(sha256, "abc"), "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
        assert_eq!(out(md5, "abc"), "900150983cd24fb0d6963f7d28e17f72");
    }

    #[test]
    fn encodings_round_trip_and_reject_garbage() {
        assert_eq!(out(base64_encode, "hello"), "aGVsbG8=");
        assert_eq!(out(base64_decode, "aGVsbG8="), "hello");
        assert_eq!(out(hex_encode, "hi"), "6869");
        assert_eq!(out(hex_decode, "6869"), "hi");
        assert!(call(base64_decode, "!!!").is_err());
        assert!(call(hex_decode, "zz").is_err());
        // "/w==" is one byte 0xFF: valid base64, not valid UTF-8 text
        assert!(call(base64_decode, "/w==").is_err());
    }
}
