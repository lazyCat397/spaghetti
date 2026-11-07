//! `crypto`: hashes, keyed hashes, encodings, random values and password encryption
//! (`install #crypto`). a text is hashed and encoded as its UTF-8 bytes. hashes come back as
//! lowercase hex. decoding gives text, so bytes that are not valid UTF-8 are an error, not a
//! garbled string.
//!
//! encryption (`encrypt` / `decrypt`): the key comes from the password through Argon2id, and
//! the text is sealed with AES-256-GCM, so a wrong password or a changed byte is detected.
//! the output is base64 of: one format byte, a random 16-byte salt, a random 12-byte nonce,
//! then the ciphertext with its tag. every call gets a new salt and nonce, so the same text
//! encrypted twice gives two different outputs.

use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Nonce};
use argon2::{Algorithm, Argon2, Params, Version};
use base64::Engine as _;
use hmac::{Hmac, Mac};
use md5::Md5;
use sha1::Sha1;
use sha2::{Digest, Sha256, Sha512};

use crate::{CallCtx, Kind, LibError, LibValue, NativeFn, NativeModule};

pub static MODULE: NativeModule = NativeModule {
    kind: Kind::Install,
    name: "crypto",
    doc: "hashes, HMAC, encodings, random values and password encryption (install #crypto)",
    structs: &[],
    objects: &[],
    functions: &[
        NativeFn { name: "sha1", sig: "(string) -> string", doc: "SHA-1 of the text, as hex. weak: only for compatibility, never for security", call: sha1 },
        NativeFn { name: "sha256", sig: "(string) -> string", doc: "SHA-256 of the text, as hex", call: sha256 },
        NativeFn { name: "sha512", sig: "(string) -> string", doc: "SHA-512 of the text, as hex", call: sha512 },
        NativeFn { name: "md5", sig: "(string) -> string", doc: "MD5 of the text, as hex. fine for checksums, never for passwords", call: md5 },
        NativeFn { name: "hmacSha256", sig: "(string, string) -> string", doc: "HMAC-SHA256 of the text with the key (key first), as hex. for signing and checking messages", call: hmac_sha256 },
        NativeFn { name: "hmacSha512", sig: "(string, string) -> string", doc: "HMAC-SHA512 of the text with the key (key first), as hex", call: hmac_sha512 },
        NativeFn { name: "base64Encode", sig: "(string) -> string", doc: "the text as base64", call: base64_encode },
        NativeFn { name: "base64Decode", sig: "(string) -> string", doc: "the text a base64 string stands for (error if it is not valid base64)", call: base64_decode },
        NativeFn { name: "base64UrlEncode", sig: "(string) -> string", doc: "the text as URL-safe base64 (- and _ instead of + and /)", call: base64url_encode },
        NativeFn { name: "base64UrlDecode", sig: "(string) -> string", doc: "the text a URL-safe base64 string stands for (error if it is not valid)", call: base64url_decode },
        NativeFn { name: "hexEncode", sig: "(string) -> string", doc: "the text as hex, two digits per byte", call: hex_encode },
        NativeFn { name: "hexDecode", sig: "(string) -> string", doc: "the text a hex string stands for (error if it is not valid hex)", call: hex_decode },
        NativeFn { name: "randomHex", sig: "(int) -> string", doc: "that many random bytes, as hex (twice as many characters). 1 to 4096 bytes", call: random_hex },
        NativeFn { name: "uuid", sig: "() -> string", doc: "a random UUID (version 4), as text", call: uuid },
        NativeFn { name: "encrypt", sig: "(string, string) -> string", doc: "encrypts the text with the password (AES-256-GCM, key from Argon2id). gives base64 text", call: encrypt },
        NativeFn { name: "decrypt", sig: "(string, string) -> string", doc: "the text that encrypt sealed, given the same password. wrong password or changed data is an error", call: decrypt },
    ],
};

/// the first byte of every encrypted text: the layout version, so a later format can be told apart
const FORMAT: u8 = 1;
const SALT_LEN: usize = 16;
const NONCE_LEN: usize = 12;
const TAG_LEN: usize = 16;
/// the smallest possible encrypted text: a header and a tag, with nothing sealed inside
const MIN_LEN: usize = 1 + SALT_LEN + NONCE_LEN + TAG_LEN;
/// the largest number of random bytes one call gives back
const MAX_RANDOM: i64 = 4096;

