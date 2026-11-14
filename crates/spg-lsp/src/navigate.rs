//! hover and go-to-definition, read from the document's tokens (like the
//! completion and signature help. no full resolve, so it works while the
//! buffer is half-typed).
//!
//! `declarations` lists everything a name can refer to in one file: functions,
//! methods, structs and their fields, enums, `let`/`const`, and params.
//! `target` picks the one a name at the cursor means: the nearest declaration
//! before it, else a file-level one, else the file it's imported from.

use crate::signature::{self, read_type, Signature};
use spg_lexer::{Token, TokenKind as T};

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Kind {
    Function,
    Method,
    Struct,
    Enum,
    Let,
    Param,
    Field,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Decl {
    pub name: String,
    pub kind: Kind,
    /// index of the name's token: the declaration's position
    pub index: usize,
    /// 1-based line and column of the name, as the lexer reports them
    pub line: usize,
    pub col: usize,
    /// type text (`int`, `Point`, `fn(int) -> int`), or the signature label
    /// for a callable. empty when nothing is known
    pub ty: String,
    /// the struct a method or field belongs to
    pub owner: Option<String>,
    /// for callables: the params and return type, for signature help
    pub sig: Option<Signature>,
}

fn kind_at(tokens: &[Token], i: usize) -> Option<&T> {
    tokens.get(i).map(|t| &t.kind)
}

fn decl(name: &str, kind: Kind, tokens: &[Token], index: usize, ty: String, owner: Option<String>, sig: Option<Signature>) -> Decl {
    Decl {
        name: name.to_string(),
        kind,
        index,
        line: tokens[index].line,
        col: tokens[index].col,
        ty,
        owner,
        sig,
    }
}

/// the type a `let` gets from its value when there's no annotation: literals,
/// struct literals (`Point { .. }`) and lambdas / function values
fn inferred(tokens: &[Token], i: usize) -> String {
    match kind_at(tokens, i) {
        Some(T::Int(_)) => "int".into(),
        Some(T::Float(_)) => "float".into(),
        Some(T::Str(_)) => "string".into(),
        Some(T::Bool(_)) => "bool".into(),
        // a lambda: its param TYPES, which read_type doesn't take from `x: T` pairs
        Some(T::Function) if kind_at(tokens, i + 1) == Some(&T::LParen) => {
            let Some((params, close)) = signature::read_params(tokens, i + 1) else {
                return String::new();
            };
            let types: Vec<String> = params.into_iter().map(|(_, t)| t).collect();
            format!("fn({}) -> {}", types.join(", "), signature::return_after(tokens, close))
        }
        Some(T::Function) => read_type(tokens, i).0,
        Some(T::Ident(name)) if kind_at(tokens, i + 1) == Some(&T::LBrace) => name.clone(),
        _ => String::new(),
    }
}

/// every declaration in the file, in source order
pub fn declarations(tokens: &[Token]) -> Vec<Decl> {
    let mut out = Vec::new();
    // brace depth, and the struct bodies we're inside (name, depth of their `{`)
    let mut depth = 0usize;
    let mut structs: Vec<(String, usize)> = Vec::new();

    for i in 0..tokens.len() {
        match &tokens[i].kind {
            T::LBrace => {
                depth += 1;
                if let (Some(T::Ident(name)), Some(T::Struct)) = (
                    i.checked_sub(1).and_then(|j| kind_at(tokens, j)),
                    i.checked_sub(2).and_then(|j| kind_at(tokens, j)),
                ) {
                    structs.push((name.clone(), depth));
                }
                continue;
            }
            T::RBrace => {
                if structs.last().map(|(_, d)| *d) == Some(depth) {
                    structs.pop();
                }
                depth = depth.saturating_sub(1);
                continue;
            }
            _ => {}
        }
        let owner = structs.last().filter(|(_, d)| *d == depth).map(|(n, _)| n.clone());

        match &tokens[i].kind {
            T::Struct => {
                if let Some(T::Ident(name)) = kind_at(tokens, i + 1) {
                    out.push(decl(name, Kind::Struct, tokens, i + 1, String::new(), None, None));
                }
            }
            T::Enum => {
                if let Some(T::Ident(name)) = kind_at(tokens, i + 1) {
                    out.push(decl(name, Kind::Enum, tokens, i + 1, String::new(), None, None));
                }
            }
            T::Function => {
                let named = matches!(kind_at(tokens, i + 1), Some(T::Ident(_))) && kind_at(tokens, i + 2) == Some(&T::LParen);
                let lambda = kind_at(tokens, i + 1) == Some(&T::LParen);
                if named || lambda {
                    let name_index = if named { i + 1 } else { i };
                    let open = if named { i + 2 } else { i + 1 };
                    let (params, close) = signature::read_params(tokens, open).unwrap_or_default();
                    let ret = signature::return_after(tokens, close);
                    if named {
                        let name = match &tokens[name_index].kind {
                            T::Ident(n) => n.clone(),
                            _ => unreachable!("checked above"),
                        };
                        let sig = Signature { name: name.clone(), params: params.clone(), ret: ret.clone() };
                        let kind = if owner.is_some() { Kind::Method } else { Kind::Function };
                        out.push(decl(&name, kind, tokens, name_index, sig.label(), owner.clone(), Some(sig)));
                    }
                    // the params, each with its own token position
                    let mut j = open + 1;
                    while j < close {
                        if let (Some(T::Ident(p)), Some(T::Colon)) = (kind_at(tokens, j), kind_at(tokens, j + 1)) {
                            let (ty, next) = read_type(tokens, j + 2);
                            out.push(decl(p, Kind::Param, tokens, j, ty, None, None));
                            j = next.max(j + 1);
                        } else {
                            j += 1;
                        }
                    }
                }
            }
            T::Let | T::Const => {
                if let Some(T::Ident(name)) = kind_at(tokens, i + 1) {
                    let ty = match kind_at(tokens, i + 2) {
                        Some(T::Colon) => read_type(tokens, i + 3).0,
                        Some(T::Eq) => inferred(tokens, i + 3),
                        _ => String::new(),
                    };
                    out.push(decl(name, Kind::Let, tokens, i + 1, ty, None, None));
                }
            }
            // a field of a struct: `x: int` directly inside the struct's braces
            T::Ident(name) if owner.is_some() && kind_at(tokens, i + 1) == Some(&T::Colon) => {
                let (ty, _) = read_type(tokens, i + 2);
                out.push(decl(name, Kind::Field, tokens, i, ty, owner.clone(), None));
            }
            _ => {}
        }
    }
    out
}

/// the identifier under the cursor (0-based line and character): its token index
pub fn word_at(tokens: &[Token], line: usize, character: usize) -> Option<usize> {
    let col = character + 1;
    tokens.iter().position(|t| match &t.kind {
        T::Ident(name) => t.line == line + 1 && t.col <= col && col <= t.col + name.chars().count(),
        _ => false,
    })
}

/// what a name at the cursor refers to
#[derive(Debug, Clone, PartialEq)]
pub enum Target {
    /// declared in this file
    Local(Decl),
    /// imported by name from another file (`use (name) from "file"`)
    Imported { name: String, file: String },
}

/// the struct a receiver names: `me` is the struct of the method we're in, a
/// variable or param is its declared type
fn receiver_struct(tokens: &[Token], decls: &[Decl], receiver: &str, before: usize) -> Option<String> {
    if receiver == "me" {
        return decls
            .iter()
            .filter(|d| d.kind == Kind::Method && d.index <= before)
            .last()
            .and_then(|d| d.owner.clone());
    }
    let _ = tokens;
    let d = decls
        .iter()
        .filter(|d| d.name == receiver && d.index <= before && matches!(d.kind, Kind::Let | Kind::Param))
        .last()?;
    if decls.iter().any(|s| s.kind == Kind::Struct && s.name == d.ty) {
        Some(d.ty.clone())
    } else {
        None
    }
}

/// what the identifier at `at` refers to. after a dot it's a field or method
/// of the receiver's struct. otherwise the nearest variable or param before
/// it, then a file-level function, struct or enum, then an import
pub fn target(tokens: &[Token], decls: &[Decl], at: usize) -> Option<Target> {
    let name = match kind_at(tokens, at)? {
        T::Ident(n) => n.clone(),
        _ => return None,
    };
    if at >= 2 && kind_at(tokens, at - 1) == Some(&T::Dot) {
        if let Some(T::Ident(receiver)) = kind_at(tokens, at - 2) {
            let owner = receiver_struct(tokens, decls, receiver, at)?;
            let found = decls.iter().find(|d| d.name == name && d.owner.as_deref() == Some(owner.as_str()))?;
            return Some(Target::Local(found.clone()));
        }
        return None;
    }

    if let Some(local) = decls
        .iter()
        .filter(|d| d.name == name && d.index <= at && matches!(d.kind, Kind::Let | Kind::Param))
        .last()
    {
        return Some(Target::Local(local.clone()));
    }
    if let Some(global) = decls
        .iter()
        .find(|d| d.name == name && d.owner.is_none() && matches!(d.kind, Kind::Function | Kind::Struct | Kind::Enum))
    {
        return Some(Target::Local(global.clone()));
    }
    // a name declared later in the file (a `let` or param after the cursor)
    if let Some(later) = decls.iter().find(|d| d.name == name && matches!(d.kind, Kind::Let | Kind::Param)) {
        return Some(Target::Local(later.clone()));
    }
    import_of(tokens, &name).map(|file| Target::Imported { name, file })
}

/// the file a name is imported from, if the document imports it
pub fn import_of(tokens: &[Token], name: &str) -> Option<String> {
    for i in 0..tokens.len() {
        if tokens[i].kind != T::Use {
            continue;
        }
        // the names between `{` and `}`, then `from "file"`
        let mut names = Vec::new();
        let mut j = i + 1;
        while j < tokens.len() && tokens[j].kind != T::From {
            if let T::Ident(n) = &tokens[j].kind {
                names.push(n.clone());
            }
            j += 1;
        }
        if let Some(T::Str(file)) = kind_at(tokens, j + 1) {
            if names.iter().any(|n| n == name) {
                return Some(file.clone());
            }
        }
    }
    None
}

/// hover text for a declaration, in the same words the editor shows elsewhere
pub fn describe(d: &Decl) -> String {
    match d.kind {
        Kind::Function | Kind::Method => d.ty.clone(),
        Kind::Struct => format!("struct {}", d.name),
        Kind::Enum => format!("enum {}", d.name),
        Kind::Let if d.ty.is_empty() => format!("let {}", d.name),
        Kind::Let => format!("let {}: {}", d.name, d.ty),
        Kind::Param => format!("parameter {}: {}", d.name, d.ty),
        Kind::Field => match &d.owner {
            Some(owner) => format!("field {}.{}: {}", owner, d.name, d.ty),
            None => format!("field {}: {}", d.name, d.ty),
        },
    }
}

/// for a method call `recv.f(`: the method's signature and the active arg.
/// `at` is the number of tokens before the cursor, as in `signature::call_site`
pub fn method_signature(tokens: &[Token], decls: &[Decl], at: usize) -> Option<(Signature, usize)> {
    let (callee, active) = signature::call_site(tokens, at)?;
    if callee < 2 || kind_at(tokens, callee - 1) != Some(&T::Dot) {
        return None;
    }
    let receiver = match kind_at(tokens, callee - 2)? {
        T::Ident(r) => r.clone(),
        _ => return None,
    };
    let method = match kind_at(tokens, callee)? {
        T::Ident(m) => m.clone(),
        _ => return None,
    };
    let owner = receiver_struct(tokens, decls, &receiver, callee)?;
    let found = decls
        .iter()
        .find(|d| d.kind == Kind::Method && d.name == method && d.owner.as_deref() == Some(owner.as_str()))?;
    Some((found.sig.clone()?, active))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn toks(src: &str) -> Vec<Token> {
        spg_lexer::Lexer::new(src).tokenize().unwrap()
    }

    #[test]
    fn struct_fields_and_methods_have_owners() {
        let t = toks("struct Point {\n  x: int,\n  y: int,\n  fn sum(k: int) -> int { return me.x + k }\n}");
        let d = declarations(&t);
        assert!(d.iter().any(|x| x.name == "x" && x.kind == Kind::Field && x.owner.as_deref() == Some("Point")));
        let m = d.iter().find(|x| x.name == "sum").unwrap();
        assert_eq!(m.kind, Kind::Method);
        assert_eq!(m.ty, "sum(k: int) -> int");
    }

    #[test]
    fn let_type_comes_from_annotation_or_value() {
        let t = toks("let a: int = 1\nlet f = fn(x: int) -> int { return x }\nlet s = \"hi\"");
        let d = declarations(&t);
        assert_eq!(d.iter().find(|x| x.name == "a").unwrap().ty, "int");
        assert_eq!(d.iter().find(|x| x.name == "f").unwrap().ty, "fn(int) -> int");
        assert_eq!(d.iter().find(|x| x.name == "s").unwrap().ty, "string");
    }

    #[test]
    fn nearest_local_wins_over_a_global_of_the_same_name() {
        let t = toks("let n = 1\nfn f(n: string) -> void { print(n) }");
        let d = declarations(&t);
        let at = t.iter().rposition(|x| x.kind == T::Ident("n".into())).unwrap();
        match target(&t, &d, at) {
            Some(Target::Local(found)) => assert_eq!(found.kind, Kind::Param),
            other => panic!("expected the parameter, got {:?}", other),
        }
    }

    #[test]
    fn imported_name_is_found_in_its_file() {
        let t = toks("use (dobro) from \"util.spg\"\nprint(dobro(2))");
        let d = declarations(&t);
        let at = t.iter().rposition(|x| x.kind == T::Ident("dobro".into())).unwrap();
        assert_eq!(target(&t, &d, at), Some(Target::Imported { name: "dobro".into(), file: "util.spg".into() }));
    }

    #[test]
    fn method_call_resolves_through_the_receiver_type() {
        let src = "struct Point {\n  x: int,\n  fn sum(k: int) -> int { return me.x + k }\n}\nlet p = Point { x: 1 }\nprint(p.sum(3))";
        let t = toks(src);
        let d = declarations(&t);
        let call_at = t.iter().position(|x| x.kind == T::Int(3)).unwrap();
        let (sig, active) = method_signature(&t, &d, call_at).unwrap();
        assert_eq!(sig.label(), "sum(k: int) -> int");
        assert_eq!(active, 0);
    }
}
