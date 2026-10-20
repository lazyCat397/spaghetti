pub mod lexer;
pub mod token;

pub use lexer::{apply_installed_namespaces, read_file_spec, FileSpec, LexError, Lexer, DIRECTIVES};
pub use token::{Token, TokenKind};

/// builtin namespaces. in source they always carry a leading `@`
/// (`@math.sqrt`, `@str.len`, ...). the names below are WITHOUT the `@`.
/// because `@` can never be part of an identifier, a namespace can never
/// collide with anything the user declares. `struct @math {}` is a lex-level
/// namespace token in a name position, reported as a reserved-name error. don't.
/// the CORE namespaces: always there, always written with `@` (`@math.sqrt`).
pub const NAMESPACES: [&str; 7] = ["str", "array", "map", "into", "math", "sys", "task"];

/// every namespace that takes an `@`: the ones built into the engine (`NAMESPACES`) and the
/// core libraries of `spg-libs` (`@file`, ...). in the order the editor and the errors list them.
pub fn namespaces() -> Vec<&'static str> {
    let mut all: Vec<&'static str> = NAMESPACES.to_vec();
    all.extend(spg_libs::modules().iter().filter(|m| m.kind == spg_libs::Kind::Core).map(|m| m.name));
    all
}

/// native (Rust) libraries need `install #name` first (the registry is in `spg-libs`).
/// once installed, their namespace is written WITHOUT `@` (`ps.run(...)`): `@` means "the engine
/// gives you this for free", no `@` means "you installed it". the lexer turns `ps.` into a
/// namespace token only in a file that installs `ps` (see `apply_installed_namespaces`).
pub fn is_namespace(name: &str) -> bool {
    NAMESPACES.contains(&name) || spg_libs::module(name).is_some_and(|m| m.kind == spg_libs::Kind::Core)
}

pub fn is_install_namespace(name: &str) -> bool {
    spg_libs::module(name).is_some_and(|m| m.kind == spg_libs::Kind::Install)
}

/// hints for the friendly tone (`#> friendly-terminal`), one per stage
pub const HINT_SYNTAX: &str = "Check the syntax around that spot: a missing or extra symbol, or a keyword in the wrong place, is the usual cause.";
pub const HINT_TYPES: &str = "Check the types on both sides of that spot. They must agree, or convert the value explicitly with @into.";
pub const HINT_COMPILE: &str = "This one is on the compiler's side, not yours. Simplify the code around that spot, and report it if it keeps happening.";
pub const HINT_RUNTIME: &str = "The program stopped while running. Check the values involved at that point; a try/catch can handle this kind of error.";
pub const HINT_MODULE: &str = "Check the file paths and the names you import or use.";

/// the friendly tone shared by every engine error: "Oops!", the same fact
/// (first letter lowercased, trailing period dropped), then a hint
pub fn friendly_sentence(fact: &str, hint: &str) -> String {
    let fact = fact.trim_end_matches('.');
    let mut chars = fact.chars();
    let lowered: String = match chars.next() {
        Some(c) => c.to_lowercase().chain(chars).collect(),
        None => String::new(),
    };
    format!("Oops! {}. {}", lowered, hint)
}

#[cfg(test)]
mod friendly_tests {
    use super::friendly_sentence;

    #[test]
    fn wraps_fact_with_oops_and_hint() {
        assert_eq!(
            friendly_sentence("Expected int, found string.", "Check it."),
            "Oops! expected int, found string. Check it."
        );
    }
}

pub const HINT_SHADOW: &str = "Rename the inner one, or reuse the outer one. Strict mode (#> strict) doesn't let a name hide another.";
pub const HINT_UNREACHABLE: &str = "Nothing after that line can ever run. Remove the extra code, or move it before that line.";
pub const HINT_OVERFLOW: &str = "The number doesn't fit in an int (64 bits) anymore. Use smaller values, or a float. Strict mode (#> strict) stops here instead of wrapping around.";
pub const HINT_TIMEOUT: &str = "Check for an endless loop, or a wait (sleep, readLine) that takes too long. If the program really needs more time, raise the number in `#> timeout`.";