/// the text at position `i`, or "" when there is none
fn text_at(a: &[LibValue], i: usize) -> &str {
    match a.get(i) {
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

fn random_bytes(function: &'static str, buf: &mut [u8]) -> Result<(), LibError> {
    getrandom::getrandom(buf).map_err(|e| LibError::new(function, format!("no random source: {}", e)))
}

fn sha1(_: &CallCtx, a: Vec<LibValue>) -> Result<LibValue, LibError> {
    Ok(LibValue::Str(hex::encode(Sha1::digest(text_at(&a, 0).as_bytes()))))
}

fn sha256(_: &CallCtx, a: Vec<LibValue>) -> Result<LibValue, LibError> {
    Ok(LibValue::Str(hex::encode(Sha256::digest(text_at(&a, 0).as_bytes()))))
}

fn sha512(_: &CallCtx, a: Vec<LibValue>) -> Result<LibValue, LibError> {
    Ok(LibValue::Str(hex::encode(Sha512::digest(text_at(&a, 0).as_bytes()))))
}

fn md5(_: &CallCtx, a: Vec<LibValue>) -> Result<LibValue, LibError> {
    Ok(LibValue::Str(hex::encode(Md5::digest(text_at(&a, 0).as_bytes()))))
}

fn hmac_sha256(_: &CallCtx, a: Vec<LibValue>) -> Result<LibValue, LibError> {
    let mut mac = <Hmac<Sha256> as Mac>::new_from_slice(text_at(&a, 0).as_bytes())
        .map_err(|_| LibError::new("crypto.hmacSha256", "the key could not be used"))?;
    mac.update(text_at(&a, 1).as_bytes());
    Ok(LibValue::Str(hex::encode(mac.finalize().into_bytes())))
}

fn hmac_sha512(_: &CallCtx, a: Vec<LibValue>) -> Result<LibValue, LibError> {
    let mut mac = <Hmac<Sha512> as Mac>::new_from_slice(text_at(&a, 0).as_bytes())
        .map_err(|_| LibError::new("crypto.hmacSha512", "the key could not be used"))?;
    mac.update(text_at(&a, 1).as_bytes());
    Ok(LibValue::Str(hex::encode(mac.finalize().into_bytes())))
}

fn base64_encode(_: &CallCtx, a: Vec<LibValue>) -> Result<LibValue, LibError> {
    Ok(LibValue::Str(base64::engine::general_purpose::STANDARD.encode(text_at(&a, 0).as_bytes())))
}

fn base64_decode(_: &CallCtx, a: Vec<LibValue>) -> Result<LibValue, LibError> {
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(text_at(&a, 0))
        .map_err(|e| LibError::new("crypto.base64Decode", format!("not valid base64: {}", e)))?;
    as_text("crypto.base64Decode", bytes)
}

fn base64url_encode(_: &CallCtx, a: Vec<LibValue>) -> Result<LibValue, LibError> {
    Ok(LibValue::Str(base64::engine::general_purpose::URL_SAFE.encode(text_at(&a, 0).as_bytes())))
}

fn base64url_decode(_: &CallCtx, a: Vec<LibValue>) -> Result<LibValue, LibError> {
    let bytes = base64::engine::general_purpose::URL_SAFE
        .decode(text_at(&a, 0))
        .map_err(|e| LibError::new("crypto.base64UrlDecode", format!("not valid URL-safe base64: {}", e)))?;
    as_text("crypto.base64UrlDecode", bytes)
}

fn hex_encode(_: &CallCtx, a: Vec<LibValue>) -> Result<LibValue, LibError> {
    Ok(LibValue::Str(hex::encode(text_at(&a, 0).as_bytes())))
}

fn hex_decode(_: &CallCtx, a: Vec<LibValue>) -> Result<LibValue, LibError> {
    let bytes = hex::decode(text_at(&a, 0)).map_err(|e| LibError::new("crypto.hexDecode", format!("not valid hex: {}", e)))?;
    as_text("crypto.hexDecode", bytes)
}

fn random_hex(_: &CallCtx, a: Vec<LibValue>) -> Result<LibValue, LibError> {
    let n = match a.first() {
        Some(LibValue::Int(n)) => *n,
        _ => 0,
    };
    if !(1..=MAX_RANDOM).contains(&n) {
        return Err(LibError::new("crypto.randomHex", format!("the count must be from 1 to {} bytes", MAX_RANDOM)));
    }
    let mut buf = vec![0u8; n as usize];
    random_bytes("crypto.randomHex", &mut buf)?;
    Ok(LibValue::Str(hex::encode(buf)))
}

fn uuid(_: &CallCtx, _: Vec<LibValue>) -> Result<LibValue, LibError> {
    let mut b = [0u8; 16];
    random_bytes("crypto.uuid", &mut b)?;
    // RFC 4122: version 4 in the high nibble of byte 6, the variant in the top bits of byte 8
    b[6] = (b[6] & 0x0f) | 0x40;
    b[8] = (b[8] & 0x3f) | 0x80;
    let h = hex::encode(b);
    Ok(LibValue::Str(format!("{}-{}-{}-{}-{}", &h[0..8], &h[8..12], &h[12..16], &h[16..20], &h[20..32])))
}

/// the AES key for a password: Argon2id with the OWASP minimum (19 MiB, 2 passes, 1 lane)
fn derive_key(function: &'static str, password: &str, salt: &[u8]) -> Result<[u8; 32], LibError> {
    let params = Params::new(19_456, 2, 1, Some(32)).map_err(|e| LibError::new(function, e.to_string()))?;
    let mut key = [0u8; 32];
    Argon2::new(Algorithm::Argon2id, Version::V0x13, params)
        .hash_password_into(password.as_bytes(), salt, &mut key)
        .map_err(|e| LibError::new(function, e.to_string()))?;
    Ok(key)
}

fn encrypt(_: &CallCtx, a: Vec<LibValue>) -> Result<LibValue, LibError> {
    const F: &str = "crypto.encrypt";
    let mut salt = [0u8; SALT_LEN];
    let mut nonce = [0u8; NONCE_LEN];
    random_bytes(F, &mut salt)?;
    random_bytes(F, &mut nonce)?;
    let key = derive_key(F, text_at(&a, 1), &salt)?;
    let cipher = <Aes256Gcm as KeyInit>::new_from_slice(&key).map_err(|_| LibError::new(F, "the key could not be used"))?;
    let sealed = cipher
        .encrypt(Nonce::from_slice(&nonce), text_at(&a, 0).as_bytes())
        .map_err(|_| LibError::new(F, "the text could not be sealed"))?;
    let mut blob = Vec::with_capacity(1 + SALT_LEN + NONCE_LEN + sealed.len());
    blob.push(FORMAT);
    blob.extend_from_slice(&salt);
    blob.extend_from_slice(&nonce);
    blob.extend_from_slice(&sealed);
    Ok(LibValue::Str(base64::engine::general_purpose::STANDARD.encode(blob)))
}

fn decrypt(_: &CallCtx, a: Vec<LibValue>) -> Result<LibValue, LibError> {
    const F: &str = "crypto.decrypt";
    let blob = base64::engine::general_purpose::STANDARD
        .decode(text_at(&a, 0))
        .map_err(|_| LibError::new(F, "not an encrypted text (it is not valid base64)"))?;
    if blob.len() < MIN_LEN {
        return Err(LibError::new(F, "not an encrypted text (too short)"));
    }
    if blob[0] != FORMAT {
        return Err(LibError::new(F, format!("unknown format version {} (this build reads version {})", blob[0], FORMAT)));
    }
    let salt = &blob[1..1 + SALT_LEN];
    let nonce = &blob[1 + SALT_LEN..1 + SALT_LEN + NONCE_LEN];
    let sealed = &blob[1 + SALT_LEN + NONCE_LEN..];
    let key = derive_key(F, text_at(&a, 1), salt)?;
    let cipher = <Aes256Gcm as KeyInit>::new_from_slice(&key).map_err(|_| LibError::new(F, "the key could not be used"))?;
    let plain = cipher
        .decrypt(Nonce::from_slice(nonce), sealed)
        .map_err(|_| LibError::new(F, "wrong password, or the text was changed"))?;
    as_text(F, plain)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::LibFn;

    fn ctx() -> CallCtx<'static> {
        CallCtx { caller_file: None, caller_line: 0 }
    }

    fn call(f: LibFn, args: &[&str]) -> Result<LibValue, LibError> {
        f(&ctx(), args.iter().map(|s| LibValue::Str(s.to_string())).collect())
    }

    fn out(f: LibFn, args: &[&str]) -> String {
        match call(f, args) {
            Ok(LibValue::Str(t)) => t,
            other => panic!("unexpected {:?}", other),
        }
    }

    #[test]
    fn hashes_match_the_published_test_vectors() {
        assert_eq!(out(sha1, &["abc"]), "a9993e364706816aba3e25717850c26c9cd0d89d");
        assert_eq!(out(sha256, &["abc"]), "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
        assert_eq!(
            out(sha512, &["abc"]),
            "ddaf35a193617abacc417349ae20413112e6fa4e89a97ea20a9eeee64b55d39a2192992a274fc1a836ba3c23a3feebbd454d4423643ce80e2a9ac94fa54ca49f"
        );
        assert_eq!(out(md5, &["abc"]), "900150983cd24fb0d6963f7d28e17f72");
    }

    #[test]
    fn hmac_matches_a_published_vector() {
        assert_eq!(
            out(hmac_sha256, &["key", "The quick brown fox jumps over the lazy dog"]),
            "f7bc83f430538424b13298e6aa6fb143ef4d59a14946175997479dbc2d1a3cd8"
        );
        assert_eq!(out(hmac_sha512, &["key", "x"]).len(), 128);
    }

    #[test]
    fn encodings_round_trip_and_reject_garbage() {
        assert_eq!(out(base64_encode, &["hello"]), "aGVsbG8=");
        assert_eq!(out(base64_decode, &["aGVsbG8="]), "hello");
        assert_eq!(out(base64url_encode, &["??>"]), "Pz8-");
        assert_eq!(out(base64url_decode, &["Pz8-"]), "??>");
        assert_eq!(out(hex_encode, &["hi"]), "6869");
        assert_eq!(out(hex_decode, &["6869"]), "hi");
        assert!(call(base64_decode, &["!!!"]).is_err());
        assert!(call(hex_decode, &["zz"]).is_err());
        // "/w==" is one byte 0xFF: valid base64, not valid UTF-8 text
        assert!(call(base64_decode, &["/w=="]).is_err());
    }

    #[test]
    fn random_values_have_the_asked_shape() {
        assert_eq!(out_int(random_hex, 8).len(), 16);
        assert!(call_int(random_hex, 0).is_err());
        assert!(call_int(random_hex, MAX_RANDOM + 1).is_err());
        let id = out(uuid_fn, &[]);
        assert_eq!(id.len(), 36);
        assert_eq!(id.as_bytes()[14], b'4', "version nibble: {}", id);
        assert_ne!(out(uuid_fn, &[]), id, "two uuids should differ");
    }

    fn uuid_fn(c: &CallCtx, a: Vec<LibValue>) -> Result<LibValue, LibError> {
        uuid(c, a)
    }

    fn call_int(f: LibFn, n: i64) -> Result<LibValue, LibError> {
        f(&ctx(), vec![LibValue::Int(n)])
    }

    fn out_int(f: LibFn, n: i64) -> String {
        match call_int(f, n) {
            Ok(LibValue::Str(t)) => t,
            other => panic!("unexpected {:?}", other),
        }
    }

    #[test]
    fn encrypt_then_decrypt_gives_the_text_back() {
        let sealed = out(encrypt, &["segredo de café", "senha forte"]);
        assert!(!sealed.contains("segredo"), "the text must not show in the output");
        assert_eq!(out(decrypt, &[&sealed, "senha forte"]), "segredo de café");
    }

    #[test]
    fn every_encryption_uses_a_new_salt_and_nonce() {
        let a = out(encrypt, &["mesmo texto", "senha"]);
        let b = out(encrypt, &["mesmo texto", "senha"]);
        assert_ne!(a, b);
    }

    #[test]
    fn a_wrong_password_or_changed_data_is_an_error() {
        let sealed = out(encrypt, &["dado", "certa"]);
        let err = call(decrypt, &[&sealed, "errada"]).unwrap_err();
        assert!(err.reason.contains("wrong password"), "{}", err.reason);
        // change one character inside the sealed part: the tag must catch it
        let mut bytes = base64::engine::general_purpose::STANDARD.decode(&sealed).unwrap();
        let last = bytes.len() - 1;
        bytes[last] ^= 0x01;
        let tampered = base64::engine::general_purpose::STANDARD.encode(bytes);
        assert!(call(decrypt, &[&tampered, "certa"]).is_err());
    }

    #[test]
    fn decrypt_says_what_is_wrong_with_the_input() {
        assert!(call(decrypt, &["não é base64!", "x"]).unwrap_err().reason.contains("not an encrypted text"));
        assert!(call(decrypt, &["aGVsbG8=", "x"]).unwrap_err().reason.contains("too short"));
    }
}
