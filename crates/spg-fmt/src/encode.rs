//! `--enc`: the names a program declares (functions, variables, parameters, loop and catch
//! variables) become random-looking ones, and the text is written in the compact layout
//! without comments. what the outside can see stays as it is: struct and enum names, fields
//! and variants (printed and serialized), methods, anything after a `.` (library members),
//! the names of native libraries, and what the program takes from the standard library.
//! the output is checked against the input: the same tokens with the new names, and the
//! same parsed program once the new names are swapped back.

use super::{format_compact_pieces, program_shape, scan, token_kinds, Atom, Class, FmtError, Piece, KEYWORDS};
use spg_lexer::TokenKind;
use std::collections::{HashMap, HashSet};

/// what the innermost open bracket is, for the atom that comes after it
#[derive(Clone, Copy, PartialEq, Debug)]
enum Open {
    /// statements: names here are variables or functions
    Block,
    /// a struct or enum declaration (`let { x, y }` too): names here are fields, never renamed
    Body,
    /// a struct literal `P { x: 1 }`: keys are fields, values are expressions
    Lit,
    Bracket,
    /// a parenthesis that declares names: parameters (`fn`), loop variables (`for`),
    /// the catch name, or a tuple pattern (`let (a, b)`)
    Paren { declares: bool },
}

/// the innermost open bracket before each atom
fn innermost(seq: &[&Atom]) -> Vec<Open> {
    let mut stack: Vec<Open> = Vec::new();
    let mut out = Vec::with_capacity(seq.len());
    let mut pending_decl = false;
    for (i, a) in seq.iter().enumerate() {
        out.push(stack.last().copied().unwrap_or(Open::Block));
        let prev = i.checked_sub(1).map(|p| seq[p]);
        let prev2 = i.checked_sub(2).map(|p| seq[p]);
        match a.class {
            Class::Keyword if a.text == "struct" || a.text == "enum" => pending_decl = true,
            Class::Punct => match a.text.as_str() {
                "(" => {
                    let declares = match prev {
                        Some(p) if matches!(p.text.as_str(), "fn" | "for" | "catch" | "let") => true,
                        Some(p) => p.class == Class::Ident && prev2.is_some_and(|q| q.text == "fn"),
                        None => false,
                    };
                    stack.push(Open::Paren { declares });
                }
                "[" => stack.push(Open::Bracket),
                "{" => {
                    let kind = if pending_decl {
                        pending_decl = false;
                        Open::Body
                    } else if prev.is_some_and(|p| p.text == "let") {
                        Open::Body
                    } else if prev.is_some_and(|p| p.class == Class::Ident)
                        && !prev2.is_some_and(|q| matches!(q.text.as_str(), "->" | "namespace" | "fn"))
                    {
                        // `P { x: 1 }` builds a struct; `-> Point {` and `namespace som {` open a body of statements
                        Open::Lit
                    } else {
                        Open::Block
                    };
                    stack.push(kind);
                }
                ")" | "]" | "}" => {
                    stack.pop();
                }
                _ => {}
            },
            _ => {}
        }
    }
    out
}

fn atoms(pieces: &[Piece]) -> Vec<&Atom> {
    pieces
        .iter()
        .filter_map(|p| match p {
            Piece::Atom(a) => Some(a),
            _ => None,
        })
        .collect()
}

/// names this file declares, and names that must never change
fn collect(pieces: &[Piece], declared: &mut HashSet<String>, keep: &mut HashSet<String>) {
    let seq = atoms(pieces);
    let inner = innermost(&seq);
    for (i, a) in seq.iter().enumerate() {
        if a.class != Class::Ident {
            continue;
        }
        let prev = i.checked_sub(1).map(|p| seq[p].text.as_str());
        // a member (`p.calls`, `ps.run`) keeps its name wherever it is: its owner may be a library
        if matches!(prev, Some("struct") | Some("enum") | Some(".")) {
            keep.insert(a.text.clone());
            continue;
        }
        let declares_here = match prev {
            Some("fn") => inner[i] != Open::Body,
            Some("let") | Some("const") => true,
            Some("(") | Some(",") => matches!(inner[i], Open::Paren { declares: true }),
            _ => false,
        };
        if declares_here {
            declared.insert(a.text.clone());
        }
    }
}

