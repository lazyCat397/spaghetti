//! member completion: what to offer after `p.` (the fields and methods of a
//! struct, or `value` for an enum value) and inside `Point { ` (the fields
//! not written yet).
//!
//! the buffer is almost always broken at this moment (`p.` followed by
//! nothing doesn't parse), so none of this uses the parser or the
//! typechecker. it works on the TOKENS of the file, like the enum-variant
//! completion does: a small table of the struct/enum/function declarations,
//! and a best-effort lookup of what a name was declared as. it's a
//! heuristic on purpose. when it can't tell, it offers nothing. honest.

use spg_lexer::{Token, TokenKind as T};
use std::collections::{HashMap, HashSet};

/// completion item kinds (LSP): field, method
pub const KIND_FIELD: i64 = 5;
pub const KIND_METHOD: i64 = 2;

#[derive(Default, Clone, Debug)]
pub struct StructInfo {
    /// (name, type as written)
    pub fields: Vec<(String, String)>,
    /// (name, signature as written: `(a: int) -> int`)
    pub methods: Vec<(String, String)>,
}

#[derive(Default, Debug)]
pub struct Decls {
    pub structs: HashMap<String, StructInfo>,
    pub enums: HashSet<String>,
    /// free function name -> return type as written
    pub functions: HashMap<String, String>,
    /// exported `const`/`let` with a written type (`export const terminal: Terminal`), taken
    /// from the installed/imported files so their members can be listed
    pub globals: HashMap<String, String>,
    /// `function Tipo.nome(...) -> R` declarations, as (type, name, signature). they travel
    /// with `merge`, so an imported type's associated functions are listed too
    pub associated: Vec<(String, String, String)>,
}

pub struct Member {
    pub name: String,
    pub detail: String,
    pub kind: i64,
    /// text to insert when it differs from `name` (struct literal fields: `name: `)
    pub insert: Option<String>,
    /// one line on what it does, for the built-in methods (see spg_typecheck::docs)
    pub doc: Option<&'static str>,
}

impl Decls {
    pub fn from_tokens(tokens: &[Token]) -> Decls {
        let mut d = Decls::default();
        for i in 0..tokens.len() {
            if let (Some(T::Function), Some(T::Ident(ty)), Some(T::Dot), Some(T::Ident(name)), Some(T::LParen)) =
                (kind(tokens, i), kind(tokens, i + 1), kind(tokens, i + 2), kind(tokens, i + 3), kind(tokens, i + 4))
            {
                if let Some((sig, _)) = signature_at(tokens, i + 4) {
                    d.associated.push((ty.clone(), name.clone(), sig));
                }
            }
        }
        // a file that calls `@ps.*` gets the PsResult struct too, as the engine gives it
        // both prelude structs go in ONE token stream, so a file using @ps and @http gets both
        let mut prelude_src = String::new();
        if tokens.iter().any(|t| matches!(&t.kind, T::Namespace(n) if n == "ps")) {
            prelude_src.push_str(spg_lexer::PS_RESULT_SOURCE);
        }
        if tokens.iter().any(|t| matches!(&t.kind, T::Namespace(n) if n == "http")) {
            prelude_src.push_str(spg_lexer::HTTP_RESPONSE_SOURCE);
        }
        if !prelude_src.is_empty() {
            if let Ok(prelude) = spg_lexer::Lexer::new(&prelude_src).tokenize() {
                d = Decls::from_tokens(&prelude);
            }
        }
        let depth = depths(tokens);
        for i in 0..tokens.len() {
            match &tokens[i].kind {
                T::Enum => {
                    if let Some(T::Ident(n)) = kind(tokens, i + 1) {
                        d.enums.insert(n.clone());
                    }
                }
                T::Struct => {
                    if let (Some(T::Ident(n)), Some(T::LBrace)) = (kind(tokens, i + 1), kind(tokens, i + 2)) {
                        d.structs.insert(n.clone(), parse_struct_body(tokens, i + 3));
                    }
                }
                T::Export => {
                    if matches!(kind(tokens, i + 1), Some(T::Const) | Some(T::Let)) {
                        if let (Some(T::Ident(n)), Some(T::Colon)) = (kind(tokens, i + 2), kind(tokens, i + 3)) {
                            let ty = read_type(tokens, i + 4).0;
                            if !ty.is_empty() {
                                d.globals.insert(n.clone(), ty);
                            }
                        }
                    }
                }
                T::Function if depth[i] == 0 => {
                    if let (Some(T::Ident(n)), Some(T::LParen)) = (kind(tokens, i + 1), kind(tokens, i + 2)) {
                        let close = matching_paren(tokens, i + 2);
                        if let Some(c) = close {
                            if kind(tokens, c + 1) == Some(&T::Arrow) {
                                let (ret, _) = read_type(tokens, c + 2);
                                d.functions.insert(n.clone(), ret);
                            }
                        }
                    }
                }
                _ => {}
            }
        }
        d
    }

    /// adds what another file declares (names already known win)
    pub fn merge(&mut self, other: Decls) {
        for (k, v) in other.structs {
            self.structs.entry(k).or_insert(v);
        }
        self.enums.extend(other.enums);
        self.associated.extend(other.associated);
        for (k, v) in other.functions {
            self.functions.entry(k).or_insert(v);
        }
        for (k, v) in other.globals {
            self.globals.entry(k).or_insert(v);
        }
    }
}

fn kind(tokens: &[Token], i: usize) -> Option<&T> {
    tokens.get(i).map(|t| &t.kind)
}

/// how many `{` are still open before each token
fn depths(tokens: &[Token]) -> Vec<i32> {
    let mut out = Vec::with_capacity(tokens.len());
    let mut d = 0;
    for t in tokens {
        out.push(d);
        match t.kind {
            T::LBrace => d += 1,
            T::RBrace => d -= 1,
            _ => {}
        }
    }
    out
}

/// a type token's text, or "" for tokens that aren't part of a type
fn tok_text(k: &T) -> &'static str {
    match k {
        T::TypeInt => "int",
        T::TypeFloat => "float",
        T::TypeBool => "bool",
        T::TypeString => "string",
        T::TypeVoid => "void",
        T::TypeArray => "array",
        T::Lt => "<",
        T::Gt => ">",
        T::Comma => ", ",
        T::Colon => ": ",
        _ => "",
    }
}

/// reads a type starting at `i` (`int`, `Point`, `array<map<string, int>>`).
/// returns its text and the index just after it
fn read_type(tokens: &[Token], mut i: usize) -> (String, usize) {
    let mut text = String::new();
    match kind(tokens, i) {
        Some(T::Ident(n)) => text.push_str(n),
        Some(k) if !tok_text(k).is_empty() => text.push_str(tok_text(k)),
        _ => return (text, i),
    }
    i += 1;
    if kind(tokens, i) == Some(&T::Lt) {
        let mut nest = 0;
        while i < tokens.len() {
            match &tokens[i].kind {
                T::Lt => nest += 1,
                T::Gt => nest -= 1,
                _ => {}
            }
            match &tokens[i].kind {
                T::Ident(n) => text.push_str(n),
                k => text.push_str(tok_text(k)),
            }
            i += 1;
            if nest == 0 {
                break;
            }
        }
    }
    (text, i)
}

/// index of the `)` that closes the `(` at `open`
fn matching_paren(tokens: &[Token], open: usize) -> Option<usize> {
    let mut nest = 0;
    for (i, t) in tokens.iter().enumerate().skip(open) {
        match t.kind {
            T::LParen => nest += 1,
            T::RParen => {
                nest -= 1;
                if nest == 0 {
                    return Some(i);
                }
            }
            _ => {}
        }
    }
    None
}

/// fields and methods of a struct body. `start` is the token after its `{`
fn parse_struct_body(tokens: &[Token], start: usize) -> StructInfo {
    let mut info = StructInfo::default();
    let mut depth = 1;
    let mut j = start;
    while j < tokens.len() && depth > 0 {
        match &tokens[j].kind {
            T::LBrace => depth += 1,
            T::RBrace => depth -= 1,
            T::Ident(name) if depth == 1 && kind(tokens, j + 1) == Some(&T::Colon) => {
                let (ty, next) = read_type(tokens, j + 2);
                info.fields.push((name.clone(), ty));
                j = next;
                continue;
            }
            T::Function if depth == 1 => {
                if let (Some(T::Ident(name)), Some(T::LParen)) = (kind(tokens, j + 1), kind(tokens, j + 2)) {
                    if let Some(close) = matching_paren(tokens, j + 2) {
                        let mut params = String::new();
                        for t in &tokens[j + 3..close] {
                            match &t.kind {
                                T::Ident(n) => params.push_str(n),
                                k => params.push_str(tok_text(k)),
                            }
                        }
                        let mut sig = format!("({})", params);
                        let mut next = close + 1;
                        if kind(tokens, next) == Some(&T::Arrow) {
                            let (ret, after) = read_type(tokens, next + 1);
                            sig.push_str(&format!(" -> {}", ret));
                            next = after;
                        }
                        info.methods.push((name.clone(), sig));
                        j = next;
                        continue;
                    }
                }
            }
            _ => {}
        }
        j += 1;
    }
    info
}