/// a name that is not an identifier of the program can't be a new one
fn random_name(state: &mut u64) -> String {
    const FIRST: &[u8] = b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ";
    const REST: &[u8] = b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789";
    let mut next = || {
        // xorshift64
        *state ^= *state << 13;
        *state ^= *state >> 7;
        *state ^= *state << 17;
        *state
    };
    let len = 6 + (next() % 3) as usize;
    let mut name = String::new();
    name.push(FIRST[(next() % FIRST.len() as u64) as usize] as char);
    for _ in 1..len {
        name.push(REST[(next() % REST.len() as u64) as usize] as char);
    }
    name
}

/// the same seed for the same sources: a build is reproducible
fn seed_of(files: &[String]) -> u64 {
    let mut hash: u64 = 0xcbf29ce484222325;
    for file in files {
        for byte in file.bytes().chain(std::iter::once(0u8)) {
            hash ^= byte as u64;
            hash = hash.wrapping_mul(0x100000001b3);
        }
    }
    hash | 1
}

/// replaces the names inside the `${...}` parts of a string, the same way as in code
fn rename_in_interpolation(text: &str, map: &HashMap<String, String>) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.find("${") {
        // `\${` is the text "${", not an interpolation
        if rest[..at].ends_with('\\') {
            out.push_str(&rest[..at + 2]);
            rest = &rest[at + 2..];
            continue;
        }
        out.push_str(&rest[..at + 2]);
        rest = &rest[at + 2..];
        let end = rest.find('}').unwrap_or(rest.len());
        out.push_str(&rename_words(&rest[..end], map));
        rest = &rest[end..];
    }
    out.push_str(rest);
    out
}

fn rename_words(code: &str, map: &HashMap<String, String>) -> String {
    let mut out = String::new();
    let chars: Vec<char> = code.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c.is_alphabetic() || c == '_' {
            let start = i;
            while i < chars.len() && (chars[i].is_alphanumeric() || chars[i] == '_') {
                i += 1;
            }
            let word: String = chars[start..i].iter().collect();
            // the word after a `.` is a member, not a name of the program
            let after_dot = chars[..start].iter().rev().find(|c| !c.is_whitespace()) == Some(&'.');
            match map.get(&word) {
                Some(new) if !after_dot => out.push_str(new),
                _ => out.push_str(&word),
            }
        } else {
            out.push(c);
            i += 1;
        }
    }
    out
}

/// the names that come after a `.` inside the `${...}` parts of a string
fn members_in_interpolation(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = text;
    while let Some(at) = rest.find('.') {
        rest = &rest[at + 1..];
        let name: String = rest.chars().take_while(|c| c.is_alphanumeric() || *c == '_').collect();
        if !name.is_empty() {
            out.push(name);
        }
    }
    out
}

/// the file's pieces with its names renamed
fn rename(pieces: &mut [Piece], map: &HashMap<String, String>) {
    let kinds: Vec<Atom> = atoms(pieces).into_iter().cloned().collect();
    let seq: Vec<&Atom> = kinds.iter().collect();
    let inner = innermost(&seq);
    // the index of each atom among the pieces, in order
    let slots: Vec<usize> = pieces
        .iter()
        .enumerate()
        .filter_map(|(i, p)| matches!(p, Piece::Atom(_)).then_some(i))
        .collect();
    for (i, a) in seq.iter().enumerate() {
        let slot = slots[i];
        let prev = i.checked_sub(1).map(|p| seq[p].text.as_str());
        let next = seq.get(i + 1).map(|n| n.text.as_str());
        if a.class == Class::Str && a.text.contains("${") && inner[i] != Open::Body {
            let new = rename_in_interpolation(&a.text, map);
            if let Piece::Atom(atom) = &mut pieces[slot] {
                atom.text = new;
            }
            continue;
        }
        if a.class != Class::Ident || prev == Some(".") {
            continue;
        }
        let Some(new) = map.get(&a.text) else { continue };
        // fields and variants: never renamed. a struct literal key `x: 1` is a field too
        if inner[i] == Open::Body || (inner[i] == Open::Lit && next == Some(":")) {
            continue;
        }
        if let Piece::Atom(atom) = &mut pieces[slot] {
            atom.text = new.clone();
        }
    }
}