/// the innermost `which` (`Function` or `Struct`) whose body contains token
/// index `at`
fn enclosing(tokens: &[Token], depth: &[i32], at: usize, which: &T) -> Option<usize> {
    for f in (0..at.min(tokens.len())).rev() {
        if tokens[f].kind != *which {
            continue;
        }
        let d0 = depth[f];
        let mut d = d0;
        let mut opened = false;
        let mut closed = false;
        for t in &tokens[f + 1..at.min(tokens.len())] {
            match t.kind {
                T::LBrace => {
                    d += 1;
                    opened = true;
                }
                T::RBrace => {
                    d -= 1;
                    if d <= d0 {
                        closed = true;
                        break;
                    }
                }
                _ => {}
            }
        }
        if opened && !closed && d > d0 {
            return Some(f);
        }
    }
    None
}

/// is the `[` at token `open` the start of a map literal (`[:]` or `["a": 1]`)?
/// it is when a `:` comes before the first `,` / closing `]` at its own nesting level.
fn bracket_is_map(tokens: &[Token], open: usize) -> bool {
    let mut nest = 0;
    for t in &tokens[open..] {
        match t.kind {
            T::LBracket | T::LParen | T::LBrace => nest += 1,
            T::RBracket | T::RParen | T::RBrace => {
                nest -= 1;
                if nest <= 0 {
                    return false;
                }
            }
            T::Colon if nest == 1 => return true,
            T::Comma if nest == 1 => return false,
            _ => {}
        }
    }
    false
}

/// `@str` / `@array` / `@map` for a value of this type text, if it's a native one
fn native_namespace(ty: &str) -> Option<&'static str> {
    if ty == "string" {
        Some("str")
    } else if ty == "array" || ty.starts_with("array<") {
        Some("array")
    } else if ty == "map" || ty.starts_with("map<") {
        Some("map")
    } else if ty.starts_with("task<") {
        Some("task")
    } else {
        None
    }
}