/// the program's debug text with every name and every string literal replaced by a
/// placeholder. the check then compares the shape of the tree: which nodes, in which order,
/// with which literals of which kinds. the names are changed on purpose, so they can't be
/// compared; the string literals may hold renamed interpolations, so they are not either
fn hide_names(shape: &str, names: &HashSet<String>) -> String {
    let mut out = String::with_capacity(shape.len());
    let parts: Vec<&str> = shape.split('"').collect();
    for (i, part) in parts.iter().enumerate() {
        if i > 0 {
            out.push('"');
        }
        if i % 2 == 1 {
            let literal = out.ends_with("StringLit(\"");
            if literal {
                out.push_str("<hidden>");
            } else {
                // a qualified name (`pair.twice`) hides the part that is a name of the program
                let hidden: Vec<&str> = part
                    .split('.')
                    .map(|seg| if names.contains(seg) { "<hidden>" } else { seg })
                    .collect();
                out.push_str(&hidden.join("."));
            }
        } else {
            out.push_str(part);
        }
    }
    out
}

/// token by token: the same kinds in the same order. names and strings are not compared
/// here: the tree check compares the names, and strings may hold renamed interpolations
fn same_tokens(before: &str, after: &str) -> bool {
    let (Ok(before), Ok(after)) = (token_kinds(before), token_kinds(after)) else { return false };
    let norm = |kinds: Vec<TokenKind>| -> Vec<TokenKind> {
        kinds
            .into_iter()
            .filter(|k| *k != TokenKind::Semicolon)
            .map(|k| match k {
                TokenKind::Ident(_) => TokenKind::Ident(String::new()),
                TokenKind::Str(_) => TokenKind::Str(String::new()),
                other => other,
            })
            .collect()
    };
    norm(before) == norm(after)
}

/// encodes the user's files (`files`, in the order they go in the build). `std_files` are the
/// standard library's, read only so the new names avoid theirs; `native` are the names of
/// the native libraries, which never change. returns the encoded files, in the same order
pub fn encode_sources(files: &[String], std_files: &[String], native: &[String]) -> Result<Vec<String>, FmtError> {
    let mut parsed: Vec<Vec<Piece>> = files.iter().map(|f| scan(f)).collect();

    let mut declared: HashSet<String> = HashSet::new();
    // `me` is the receiver of a method: the parser adds it, it is not a name the program chose.
    // the type words are names too (`let map = 3` must not turn `map<int, int>` into a new name)
    const FIXED: &[&str] = &["me", "map", "array", "task", "int", "float", "bool", "string", "void", "i8", "i16", "i32", "i64", "u8", "u16", "u32", "u64", "f32"];
    let mut keep: HashSet<String> = native.iter().cloned().chain(FIXED.iter().map(|s| s.to_string())).collect();
    for pieces in &parsed {
        collect(pieces, &mut declared, &mut keep);
        // a member used inside `${...}` (`${som.count()}`) is a member too
        for a in atoms(pieces) {
            if a.class == Class::Str && a.text.contains("${") {
                keep.extend(members_in_interpolation(&a.text));
            }
        }
    }
    let mut names: Vec<String> = declared.into_iter().filter(|n| !keep.contains(n)).collect();
    names.sort();

    // every word the sources use is taken, and so is every keyword
    let mut taken: HashSet<String> = KEYWORDS.iter().map(|k| k.to_string()).collect();
    for text in files.iter().chain(std_files.iter()) {
        for a in atoms(&scan(text)) {
            if matches!(a.class, Class::Ident | Class::Keyword | Class::Namespace) {
                taken.insert(a.text.clone());
            }
        }
    }
    taken.extend(native.iter().cloned());

    let mut state = seed_of(files);
    let mut map: HashMap<String, String> = HashMap::new();
    for name in names {
        let new = loop {
            let candidate = random_name(&mut state);
            if !taken.contains(&candidate) {
                break candidate;
            }
        };
        taken.insert(new.clone());
        map.insert(name, new);
    }

    let mut out = Vec::with_capacity(files.len());
    for (file, pieces) in files.iter().zip(parsed.iter_mut()) {
        rename(pieces, &map);
        let encoded = format_compact_pieces(std::mem::take(pieces), false);
        if !same_tokens(file, &encoded) {
            return Err(FmtError::EncodeChanged);
        }
        if let Some(shape) = program_shape(file) {
            let hidden: HashSet<String> = map.iter().flat_map(|(old, new)| [old.clone(), new.clone()]).collect();
            let before_shape = hide_names(&shape, &hidden);
            let after_shape = program_shape(&encoded).map(|s| hide_names(&s, &hidden));
            if after_shape.as_deref() != Some(before_shape.as_str()) {
                return Err(FmtError::EncodeChanged);
            }
        }
        out.push(encoded);
    }
    Ok(out)
}