/// the methods of a string, array or map: the builtins of its namespace whose first
/// param takes the value (`xs.push(1)` is `@array.push(xs, 1)`), shown without
/// that first param
fn native_methods(ns: &str) -> Vec<Member> {
    use spg_typecheck::typed_ast::Ty;
    // a task has one method: `t.wait()` gives back the T. `@task.all` takes a list, not a task
    if ns == "task" {
        return vec![Member {
            name: "wait".to_string(),
            detail: "wait() -> T".to_string(),
            kind: KIND_METHOD,
            insert: None,
            doc: spg_typecheck::docs::builtin_doc("@task.wait"),
        }];
    }
    let prefix = format!("@{}.", ns);
    let mut out: Vec<Member> = Vec::new();
    for (name, overloads) in spg_typecheck::builtins::builtin_signatures() {
        let Some(short) = name.strip_prefix(&prefix) else { continue };
        let takes_receiver = |sig: &spg_typecheck::builtins::BuiltinSig| {
            matches!(
                (ns, sig.params.first()),
                ("str", Some(Ty::String)) | ("array", Some(Ty::AnyArray)) | ("array", Some(Ty::Array(_))) | ("map", Some(Ty::AnyMap))
            )
        };
        let Some(sig) = overloads.iter().find(|s| takes_receiver(s)) else { continue };
        let params: Vec<String> = sig.params.iter().skip(1).map(|p| p.to_string()).collect();
        out.push(Member {
            name: short.to_string(),
            detail: format!("{}({}) -> {}", short, params.join(", "), sig.return_type),
            kind: KIND_METHOD,
            insert: None,
            doc: spg_typecheck::docs::builtin_doc(&format!("@{}.{}", ns, short)),
        });
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

/// `let|const <head> [: T] [= expr]` at token `i`: `Some(type)` when this
/// declares `head` (the type may be unknown), `None` when it declares
/// something else
/// the result type of a built-in like `@ps.run`, as the user writes it. `None` when the
/// result depends on the arguments (`@array.pop` gives back the element type, say)
fn builtin_return(ns: &str, name: &str) -> Option<String> {
    use spg_typecheck::typed_ast::Ty;
    let full = format!("@{}.{}", ns, name);
    let (_, overloads) = spg_typecheck::builtins::builtin_signatures().into_iter().find(|(n, _)| *n == full)?;
    let ret = &overloads.first()?.return_type;
    match ret {
        Ty::Any | Ty::AnyArray | Ty::AnyMap | Ty::Void => None,
        other => Some(other.to_string()),
    }
}

fn decl_type(tokens: &[Token], i: usize, head: &str, decls: &Decls) -> Option<Option<String>> {
    if kind(tokens, i + 1) != Some(&T::Ident(head.to_string())) {
        return None;
    }
    match kind(tokens, i + 2) {
        Some(T::Colon) => {
            let ty = read_type(tokens, i + 3).0;
            Some(if ty.is_empty() { None } else { Some(ty) })
        }
        Some(T::Eq) => {
            // `let r = @ps.run(...)`: a built-in call gives its result type
            if let (Some(T::Namespace(ns)), Some(T::Dot), Some(T::Ident(f)), Some(T::LParen)) =
                (kind(tokens, i + 3), kind(tokens, i + 4), kind(tokens, i + 5), kind(tokens, i + 6))
            {
                return Some(builtin_return(ns, f));
            }
            let found = match (kind(tokens, i + 3), kind(tokens, i + 4)) {
                (Some(T::Ident(a)), Some(T::LBrace)) if decls.structs.contains_key(a) => Some(a.clone()),
                (Some(T::Ident(a)), Some(T::LParen)) => decls.functions.get(a).cloned(),
                (Some(T::Ident(a)), Some(T::Dot)) if decls.enums.contains(a) => Some(a.clone()),
                // literals: a string, an array `[1, 2]` or a map `["a": 1]` / `[:]`
                (Some(T::Str(_)), _) => Some("string".to_string()),
                (Some(T::LBracket), _) => Some(if bracket_is_map(tokens, i + 3) { "map" } else { "array" }.to_string()),
                _ => None,
            };
            Some(found)
        }
        _ => Some(None),
    }
}

/// token ranges `(keyword, closing brace)` of the top-level functions and structs. their
/// locals aren't visible from the code outside them
fn hidden_ranges(tokens: &[Token], depth: &[i32]) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    for k in 0..tokens.len() {
        if depth[k] == 0 && matches!(tokens[k].kind, T::Function | T::Struct) {
            if let Some(open) = (k..tokens.len()).find(|&j| tokens[j].kind == T::LBrace) {
                let mut nest = 0;
                for j in open..tokens.len() {
                    match tokens[j].kind {
                        T::LBrace => nest += 1,
                        T::RBrace => {
                            nest -= 1;
                            if nest == 0 {
                                out.push((k, j));
                                break;
                            }
                        }
                        _ => {}
                    }
                }
            }
        }
    }
    out
}

/// the declared type name of the variable `head` as seen at token index `at`
fn resolve_head(tokens: &[Token], depth: &[i32], at: usize, head: &str, decls: &Decls) -> Option<String> {
    if head == "me" {
        let s = enclosing(tokens, depth, at, &T::Struct)?;
        return match kind(tokens, s + 1) {
            Some(T::Ident(n)) => Some(n.clone()),
            _ => None,
        };
    }
    let func = enclosing(tokens, depth, at, &T::Function);
    // 1. locals of the enclosing function (or, at the top level, of the script itself,
    //    skipping the insides of functions and structs, which aren't visible from there),
    //    nearest first
    {
        let hidden = if func.is_none() { hidden_ranges(tokens, depth) } else { Vec::new() };
        let from = func.unwrap_or(0);
        for k in (from..at.min(tokens.len())).rev() {
            if hidden.iter().any(|&(a, b)| k > a && k < b) {
                continue;
            }
            match &tokens[k].kind {
                T::Let | T::Const => {
                    if let Some(r) = decl_type(tokens, k, head, decls) {
                        return r;
                    }
                }
                T::For => {
                    if kind(tokens, k + 1) == Some(&T::Ident(head.to_string())) && kind(tokens, k + 2) == Some(&T::In) {
                        return None; // a loop variable: its type is not written anywhere
                    }
                }
                T::Catch => {
                    // `catch (e)` / `catch (e: string)`: what's caught is always a string
                    if let (Some(T::LParen), Some(T::Ident(n))) = (kind(tokens, k + 1), kind(tokens, k + 2)) {
                        if n == head {
                            return Some("string".to_string());
                        }
                    }
                }
                _ => {}
            }
        }
    }
    if let Some(f) = func {
        // 2. its params
        if let (Some(T::Ident(_)), Some(T::LParen)) = (kind(tokens, f + 1), kind(tokens, f + 2)) {
            if let Some(close) = matching_paren(tokens, f + 2) {
                for k in f + 3..close {
                    if tokens[k].kind == T::Ident(head.to_string())
                        && kind(tokens, k + 1) == Some(&T::Colon)
                        && matches!(kind(tokens, k - 1), Some(T::LParen) | Some(T::Comma))
                    {
                        let ty = read_type(tokens, k + 2).0;
                        return if ty.is_empty() { None } else { Some(ty) };
                    }
                }
            }
        }
    }
    // 3. top-level let/const (globals): nearest before the cursor, else any
    let globals: Vec<usize> = (0..tokens.len())
        .filter(|&k| depth[k] == 0 && matches!(tokens[k].kind, T::Let | T::Const))
        .collect();
    for &k in globals.iter().rev().filter(|&&k| k < at).chain(globals.iter().filter(|&&k| k >= at)) {
        if let Some(r) = decl_type(tokens, k, head, decls) {
            return r;
        }
    }
    // 4. an exported global of an installed/imported file (its type is written there)
    decls.globals.get(head).cloned()
}

/// the members after `value.`: the type's own list, plus the conversion
/// shortcuts every value has (`x.string()`), and `x.int()` / `x.float()` on
/// numbers (`@into.int` / `@into.float` take them)
fn members_of(ty: &str, decls: &Decls) -> Vec<Member> {
    let mut out = members_of_own(ty, decls);
    // a name starting with `_` is private to its struct. never offered after a dot
    out.retain(|m| !m.name.starts_with('_'));
    let mut shortcut = |name: &str, detail: &str| {
        if !out.iter().any(|m| m.name == name) {
            out.push(Member { name: name.to_string(), detail: detail.to_string(), kind: KIND_METHOD, insert: None, doc: None });
        }
    };
    shortcut("string", "text of this value (@into.string)");
    if matches!(ty, "int" | "float") {
        shortcut("int", "convert to int (@into.int)");
        shortcut("float", "convert to float (@into.float)");
    }
    out
}

fn members_of_own(ty: &str, decls: &Decls) -> Vec<Member> {
    let mut out = Vec::new();
    if let Some(ns) = native_namespace(ty) {
        return native_methods(ns);
    }
    // numbers and bools: every function of the method namespaces that takes that type first
    let prim = match ty {
        "int" => Some(spg_typecheck::typed_ast::Ty::Int),
        "float" => Some(spg_typecheck::typed_ast::Ty::Float),
        "bool" => Some(spg_typecheck::typed_ast::Ty::Bool),
        _ => None,
    };
    if let Some(t) = prim {
        return spg_typecheck::builtins::methods_of(&t)
            .into_iter()
            .map(|(name, params, ret)| Member {
                detail: format!(
                    "{}({}) -> {}",
                    name,
                    params.iter().map(|p| p.to_string()).collect::<Vec<_>>().join(", "),
                    ret
                ),
                name,
                kind: KIND_METHOD,
                insert: None,
             doc: None, })
            .collect();
    }
    if let Some(info) = decls.structs.get(ty) {
        for (n, t) in &info.fields {
            out.push(Member { name: n.clone(), detail: t.clone(), kind: KIND_FIELD, insert: None, doc: None });
        }
        for (n, sig) in &info.methods {
            out.push(Member { name: n.clone(), detail: format!("{}{}", n, sig), kind: KIND_METHOD, insert: None, doc: None });
        }
    } else if decls.enums.contains(ty) {
        out.push(Member {
            name: "value".to_string(),
            detail: "the value this variant was declared with".to_string(),
            kind: KIND_FIELD,
            insert: None,
         doc: None, });
    }
    out
}

/// what to offer after `chain.`. `chain` is the dotted names before the dot
/// (`["p"]`, `["a", "pos"]`, `["Color", "Red"]`). `at` is the number of
/// tokens before the word being typed
/// `(params) -> R` for the parameter list opening at `open`, and the index after it
fn signature_at(tokens: &[Token], open: usize) -> Option<(String, usize)> {
    let close = matching_paren(tokens, open)?;
    let mut params = String::new();
    for t in &tokens[open + 1..close] {
        match &t.kind {
            T::Ident(n) => params.push_str(n),
            k => params.push_str(tok_text(k)),
        }
    }
    let mut sig = format!("({})", params);
    let mut next = close + 1;
    if kind(tokens, next) == Some(&T::Arrow) {
        let (ret, after) = read_type(tokens, next + 1);
        sig.push_str(&format!(" -> {}", ret));
        next = after;
    }
    Some((sig, next))
}

/// the associated functions of type `ty` known to this program (its own file and imports)
pub fn associated_of(decls: &Decls, ty: &str) -> Vec<Member> {
    decls
        .associated
        .iter()
        .filter(|(t, _, _)| t == ty)
        .map(|(t, name, sig)| Member {
            name: name.clone(),
            detail: format!("{}.{}{}", t, name, sig),
            kind: 3,
            insert: None,
            doc: Some("associated function: no instance, call it as Type.name()"),
        })
        .collect()
}

pub fn after_dot(tokens: &[Token], at: usize, chain: &[String], decls: &Decls) -> Vec<Member> {
    if chain.is_empty() {
        return Vec::new();
    }
    // `Ponto.` where Ponto is a struct or enum name: its associated functions
    // (`function Ponto.nome(...)`), the way they are called. not a variable of that name
    if chain.len() == 1 && (decls.structs.contains_key(&chain[0]) || decls.enums.contains(&chain[0])) {
        let depth = depths(tokens);
        if resolve_head(tokens, &depth, at, &chain[0], decls).is_none() {
            return associated_of(decls, &chain[0]);
        }
    }
    let depth = depths(tokens);
    let mut ty = if decls.enums.contains(&chain[0]) {
        // `Color.Red.`: a variant of the enum
        if chain.len() == 2 { Some(chain[0].clone()) } else { return Vec::new() }
    } else {
        let head = resolve_head(tokens, &depth, at, &chain[0], decls);
        let mut cur = head;
        for seg in &chain[1..] {
            cur = cur
                .and_then(|t| decls.structs.get(&t).cloned())
                .and_then(|s| s.fields.into_iter().find(|(n, _)| n == seg))
                .map(|(_, t)| t);
        }
        cur
    };
    match ty.take() {
        Some(t) => members_of(&t, decls),
        None => Vec::new(),
    }
}

/// inside `Name { `: the fields of the struct literal being written that are
/// not there yet, offered as `field: `. `None` when the cursor isn't at a
/// field-name position of a struct literal.
pub fn literal_fields(tokens: &[Token], at: usize, decls: &Decls) -> Option<Vec<Member>> {
    let at = at.min(tokens.len());
    // a field name goes right after `{` or `,`
    let mut k = at;
    loop {
        if k == 0 {
            return None;
        }
        k -= 1;
        if tokens[k].kind != T::Newline {
            break;
        }
    }
    if !matches!(tokens[k].kind, T::LBrace | T::Comma) {
        return None;
    }
    // walk back to the unmatched `{`, collecting the field names already written
    let mut written: HashSet<String> = HashSet::new();
    let mut nest = 0;
    let mut open = None;
    let mut j = at;
    while j > 0 {
        j -= 1;
        match &tokens[j].kind {
            T::RBrace | T::RParen | T::RBracket => nest += 1,
            T::LBrace => {
                if nest == 0 {
                    open = Some(j);
                    break;
                }
                nest -= 1;
            }
            T::LParen | T::LBracket => {
                if nest == 0 {
                    return None; // inside call arguments / an index, not directly in the literal
                }
                nest -= 1;
            }
            T::Ident(n) if nest == 0 && kind(tokens, j + 1) == Some(&T::Colon) => {
                if j > 0 && matches!(tokens[j - 1].kind, T::LBrace | T::Comma | T::Newline) {
                    written.insert(n.clone());
                }
            }
            _ => {}
        }
    }
    let open = open?;
    if open < 1 {
        return None;
    }
    let name = match &tokens[open - 1].kind {
        T::Ident(n) => n,
        _ => return None,
    };
    // `struct Name {` (the declaration) and `-> Name {` (a function body) aren't literals
    if open >= 2 && matches!(tokens[open - 2].kind, T::Struct | T::Arrow) {
        return None;
    }
    let info = decls.structs.get(name)?;
    Some(
        info.fields
            .iter()
            .filter(|(n, _)| !written.contains(n))
            .map(|(n, t)| Member {
                name: n.clone(),
                detail: t.clone(),
                kind: KIND_FIELD,
                insert: Some(format!("{}: ", n)),
             doc: None, })
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// tokens of `text` and the number of them before (line, col) (both 0-based)
    fn setup(text: &str, line: usize, col: usize) -> (Vec<Token>, usize, Decls) {
        let tokens = spg_lexer::Lexer::new(text).tokenize().unwrap();
        let at = tokens.iter().filter(|t| (t.line - 1, t.col - 1) < (line, col)).count();
        let decls = Decls::from_tokens(&tokens);
        (tokens, at, decls)
    }

    fn names(ms: &[Member]) -> Vec<String> {
        ms.iter().map(|m| m.name.clone()).collect()
    }

    const SRC: &str = "struct Point {\n    x: int,\n    y: int\n    function len2() -> int { return me.x * me.x + me.y * me.y }\n    function shift(dx: int, dy: int) { me.x += dx }\n}\nstruct Line { a: Point, b: Point, tag: array<string> }\nenum Color { Red, Green }\nfunction origin() -> Point { return Point { x: 0, y: 0 } }\n";

    #[test]
    fn type_name_dot_lists_its_associated_functions() {
        let text = "struct Ponto { x: int }\nfunction Ponto.origem() -> Ponto { return Ponto { x: 0 } }\nfunction Ponto.de(x: int) -> Ponto { return Ponto { x: x } }\n";
        let (tokens, at, decls) = setup(text, 3, 0);
        let got = names(&after_dot(&tokens, at, &["Ponto".to_string()], &decls));
        assert!(got.contains(&"origem".to_string()) && got.contains(&"de".to_string()), "{:?}", got);
    }

    #[test]
    fn associated_functions_of_imported_files_are_listed() {
        let lex = |t: &str| spg_lexer::Lexer::new(t).tokenize().unwrap();
        let mut here = Decls::from_tokens(&lex("struct Ponto { x: int }\nlet p = 1\n"));
        let there = Decls::from_tokens(&lex("struct Ponto { x: int }\nfunction Ponto.origem() -> Ponto { return Ponto { x: 0 } }\n"));
        here.merge(there);
        let got = names(&associated_of(&here, "Ponto"));
        assert_eq!(got, vec!["origem".to_string()]);
        assert_eq!(associated_of(&here, "Outro").len(), 0);
    }

    #[test]
    fn struct_table_has_fields_methods_and_signatures() {
        let (_, _, d) = setup(SRC, 0, 0);
        let p = &d.structs["Point"];
        assert_eq!(p.fields, vec![("x".into(), "int".into()), ("y".into(), "int".into())]);
        assert_eq!(p.methods[0], ("len2".to_string(), "() -> int".to_string()));
        assert_eq!(p.methods[1], ("shift".to_string(), "(dx: int, dy: int)".to_string()));
        assert_eq!(d.structs["Line"].fields[2], ("tag".to_string(), "array<string>".to_string()));
        assert!(d.enums.contains("Color"));
        assert_eq!(d.functions["origin"], "Point");
    }

    #[test]
    fn dot_after_a_local_a_param_a_global_and_me() {
        let local = format!("{}function f() {{\n    let p = Point {{ x: 1, y: 2 }}\n    p.\n}}\n", SRC);
        let (t, at, d) = setup(&local, 11, 6);
        assert_eq!(names(&after_dot(&t, at, &["p".into()], &d)), ["x", "y", "len2", "shift", "string"]);

        let param = format!("{}function f(q: Point, n: int) {{\n    q.\n}}\n", SRC);
        let (t, at, d) = setup(&param, 10, 6);
        assert_eq!(names(&after_dot(&t, at, &["q".into()], &d)), ["x", "y", "len2", "shift", "string"]);
        assert!(names(&after_dot(&t, at, &["n".into()], &d)).iter().any(|n| n == "abs"), "an int offers its @math methods");

        let global = format!("{}let g = origin()\nfunction f() {{\n    g.\n}}\n", SRC);
        let (t, at, d) = setup(&global, 11, 6);
        assert_eq!(names(&after_dot(&t, at, &["g".into()], &d)).len(), 5);

        let me = "struct S {\n    n: int\n    function f() -> int {\n        return me.\n    }\n}\n";
        let (t, at, d) = setup(me, 3, 18);
        assert_eq!(names(&after_dot(&t, at, &["me".into()], &d)), ["n", "f", "string"]);
    }

    #[test]
    fn chains_follow_field_types_and_enums_offer_value() {
        let src = format!("{}let l = Line {{ a: origin(), b: origin(), tag: [] }}\nlet c = Color.Red\nl.a.\n", SRC);
        let (t, at, d) = setup(&src, 11, 4);
        assert_eq!(names(&after_dot(&t, at, &["l".into(), "a".into()], &d)).len(), 5);
        let tag = names(&after_dot(&t, at, &["l".into(), "tag".into()], &d));
        assert!(tag.iter().any(|n| n == "push"), "array<string> offers the array methods: {:?}", tag);
        assert!(names(&after_dot(&t, at, &["l".into(), "a".into(), "x".into()], &d)).iter().any(|n| n == "abs"), "an int field offers its @math methods");
        assert_eq!(names(&after_dot(&t, at, &["c".into()], &d)), ["value", "string"]);
        assert_eq!(names(&after_dot(&t, at, &["Color".into(), "Red".into()], &d)), ["value", "string"]);
    }

    #[test]
    fn a_ps_result_offers_its_fields() {
        // `ps.run` gives back a PsResult, and its fields are there once `ps` is installed
        let (t, at, d) = setup("install <std: \"ps\">\nlet r = ps.run(\"x\")\nr.\n", 2, 2);
        let got = names(&after_dot(&t, at, &["r".into()], &d));
        for want in ["success", "code", "output", "error"] {
            assert!(got.iter().any(|n| n == want), "PsResult misses {}: {:?}", want, got);
        }
    }

    #[test]
    fn a_task_offers_wait_and_the_any_value_conversion() {
        // `t.wait()` is the task's own method. `string` is the conversion every value gets, as with maps.
        // `@task.all` takes a list, so it stays out
        let (t, at, d) = setup("let k: task<int> = 0\nk.\n", 1, 2);
        assert_eq!(names(&after_dot(&t, at, &["k".into()], &d)), ["wait", "string"]);
    }

    #[test]
    fn strings_arrays_and_maps_offer_their_native_methods() {
        let src = format!(
            "{}let xs: array<int> = [1]\nlet s = \"a\"\nlet m = [\"k\": 1]\nlet e = [:]\nlet n = [1, 2]\nlet l = Line {{ a: origin(), b: origin(), tag: [] }}\nl.tag.\n",
            SRC
        );
        let (t, at, d) = setup(&src, 15, 6);
        let arr = names(&after_dot(&t, at, &["xs".into()], &d));
        for want in ["push", "pop", "len", "insert", "remove", "contains", "sum", "join"] {
            assert!(arr.iter().any(|n| n == want), "array methods miss {}: {:?}", want, arr);
        }
        assert!(!arr.iter().any(|n| n == "range" || n == "fill"), "range/fill do not take an array first: {:?}", arr);
        assert_eq!(names(&after_dot(&t, at, &["n".into()], &d)), arr, "a literal array is an array");
        let st = names(&after_dot(&t, at, &["s".into()], &d));
        assert!(st.iter().any(|n| n == "trim") && st.iter().any(|n| n == "padStart") && !st.iter().any(|n| n == "push"));
        let mp = names(&after_dot(&t, at, &["m".into()], &d));
        assert_eq!(mp, ["get", "has", "keys", "keysOf", "len", "remove", "values", "string"]);
        assert_eq!(names(&after_dot(&t, at, &["e".into()], &d)), mp, "`[:]` is a map");
        // a struct field of type array<string>
        assert_eq!(names(&after_dot(&t, at, &["l".into(), "tag".into()], &d)), arr);
        // the detail drops the receiver
        let push = after_dot(&t, at, &["xs".into()], &d).into_iter().find(|m| m.name == "push").unwrap();
        assert_eq!(push.detail, "push(any) -> void");
    }

    #[test]
    fn the_catch_variable_is_a_string() {
        let src = "try {\n    print(1)\n} catch (e) {\n    e.\n}\n";
        let (t, at, d) = setup(src, 3, 6);
        let m = names(&after_dot(&t, at, &["e".into()], &d));
        assert!(m.iter().any(|n| n == "trim") && m.iter().any(|n| n == "len"), "{:?}", m);
    }

    #[test]
    fn a_loop_variable_or_an_unknown_name_offers_nothing() {
        let src = format!("{}function f(ps: array<Point>) {{\n    for p in ps {{\n        p.\n    }}\n}}\n", SRC);
        let (t, at, d) = setup(&src, 11, 10);
        assert!(after_dot(&t, at, &["p".into()], &d).is_empty());
        assert!(after_dot(&t, at, &["nope".into()], &d).is_empty());
    }

    #[test]
    fn shadowing_picks_the_nearest_declaration() {
        let src = format!("{}let p = Line {{ a: origin(), b: origin(), tag: [] }}\nfunction f() {{\n    let p = origin()\n    p.\n}}\n", SRC);
        let (t, at, d) = setup(&src, 12, 6);
        assert_eq!(names(&after_dot(&t, at, &["p".into()], &d)), ["x", "y", "len2", "shift", "string"]);
    }

    #[test]
    fn struct_literal_offers_the_missing_fields_only_in_name_position() {
        let base = format!("{}let a = Line {{ ", SRC);
        let (t, at, d) = setup(&base, 9, 15);
        assert_eq!(names(&literal_fields(&t, at, &d).unwrap()), ["a", "b", "tag"]);

        let some = format!("{}let a = Line {{ a: origin(), \n", SRC);
        let (t, at, d) = setup(&some, 10, 0);
        let m = literal_fields(&t, at, &d).unwrap();
        assert_eq!(names(&m), ["b", "tag"]);
        assert_eq!(m[0].insert.as_deref(), Some("b: "));

        // after `field:` it's a value position, and a call / a block aren't literals
        let val = format!("{}let a = Line {{ a: ", SRC);
        let (t, at, d) = setup(&val, 9, 18);
        assert!(literal_fields(&t, at, &d).is_none());
        let call = format!("{}let a = f(Line {{ a: origin(), b: g(", SRC);
        let (t, at, d) = setup(&call, 9, 35);
        assert!(literal_fields(&t, at, &d).is_none());
        let block = "struct P { x: int }\nlet n = 1\nif n == 1 {\n    \n}\n";
        let (t, at, d) = setup(block, 3, 4);
        assert!(literal_fields(&t, at, &d).is_none());
        let decl = "struct P { x: int, \n";
        let (t, at, d) = setup(decl, 1, 0);
        assert!(literal_fields(&t, at, &d).is_none(), "the declaration itself is not a literal");
    }
}
