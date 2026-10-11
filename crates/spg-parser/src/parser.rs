//! Turns tokens into an AST.
//!
//! Classic recursive descent, with operator precedence handled by a
//! chain of functions (parse_or -> parse_and -> parse_equality -> ... ->
//! parse_primary).
//!
//! Special rule: `if` is parsed as `Expr::IfExpr` when it shows up in
//! expression position (e.g. the right side of a `let`) — there, `else`
//! is REQUIRED and its absence is a parse error, not something left for
//! the typechecker to catch later.

use crate::ast::*;
use spg_lexer::{Token, TokenKind};
use std::fmt;

#[derive(Debug)]
pub enum ParseError {
    Unexpected {
        expected: String,
        found: TokenKind,
        line: usize,
        col: usize,
    },
    MissingElseInIfExpr { line: usize, col: usize },
    UnexpectedEof { expected: String },
    /// a namespace (`@math`, `@time`, ...) in a place that needs a plain
    /// name: variable, function, parameter, struct, field, type or import
    ReservedName { name: String, line: usize, col: usize },
    /// a namespace used without `.member` (e.g. `let x = math`)
    NamespaceWithoutMember { name: String, line: usize, col: usize },
    /// `string.len(...)`: `string` is a type keyword, so the old namespace
    /// spelling can't even reach the typechecker
    OldStringNamespace { member: String, line: usize, col: usize },
    /// `a..b` anywhere but a `for .. in` header (`let r = 1..4`)
    RangeOutsideFor { line: usize, col: usize },
    /// a struct with a field AND a method of the same name (`p.x` would be ambiguous)
    FieldMethodClash { struct_name: String, name: String, line: usize, col: usize },
    /// `enum E { A, A }`
    DuplicateVariant { enum_name: String, name: String, line: usize, col: usize },
    /// `enum E { }` — an enum needs at least one variant
    EmptyEnum { name: String, line: usize, col: usize },
    /// `enum E { A = 1, B = "x" }` — the variants of one enum share one value type
    EnumValueType { enum_name: String, name: String, found: &'static str, expected: &'static str, line: usize, col: usize },
    /// a variant with no `= value` where no number can be made up for it
    EnumValueNeeded { enum_name: String, name: String, line: usize, col: usize },
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ParseError::Unexpected { expected, found, line, col } => write!(
                f,
                "expected {}, found {:?} at line {}, column {}",
                expected, found, line, col
            ),
            ParseError::MissingElseInIfExpr { line, col } => write!(
                f,
                "'if' used as an expression requires a mandatory 'else' (line {}, column {})",
                line, col
            ),
            ParseError::UnexpectedEof { expected } => {
                write!(f, "unexpected end of file, expected {}", expected)
            }
            ParseError::ReservedName { name, line, col } => write!(
                f,
                "'@{}' is a reserved namespace and cannot be used as a name (line {}, column {})",
                name, line, col
            ),
            ParseError::OldStringNamespace { member, line, col } => write!(
                f,
                "'string.{}' uses the old namespace spelling; use '@str.{}' (line {}, column {})",
                member, member, line, col
            ),
            ParseError::FieldMethodClash { struct_name, name, line, col } => write!(
                f,
                "struct '{}' has both a field and a method named '{}' (line {}, column {})",
                struct_name, name, line, col
            ),
            ParseError::DuplicateVariant { enum_name, name, line, col } => write!(
                f,
                "enum '{}' already has a variant named '{}' (line {}, column {})",
                enum_name, name, line, col
            ),
            ParseError::EnumValueType { enum_name, name, found, expected, line, col } => write!(
                f,
                "enum '{}': variant '{}' has a {} value but the earlier variants are {} (all variants of one enum share one type; a variant without '=' counts as an int) (line {}, column {})",
                enum_name, name, found, expected, line, col
            ),
            ParseError::EnumValueNeeded { enum_name, name, line, col } => write!(
                f,
                "enum '{}': variant '{}' needs a value ('{} = ...'); only int enums are numbered automatically (line {}, column {})",
                enum_name, name, name, line, col
            ),
            ParseError::EmptyEnum { name, line, col } => write!(
                f,
                "enum '{}' needs at least one variant (line {}, column {})",
                name, line, col
            ),
            ParseError::RangeOutsideFor { line, col } => write!(
                f,
                "'..' only works in a for loop header ('for i in 1..4 {{ }}'); to get the numbers as an array use '@array.range(1, 4)' (line {}, column {})",
                line, col
            ),
            ParseError::NamespaceWithoutMember { name, line, col } => write!(
                f,
                "'@{}' is a namespace; use '@{}.<name>(...)' (line {}, column {})",
                name, name, line, col
            ),
        }
    }
}

impl std::error::Error for ParseError {}

pub struct Parser {
    tokens: Vec<Token>,
    pos: usize,
    /// true while parsing an `if`/`while` condition or a `for .. in`
    /// iterable — in those positions, `Ident {` is the following
    /// block's brace, not the start of a struct literal. Mirrors the
    /// same ambiguity (and the same fix) Rust itself has with struct
    /// literals in condition position. Only `parse_primary`'s struct
    /// literal branch reads this; every other expression form parses
    /// identically regardless.
    no_struct_literal: bool,
    /// the methods of the `struct` just parsed, already lowered to plain
    /// `FunctionDecl`s (`P.f`, with `me: P` as the first parameter). A
    /// statement list (`parse_program`, `parse_block`) drains this right
    /// after the struct itself, so they land next to it in source order.
    pending: Vec<Stmt>,
}

impl Parser {
    pub fn new(tokens: Vec<Token>) -> Self {
        Parser { tokens, pos: 0, no_struct_literal: false, pending: Vec::new() }
    }

    pub fn parse_program(mut self) -> Result<Program, ParseError> {
        let mut stmts = Vec::new();
        while !self.check(&TokenKind::Eof) {
            stmts.push(self.parse_stmt()?);
            stmts.append(&mut self.pending);
        }
        Ok(Program { stmts })
    }

    // navigation helpers

    fn peek(&self) -> &Token {
        &self.tokens[self.pos]
    }

    /// whether the token after the current one is `kind` (one token of lookahead)
    fn next_is(&self, kind: &TokenKind) -> bool {
        self.tokens.get(self.pos + 1).is_some_and(|t| std::mem::discriminant(&t.kind) == std::mem::discriminant(kind))
    }

    fn peek_pos(&self) -> Pos {
        let t = self.peek();
        Pos { line: t.line, col: t.col, file: None }
    }

    fn check(&self, kind: &TokenKind) -> bool {
        std::mem::discriminant(&self.peek().kind) == std::mem::discriminant(kind)
    }

    fn advance(&mut self) -> Token {
        let t = self.tokens[self.pos].clone();
        if !self.check(&TokenKind::Eof) {
            self.pos += 1;
        }
        t
    }

    fn expect(&mut self, kind: TokenKind, what: &str) -> Result<Token, ParseError> {
        if self.check(&kind) {
            Ok(self.advance())
        } else {
            let t = self.peek();
            Err(ParseError::Unexpected {
                expected: what.to_string(),
                found: t.kind.clone(),
                line: t.line,
                col: t.col,
            })
        }
    }

    fn match_kind(&mut self, kind: &TokenKind) -> bool {
        if self.check(kind) {
            self.advance();
            true
        } else {
            false
        }
    }

    // Statements

    fn parse_stmt(&mut self) -> Result<Stmt, ParseError> {
        match &self.peek().kind {
            TokenKind::Let => self.parse_let(false, false),
            TokenKind::Const => self.parse_let(true, false),
            TokenKind::If => self.parse_if_stmt(),
            TokenKind::While => self.parse_while(),
            TokenKind::For => self.parse_for_in(),
            TokenKind::Return => self.parse_return(),
            TokenKind::Break => {
                let pos = self.peek_pos();
                self.advance();
                Ok(Stmt::Break { pos })
            }
            TokenKind::Continue => {
                let pos = self.peek_pos();
                self.advance();
                Ok(Stmt::Continue { pos })
            }
            TokenKind::Try => self.parse_try(),
            TokenKind::Throw => {
                let pos = self.peek_pos();
                self.advance(); // throw
                let value = self.parse_expr()?;
                Ok(Stmt::Throw { value, pos })
            }
            // `function (` at statement start is an anonymous function used as an
            // expression statement (`function() { ... }()`), not a declaration
            TokenKind::Function if !self.next_is(&TokenKind::LParen) => self.parse_function_decl(false),
            TokenKind::Struct => self.parse_struct_decl(false),
            TokenKind::Enum => self.parse_enum_decl(false),
            TokenKind::Export => self.parse_export(),
            TokenKind::Import => self.parse_import(),
            TokenKind::Install => self.parse_install(),
            // `x = ...`, `arr[i] = ...`, and their compound forms
            // (`x += ...`, `arr[i] -= ...`) all start as an ordinary
            // expression (Ident, or Ident followed by one or more
            // postfix `[...]`/`(...)`) — parse that expression once,
            // then decide from its shape plus the trailing operator
            // which statement this actually is. Parsing the target
            // expression exactly once matters here: `arr[f()] += 1`
            // must call `f()` a single time, not once per desugared
            // occurrence of the index.
            _ => {
                let expr = self.parse_expr()?;

                if self.check(&TokenKind::Eq) {
                    let pos = self.peek_pos();
                    self.advance(); // =
                    let value = self.parse_expr()?;
                    return self.assign_stmt_from_target(expr, value, pos);
                }

                if let Some(op) = self.peek_compound_op() {
                    let pos = self.peek_pos();
                    self.advance(); // +=/-=/*=//=
                    let rhs = self.parse_expr()?;
                    return match expr {
                        // plain `x OP= rhs` desugars to `x = x OP rhs`
                        // right here — rereading a local has no side
                        // effect, so duplicating it in the AST is free
                        Expr::Ident(name, ipos) => {
                            let value = Expr::Binary {
                                left: Box::new(Expr::Ident(name.clone(), ipos)),
                                op,
                                right: Box::new(rhs),
                                pos: pos.clone(),
                            };
                            Ok(Stmt::Assign { name, value, pos })
                        }
                        // `arr[index] OP= rhs` keeps `array`/`index` as
                        // single subexpressions via a dedicated node
                        // instead — see IndexCompoundAssign's doc comment
                        // for why desugaring here would be wrong
                        Expr::Index { array, index, .. } => {
                            Ok(Stmt::IndexCompoundAssign { array: *array, index: *index, op, value: rhs, pos })
                        }
                        // `obj.field OP= rhs` — same reasoning as the
                        // index case (object is evaluated once), see
                        // FieldCompoundAssign's doc comment
                        Expr::Field { object, field, .. } => {
                            Ok(Stmt::FieldCompoundAssign { object: *object, field, op, value: rhs, pos })
                        }
                        _other => Err(ParseError::Unexpected {
                            expected: "an assignable target ('name', 'name[index]', or 'name.field')".to_string(),
                            found: TokenKind::Eq,
                            line: pos.line,
                            col: pos.col,
                        }),
                    };
                }

                Ok(Stmt::ExprStmt(expr))
            }
        }
    }

    /// true when the current token is one of the compound-assignment
    /// operators (+=, -=, *=, /=), mapped to the `BinaryOp` it desugars
    /// to. No %= by design — an accumulating modulo has no real use case
    /// here, so it was deliberately left out rather than added for
    /// completeness.
    fn peek_compound_op(&self) -> Option<BinaryOp> {
        match &self.peek().kind {
            TokenKind::PlusEq => Some(BinaryOp::Add),
            TokenKind::MinusEq => Some(BinaryOp::Sub),
            TokenKind::StarEq => Some(BinaryOp::Mul),
            TokenKind::SlashEq => Some(BinaryOp::Div),
            _ => None,
        }
    }

    /// builds the right `Stmt::Assign`/`Stmt::IndexAssign` from an
    /// already-parsed target expression — shared by plain `=` and every
    /// desugared compound-assignment operator so both paths reject the
    /// same set of non-assignable targets the same way.
    fn assign_stmt_from_target(&self, target: Expr, value: Expr, pos: Pos) -> Result<Stmt, ParseError> {
        match target {
            Expr::Ident(name, _) => Ok(Stmt::Assign { name, value, pos }),
            Expr::Index { array, index, .. } => Ok(Stmt::IndexAssign { array: *array, index: *index, value, pos }),
            Expr::Field { object, field, .. } => Ok(Stmt::FieldAssign { object: *object, field, value, pos }),
            _other => Err(ParseError::Unexpected {
                expected: "an assignable target ('name', 'name[index]', or 'name.field')".to_string(),
                found: TokenKind::Eq,
                line: pos.line,
                col: pos.col,
            }),
        }
    }

    fn parse_let(&mut self, is_const: bool, exported: bool) -> Result<Stmt, ParseError> {
        let pos = self.peek_pos();
        self.advance(); // let/const

        let name = self.parse_binding_name()?;

        let type_ann = if self.match_kind(&TokenKind::Colon) {
            Some(self.parse_type_expr()?)
        } else {
            None
        };

        self.expect(TokenKind::Eq, "'='")?;
        let value = self.parse_expr()?;

        if is_const {
            Ok(Stmt::Const { name, type_ann, value, exported, pos })
        } else {
            Ok(Stmt::Let { name, type_ann, value, exported, pos })
        }
    }

    /// `if (cond) { } when (cond2) { } when (cond3) { } else { }` — `when`
    /// is Flow Code's else-if, ported as-is (same shape, same keyword).
    /// Structurally there's no new AST node: each `when` just becomes a
    /// nested `IfStmt` living inside the previous branch's `else_branch`
    /// — the same trick "else if" chains use under the hood in most
    /// languages, so spg-typecheck and spg-compiler need zero changes.
    fn parse_if_stmt(&mut self) -> Result<Stmt, ParseError> {
        let pos = self.peek_pos();
        self.advance(); // if

        self.expect(TokenKind::LParen, "'('")?;
        let cond = self.parse_expr_no_struct_literal()?;
        self.expect(TokenKind::RParen, "')'")?;

        let then_branch = self.parse_block()?;
        let else_branch = self.parse_when_or_else_tail()?;

        Ok(Stmt::IfStmt { cond, then_branch, else_branch, pos })
    }

    /// parses whatever comes after an `if`'s (or a previous `when`'s)
    /// body: another `when (cond) { }` — which recurses, wrapping itself
    /// in a one-statement Block so it fits `else_branch: Option<Block>`
    /// unchanged and keeps the chain going — or a trailing `else { }`,
    /// or neither.
    fn parse_when_or_else_tail(&mut self) -> Result<Option<Block>, ParseError> {
        if self.check(&TokenKind::When) {
            let when_pos = self.peek_pos();
            self.advance(); // when
            self.expect(TokenKind::LParen, "'('")?;
            let when_cond = self.parse_expr_no_struct_literal()?;
            self.expect(TokenKind::RParen, "')'")?;
            let when_body = self.parse_block()?;
            let rest = self.parse_when_or_else_tail()?;

            Ok(Some(Block {
                stmts: vec![Stmt::IfStmt { cond: when_cond, then_branch: when_body, else_branch: rest, pos: when_pos }],
                pos: self.peek_pos(),
            }))
        } else if self.match_kind(&TokenKind::Else) {
            Ok(Some(self.parse_block()?))
        } else {
            Ok(None)
        }
    }

    fn parse_while(&mut self) -> Result<Stmt, ParseError> {
        let pos = self.peek_pos();
        self.advance(); // while

        self.expect(TokenKind::LParen, "'('")?;
        let cond = self.parse_expr_no_struct_literal()?;
        self.expect(TokenKind::RParen, "')'")?;

        let body = self.parse_block()?;
        Ok(Stmt::While { cond, body, pos })
    }

    fn parse_for_in(&mut self) -> Result<Stmt, ParseError> {
        let pos = self.peek_pos();
        self.advance(); // for

        let var_name = self.parse_binding_name()?;
        self.expect(TokenKind::In, "'in'")?;
        let iterable = self.parse_expr_no_struct_literal()?;

        // `a..b` / `a..b step s`: the first expression was the START
        if self.match_kind(&TokenKind::DotDot) {
            let end = self.parse_expr_no_struct_literal()?;
            let step = if matches!(&self.peek().kind, TokenKind::Ident(w) if w == "step") {
                self.advance(); // step
                Some(self.parse_expr_no_struct_literal()?)
            } else {
                None
            };
            let body = self.parse_block()?;
            return Ok(Stmt::ForRange { var_name, start: iterable, end, step, body, pos });
        }

        let body = self.parse_block()?;

        Ok(Stmt::ForIn { var_name, iterable, body, pos })
    }

    fn parse_return(&mut self) -> Result<Stmt, ParseError> {
        let pos = self.peek_pos();
        self.advance(); // return

        // no value: the next token is '}' or already ends the statement
        let value = if self.check(&TokenKind::RBrace) {
            None
        } else {
            Some(self.parse_expr()?)
        };

        Ok(Stmt::Return { value, pos })
    }

    fn parse_try(&mut self) -> Result<Stmt, ParseError> {
        let pos = self.peek_pos();
        self.advance(); // try

        let try_block = self.parse_block()?;
        self.expect(TokenKind::Catch, "'catch'")?;
        self.expect(TokenKind::LParen, "'('")?;
        let catch_var = self.parse_binding_name()?;

        let catch_type = if self.match_kind(&TokenKind::Colon) {
            Some(self.parse_type_expr()?)
        } else {
            None
        };

        self.expect(TokenKind::RParen, "')'")?;
        let catch_block = self.parse_block()?;

        Ok(Stmt::Try { try_block, catch_var, catch_type, catch_block, pos })
    }

    fn parse_export(&mut self) -> Result<Stmt, ParseError> {
        let export_pos = self.peek_pos();
        self.advance(); // export
        match &self.peek().kind {
            TokenKind::Function => self.parse_function_decl(true),
            TokenKind::Struct => self.parse_struct_decl(true),
            TokenKind::Enum => self.parse_enum_decl(true),
            TokenKind::Let => self.parse_let(false, true),
            TokenKind::Const => self.parse_let(true, true),
            TokenKind::LBrace => self.parse_re_export(export_pos),
            _ => {
                let t = self.peek();
                Err(ParseError::Unexpected {
                    expected: "'function', 'struct', 'enum', 'let', 'const' or '{' after 'export'".to_string(),
                    found: t.kind.clone(),
                    line: t.line,
                    col: t.col,
                })
            }
        }
    }

    fn parse_import(&mut self) -> Result<Stmt, ParseError> {
        let pos = self.peek_pos();
        self.advance(); // import
        let (names, from) = self.parse_name_list_from(&pos)?;
        Ok(Stmt::Import { names, from, pos })
    }

    /// `install "./file.spg"` — every export of the file, no name list
    fn parse_install(&mut self) -> Result<Stmt, ParseError> {
        let pos = self.peek_pos();
        self.advance(); // install
        let from = match self.advance().kind {
            TokenKind::Str(s) => s,
            other => {
                return Err(ParseError::Unexpected {
                    expected: "string with the module path".to_string(),
                    found: other,
                    line: pos.line,
                    col: pos.col,
                })
            }
        };
        Ok(Stmt::Install { from, pos })
    }

    /// `export { a, b } from "./y.spg"` — the `export` keyword is already
    /// consumed, the cursor sits on `{`.
    fn parse_re_export(&mut self, pos: Pos) -> Result<Stmt, ParseError> {
        let (names, from) = self.parse_name_list_from(&pos)?;
        Ok(Stmt::ReExport { names, from, pos })
    }

    /// the `{ a, b } from "path"` tail that `import` and `export ... from`
    /// have in common
    fn parse_name_list_from(&mut self, pos: &Pos) -> Result<(Vec<String>, String), ParseError> {
        self.expect(TokenKind::LBrace, "'{'")?;

        let mut names = Vec::new();
        if !self.check(&TokenKind::RBrace) {
            names.push(self.parse_binding_name()?);
            while self.match_kind(&TokenKind::Comma) {
                names.push(self.parse_binding_name()?);
            }
        }

        self.expect(TokenKind::RBrace, "'}'")?;
        self.expect(TokenKind::From, "'from'")?;

        let from = match self.advance().kind {
            TokenKind::Str(s) => s,
            other => {
                return Err(ParseError::Unexpected {
                    expected: "string with the module path".to_string(),
                    found: other,
                    line: pos.line,
                    col: pos.col,
                })
            }
        };

        Ok((names, from))
    }

    fn parse_function_decl(&mut self, exported: bool) -> Result<Stmt, ParseError> {
        let pos = self.peek_pos();
        self.advance(); // function

        let name = self.parse_binding_name()?;
        self.expect(TokenKind::LParen, "'('")?;

        let mut params = Vec::new();
        if !self.check(&TokenKind::RParen) {
            params.push(self.parse_param()?);
            while self.match_kind(&TokenKind::Comma) {
                params.push(self.parse_param()?);
            }
        }
        self.expect(TokenKind::RParen, "')'")?;

        // trailing return type is optional: `-> Type` present keeps the
        // old explicit behavior; absent, the typechecker infers it from
        // the body's return statements instead
        let return_type = if self.match_kind(&TokenKind::Arrow) {
            Some(self.parse_type_expr()?)
        } else {
            None
        };

        let body = self.parse_block()?;

        Ok(Stmt::FunctionDecl { name, params, return_type, body, exported, pos })
    }

    fn parse_struct_decl(&mut self, exported: bool) -> Result<Stmt, ParseError> {
        let pos = self.peek_pos();
        self.advance(); // struct

        let name = self.parse_binding_name()?;
        self.expect(TokenKind::LBrace, "'{'")?;

        // the body mixes fields (`x: int`) and methods (`function f() { }`).
        // Two fields need a comma between them (a trailing comma is fine);
        // a method needs no separator on either side.
        let mut fields: Vec<Param> = Vec::new();
        let mut methods: Vec<Stmt> = Vec::new();
        while !self.check(&TokenKind::RBrace) && !self.check(&TokenKind::Eof) {
            if self.check(&TokenKind::Function) {
                methods.push(self.parse_method(&name, exported)?);
                self.match_kind(&TokenKind::Comma);
            } else {
                fields.push(self.parse_param()?);
                if !self.match_kind(&TokenKind::Comma)
                    && !self.check(&TokenKind::RBrace)
                    && !self.check(&TokenKind::Function)
                {
                    break; // the `expect` below reports the missing ',' or '}'
                }
            }
        }
        self.expect(TokenKind::RBrace, "'}'")?;

        for m in &methods {
            if let Stmt::FunctionDecl { name: full, pos: mpos, .. } = m {
                let short = &full[name.len() + 1..];
                if fields.iter().any(|f| f.name == short) {
                    return Err(ParseError::FieldMethodClash {
                        struct_name: name.clone(),
                        name: short.to_string(),
                        line: mpos.line,
                        col: mpos.col,
                    });
                }
            }
        }
        self.pending.extend(methods);

        Ok(Stmt::StructDecl { name, fields, exported, pos })
    }

    /// `enum Color { Red, Green, Blue }` — variants are plain names separated
    /// by commas (a trailing comma is fine). A variant may carry a literal value
    /// (`Ok = 200`, `Name = "x"`, `On = true`, `Half = 0.5`, negative numbers too).
    /// Without any `=` the variants are the ints 0, 1, 2...; after an int value the
    /// next variant without `=` is that value + 1. Repeating a VALUE is fine,
    /// repeating a NAME is not.
    fn parse_enum_decl(&mut self, exported: bool) -> Result<Stmt, ParseError> {
        let pos = self.peek_pos();
        self.advance(); // enum

        let name = self.parse_binding_name()?;
        self.expect(TokenKind::LBrace, "'{'")?;

        let mut variants: Vec<String> = Vec::new();
        let mut values: Vec<EnumValue> = Vec::new();
        while !self.check(&TokenKind::RBrace) && !self.check(&TokenKind::Eof) {
            let vpos = self.peek_pos();
            let variant = self.parse_ident_name()?;
            if variants.contains(&variant) {
                return Err(ParseError::DuplicateVariant {
                    enum_name: name.clone(),
                    name: variant,
                    line: vpos.line,
                    col: vpos.col,
                });
            }
            let value = if self.match_kind(&TokenKind::Eq) {
                self.parse_enum_value()?
            } else {
                match values.last() {
                    None => EnumValue::Int(0),
                    Some(EnumValue::Int(prev)) => match prev.checked_add(1) {
                        Some(next) => EnumValue::Int(next),
                        None => {
                            return Err(ParseError::EnumValueNeeded {
                                enum_name: name.clone(),
                                name: variant,
                                line: vpos.line,
                                col: vpos.col,
                            })
                        }
                    },
                    Some(_) => {
                        return Err(ParseError::EnumValueNeeded {
                            enum_name: name.clone(),
                            name: variant,
                            line: vpos.line,
                            col: vpos.col,
                        })
                    }
                }
            };
            if let Some(first) = values.first() {
                if first.type_name() != value.type_name() {
                    return Err(ParseError::EnumValueType {
                        enum_name: name.clone(),
                        name: variant,
                        found: value.type_name(),
                        expected: first.type_name(),
                        line: vpos.line,
                        col: vpos.col,
                    });
                }
            }
            variants.push(variant);
            values.push(value);
            if !self.match_kind(&TokenKind::Comma) && !self.check(&TokenKind::RBrace) {
                break; // the `expect` below reports the missing ',' or '}'
            }
        }
        self.expect(TokenKind::RBrace, "'}'")?;

        if variants.is_empty() {
            return Err(ParseError::EmptyEnum { name, line: pos.line, col: pos.col });
        }
        Ok(Stmt::EnumDecl { name, variants, values, exported, pos })
    }

    /// the literal after `=` in an enum variant: int, float (both may start with
    /// `-`), string, `true` or `false`
    fn parse_enum_value(&mut self) -> Result<EnumValue, ParseError> {
        let negative = self.match_kind(&TokenKind::Minus);
        let t = self.advance();
        match (negative, t.kind) {
            (neg, TokenKind::Int(n)) => Ok(EnumValue::Int(if neg { n.wrapping_neg() } else { n })),
            (neg, TokenKind::Float(x)) => Ok(EnumValue::Float(if neg { -x } else { x })),
            (false, TokenKind::Str(s)) => Ok(EnumValue::Str(s)),
            (false, TokenKind::Bool(b)) => Ok(EnumValue::Bool(b)),
            (_, other) => Err(ParseError::Unexpected {
                expected: "a literal value for the variant (int, float, string, true or false)".to_string(),
                found: other,
                line: t.line,
                col: t.col,
            }),
        }
    }

    /// `function f(a: int) -> T { .. }` inside `struct P { .. }`, lowered to the
    /// plain function `P.f(me: P, a: int) -> T { .. }`. The `.` can never appear
    /// in a name the user writes, so these functions cannot clash with anything,
    /// and the module renamer mangles `P` and `P.f` to `<tag>::P` / `<tag>::P.f`
    /// (the typechecker relies on exactly that to find a method from its struct).
    fn parse_method(&mut self, struct_name: &str, exported: bool) -> Result<Stmt, ParseError> {
        match self.parse_function_decl(exported)? {
            Stmt::FunctionDecl { name, mut params, return_type, body, exported, pos } => {
                params.insert(0, Param { name: "me".to_string(), type_ann: TypeExpr::Named(struct_name.to_string()) });
                Ok(Stmt::FunctionDecl {
                    name: format!("{}.{}", struct_name, name),
                    params,
                    return_type,
                    body,
                    exported,
                    pos,
                })
            }
            _ => unreachable!("parse_function_decl always returns a FunctionDecl"),
        }
    }

    /// parameter or struct field: since namespaces now carry an `@`, both
    /// are plain identifiers (`math`, `time`, ... are ordinary names again).
    fn parse_param(&mut self) -> Result<Param, ParseError> {
        let name = self.parse_binding_name()?;
        self.expect(TokenKind::Colon, "':' (type required in parameter/field)")?;
        let type_ann = self.parse_type_expr()?;
        Ok(Param { name, type_ann })
    }

    fn parse_block(&mut self) -> Result<Block, ParseError> {
        let pos = self.peek_pos();
        self.expect(TokenKind::LBrace, "'{'")?;

        let mut stmts = Vec::new();
        while !self.check(&TokenKind::RBrace) && !self.check(&TokenKind::Eof) {
            stmts.push(self.parse_stmt()?);
            stmts.append(&mut self.pending);
        }

        self.expect(TokenKind::RBrace, "'}'")?;
        Ok(Block { stmts, pos })
    }

    /// the name-introducing places (let/const, function, struct, params,
    /// for/catch variables, imports) all go through here. A `@namespace` in
    /// such a spot is rejected by `parse_ident_name` as a reserved name.
    fn parse_binding_name(&mut self) -> Result<String, ParseError> {
        self.parse_ident_name()
    }

    fn parse_ident_name(&mut self) -> Result<String, ParseError> {
        let (line, col) = {
            let t = self.peek();
            (t.line, t.col)
        };
        match self.advance().kind {
            TokenKind::Ident(s) => Ok(s),
            TokenKind::Namespace(name) => Err(ParseError::ReservedName { name, line, col }),
            other => {
                let t = self.peek();
                Err(ParseError::Unexpected {
                    expected: "identifier".to_string(),
                    found: other,
                    line: t.line,
                    col: t.col,
                })
            }
        }
    }

    /// the `member` in `@ns.member`. Besides plain identifiers it accepts the
    /// primitive-type keywords, because `@into.int`, `@into.float` and
    /// `@into.string` are spelled exactly like them.
    fn parse_member_name(&mut self) -> Result<String, ParseError> {
        let t = self.peek().clone();
        let name = match &t.kind {
            TokenKind::Ident(s) => s.clone(),
            TokenKind::TypeInt => "int".to_string(),
            TokenKind::TypeFloat => "float".to_string(),
            TokenKind::TypeBool => "bool".to_string(),
            TokenKind::TypeString => "string".to_string(),
            TokenKind::TypeVoid => "void".to_string(),
            TokenKind::TypeArray => "array".to_string(),
            other => {
                return Err(ParseError::Unexpected {
                    expected: "a namespace member name".to_string(),
                    found: other.clone(),
                    line: t.line,
                    col: t.col,
                })
            }
        };
        self.advance();
        Ok(name)
    }

    // Types

    /// a type, with an optional `?` after it: `int?`, `array<int>?`, `Point?`
    fn parse_type_expr(&mut self) -> Result<TypeExpr, ParseError> {
        let base = self.parse_type_base()?;
        if self.check(&TokenKind::Question) {
            let t = self.peek();
            let (line, col) = (t.line, t.col);
            if matches!(base, TypeExpr::Void) {
                return Err(ParseError::Unexpected {
                    expected: "a type that can hold a value (void? makes no sense)".to_string(),
                    found: TokenKind::Question,
                    line,
                    col,
                });
            }
            self.advance();
            return Ok(TypeExpr::Optional(Box::new(base)));
        }
        Ok(base)
    }

    fn parse_type_base(&mut self) -> Result<TypeExpr, ParseError> {
        let (tline, tcol) = {
            let t = self.peek();
            (t.line, t.col)
        };
        match self.advance().kind {
            TokenKind::Namespace(name) => Err(ParseError::ReservedName { name, line: tline, col: tcol }),
            TokenKind::TypeInt => Ok(TypeExpr::Int),
            TokenKind::TypeFloat => Ok(TypeExpr::Float),
            TokenKind::TypeBool => Ok(TypeExpr::Bool),
            TokenKind::TypeString => Ok(TypeExpr::String),
            TokenKind::TypeVoid => Ok(TypeExpr::Void),
            TokenKind::Function => {
                self.expect(TokenKind::LParen, "'(' (function(T, ...) -> R)")?;
                let mut params = Vec::new();
                if !self.check(&TokenKind::RParen) {
                    params.push(self.parse_type_expr()?);
                    while self.match_kind(&TokenKind::Comma) {
                        params.push(self.parse_type_expr()?);
                    }
                }
                self.expect(TokenKind::RParen, "')' (function(T, ...) -> R)")?;
                self.expect(TokenKind::Arrow, "'->' (function(T, ...) -> R)")?;
                let ret = self.parse_type_expr()?;
                Ok(TypeExpr::Function { params, ret: Box::new(ret) })
            }
            TokenKind::TypeArray => {
                self.expect(TokenKind::Lt, "'<' (array<T>)")?;
                let inner = self.parse_type_expr()?;
                self.expect(TokenKind::Gt, "'>' (array<T>)")?;
                Ok(TypeExpr::Array(Box::new(inner)))
            }
            TokenKind::Ident(name) if name == "map" && self.check(&TokenKind::Lt) => {
                self.advance(); // '<'
                let key = self.parse_type_expr()?;
                self.expect(TokenKind::Comma, "',' (map<K, V>)")?;
                let value = self.parse_type_expr()?;
                self.expect(TokenKind::Gt, "'>' (map<K, V>)")?;
                Ok(TypeExpr::Map(Box::new(key), Box::new(value)))
            }
            TokenKind::Ident(name) => Ok(TypeExpr::Named(name)),
            other => {
                let t = self.peek();
                Err(ParseError::Unexpected {
                    expected: "a type (int, float, bool, string, void, array<T>, map<K, V> or struct name)".to_string(),
                    found: other,
                    line: t.line,
                    col: t.col,
                })
            }
        }
    }

    // Expressions — precedence chain
    // (or -> and -> equality -> comparison -> term -> factor -> unary -> call -> primary)

    fn parse_expr(&mut self) -> Result<Expr, ParseError> {
        // `if` in expression position is caught right here, at the top
        // of the chain
        if self.check(&TokenKind::If) {
            return self.parse_if_expr();
        }
        self.parse_coalesce()
    }

    /// `a ?? b ?? c`: the loosest binary operator, right-associative
    fn parse_coalesce(&mut self) -> Result<Expr, ParseError> {
        let left = self.parse_or()?;
        if self.check(&TokenKind::QuestionQuestion) {
            let pos = self.peek_pos();
            self.advance();
            let right = self.parse_coalesce()?;
            return Ok(Expr::Binary { left: Box::new(left), op: BinaryOp::Coalesce, right: Box::new(right), pos });
        }
        Ok(left)
    }

    /// parses one expression with struct literals disabled — used for
    /// `if`/`while` conditions and `for .. in` iterables, where `Ident {`
    /// would otherwise be read as a struct literal instead of the
    /// following block's opening brace. Saves/restores the previous
    /// flag value (rather than unconditionally clearing it back to
    /// `false`) so nesting is correct: parsing the condition of an `if`
    /// that sits inside another `if`'s condition still disables struct
    /// literals for the inner one too.
    fn parse_expr_no_struct_literal(&mut self) -> Result<Expr, ParseError> {
        let prev = self.no_struct_literal;
        self.no_struct_literal = true;
        let result = self.parse_expr();
        self.no_struct_literal = prev;
        result
    }

    /// `if (cond) { } when (cond2) { } else { }` as an EXPRESSION (used
    /// as a value, e.g. `let x = if (a) {1} when (b) {2} else {3}`).
    /// Same desugaring trick as the statement version: each `when`
    /// becomes a nested `IfExpr` wrapped in a one-statement `Block` so it
    /// fits the existing `else_branch: Box<Block>` field unchanged. The
    /// difference from the statement version is that `else` stays
    /// mandatory at the end of the chain — an if-EXPRESSION must always
    /// produce a value on every path, so the recursion here has no
    /// "no else, no problem" exit; it's `when` all the way down to a
    /// required final `else`.
    fn parse_if_expr(&mut self) -> Result<Expr, ParseError> {
        let pos = self.peek_pos();
        self.advance(); // if

        self.expect(TokenKind::LParen, "'('")?;
        let cond = self.parse_expr_no_struct_literal()?;
        self.expect(TokenKind::RParen, "')'")?;

        let then_branch = self.parse_block()?;
        let else_branch = self.parse_when_or_else_tail_expr()?;

        Ok(Expr::IfExpr { cond: Box::new(cond), then_branch: Box::new(then_branch), else_branch: Box::new(else_branch), pos })
    }

    /// parses what follows an if-expression's (or a previous when's)
    /// body: another `when (cond) { }` (recurses, same wrap-in-Block
    /// trick as the statement version) or the mandatory trailing
    /// `else { }` — there's no "neither" case here, unlike the
    /// statement version, since an if-expression always needs a value.
    fn parse_when_or_else_tail_expr(&mut self) -> Result<Block, ParseError> {
        if self.check(&TokenKind::When) {
            let when_pos = self.peek_pos();
            self.advance(); // when
            self.expect(TokenKind::LParen, "'('")?;
            let when_cond = self.parse_expr_no_struct_literal()?;
            self.expect(TokenKind::RParen, "')'")?;
            let when_body = self.parse_block()?;
            let rest = self.parse_when_or_else_tail_expr()?;

            Ok(Block {
                stmts: vec![Stmt::ExprStmt(Expr::IfExpr {
                    cond: Box::new(when_cond),
                    then_branch: Box::new(when_body),
                    else_branch: Box::new(rest),
                    pos: when_pos,
                })],
                pos: self.peek_pos(),
            })
        } else if self.match_kind(&TokenKind::Else) {
            self.parse_block()
        } else {
            let p = self.peek_pos();
            Err(ParseError::MissingElseInIfExpr { line: p.line, col: p.col })
        }
    }

    fn parse_or(&mut self) -> Result<Expr, ParseError> {
        let mut left = self.parse_and()?;
        while self.check(&TokenKind::OrOr) {
            let pos = self.peek_pos();
            self.advance();
            let right = self.parse_and()?;
            left = Expr::Binary { left: Box::new(left), op: BinaryOp::Or, right: Box::new(right), pos };
        }
        Ok(left)
    }

    fn parse_and(&mut self) -> Result<Expr, ParseError> {
        let mut left = self.parse_equality()?;
        while self.check(&TokenKind::AndAnd) {
            let pos = self.peek_pos();
            self.advance();
            let right = self.parse_equality()?;
            left = Expr::Binary { left: Box::new(left), op: BinaryOp::And, right: Box::new(right), pos };
        }
        Ok(left)
    }

    fn parse_equality(&mut self) -> Result<Expr, ParseError> {
        let mut left = self.parse_comparison()?;
        loop {
            let op = match &self.peek().kind {
                TokenKind::EqEq => BinaryOp::Eq,
                TokenKind::NotEq => BinaryOp::NotEq,
                _ => break,
            };
            let pos = self.peek_pos();
            self.advance();
            let right = self.parse_comparison()?;
            left = Expr::Binary { left: Box::new(left), op, right: Box::new(right), pos };
        }
        Ok(left)
    }

    fn parse_comparison(&mut self) -> Result<Expr, ParseError> {
        let mut left = self.parse_term()?;
        loop {
            let op = match &self.peek().kind {
                TokenKind::Lt => BinaryOp::Lt,
                TokenKind::Gt => BinaryOp::Gt,
                TokenKind::LtEq => BinaryOp::LtEq,
                TokenKind::GtEq => BinaryOp::GtEq,
                _ => break,
            };
            let pos = self.peek_pos();
            self.advance();
            let right = self.parse_term()?;
            left = Expr::Binary { left: Box::new(left), op, right: Box::new(right), pos };
        }
        Ok(left)
    }

    fn parse_term(&mut self) -> Result<Expr, ParseError> {
        let mut left = self.parse_factor()?;
        loop {
            let op = match &self.peek().kind {
                TokenKind::Plus => BinaryOp::Add,
                TokenKind::Minus => BinaryOp::Sub,
                _ => break,
            };
            let pos = self.peek_pos();
            self.advance();
            let right = self.parse_factor()?;
            left = Expr::Binary { left: Box::new(left), op, right: Box::new(right), pos };
        }
        Ok(left)
    }

    fn parse_factor(&mut self) -> Result<Expr, ParseError> {
        let mut left = self.parse_unary()?;
        loop {
            let op = match &self.peek().kind {
                TokenKind::Star => BinaryOp::Mul,
                TokenKind::Slash => BinaryOp::Div,
                TokenKind::Percent => BinaryOp::Mod,
                _ => break,
            };
            let pos = self.peek_pos();
            self.advance();
            let right = self.parse_unary()?;
            left = Expr::Binary { left: Box::new(left), op, right: Box::new(right), pos };
        }
        Ok(left)
    }

    fn parse_unary(&mut self) -> Result<Expr, ParseError> {
        let pos = self.peek_pos();
        match &self.peek().kind {
            TokenKind::Minus => {
                self.advance();
                let expr = self.parse_unary()?;
                Ok(Expr::Unary { op: UnaryOp::Neg, expr: Box::new(expr), pos })
            }
            TokenKind::Not => {
                self.advance();
                let expr = self.parse_unary()?;
                Ok(Expr::Unary { op: UnaryOp::Not, expr: Box::new(expr), pos })
            }
            _ => self.parse_power(),
        }
    }

    /// `**` binds tighter than `* / %` (standard math convention) and,
    /// unlike them, is RIGHT-associative: `2 ** 3 ** 2` parses as
    /// `2 ** (3 ** 2)` = 512, not `(2 ** 3) ** 2` = 64 — matching every
    /// other language that has `**` (Python, JS, Ruby...). That's why
    /// this recurses back into itself on the right-hand side instead of
    /// looping like parse_factor does for its left-associative operators.
    fn parse_power(&mut self) -> Result<Expr, ParseError> {
        let base = self.parse_call()?;
        if self.check(&TokenKind::StarStar) {
            let pos = self.peek_pos();
            self.advance();
            let exponent = self.parse_power()?; // right-associative recursion
            return Ok(Expr::Binary { left: Box::new(base), op: BinaryOp::Pow, right: Box::new(exponent), pos });
        }
        Ok(base)
    }

    fn parse_call(&mut self) -> Result<Expr, ParseError> {
        let mut expr = self.parse_primary()?;

        loop {
            if self.check(&TokenKind::LParen) {
                let pos = self.peek_pos();
                self.advance(); // (

                let mut args = Vec::new();
                if !self.check(&TokenKind::RParen) {
                    args.push(self.parse_expr()?);
                    while self.match_kind(&TokenKind::Comma) {
                        args.push(self.parse_expr()?);
                    }
                }
                self.expect(TokenKind::RParen, "')'")?;

                expr = Expr::Call { callee: Box::new(expr), args, pos };
            } else if self.check(&TokenKind::LBracket) {
                let pos = self.peek_pos();
                self.advance(); // [
                let index = self.parse_expr()?;
                self.expect(TokenKind::RBracket, "']'")?;

                expr = Expr::Index { array: Box::new(expr), index: Box::new(index), pos };
            } else if self.check(&TokenKind::Dot) {
                let pos = self.peek_pos();
                self.advance(); // .
                // `x.string()`, `x.int()`, `x.float()`: the type keywords are also
                // method names after a dot (the other names are not affected)
                let field = match self.peek().kind {
                    TokenKind::TypeInt | TokenKind::TypeFloat | TokenKind::TypeString | TokenKind::TypeBool => {
                        let name = self.parse_member_name()?;
                        name
                    }
                    _ => self.parse_ident_name()?,
                };

                expr = Expr::Field { object: Box::new(expr), field, pos };
            } else {
                break;
            }
        }

        Ok(expr)
    }

    /// parses `{ field: value, ... }` for a struct literal whose type
    /// name and position were already consumed by `parse_primary`.
    /// Struct literals are allowed again inside the field values even if
    /// the literal itself was reached with `no_struct_literal` cleared
    /// by an enclosing call — once we've committed to `Name {`, we're no
    /// longer in the ambiguous position, so nested field values (e.g. a
    /// field that is itself a struct literal) parse under the ordinary
    /// rule.
    fn parse_struct_lit(&mut self, name: String, pos: Pos) -> Result<Expr, ParseError> {
        self.advance(); // {
        let prev = self.no_struct_literal;
        self.no_struct_literal = false;

        let mut fields = Vec::new();
        if !self.check(&TokenKind::RBrace) {
            fields.push(self.parse_struct_field()?);
            while self.match_kind(&TokenKind::Comma) {
                if self.check(&TokenKind::RBrace) {
                    break; // trailing comma
                }
                fields.push(self.parse_struct_field()?);
            }
        }
        self.expect(TokenKind::RBrace, "'}'")?;

        self.no_struct_literal = prev;
        Ok(Expr::StructLit { name, fields, pos })
    }

    fn parse_struct_field(&mut self) -> Result<(String, Expr), ParseError> {
        let field_name = self.parse_ident_name()?;
        self.expect(TokenKind::Colon, "':'")?;
        let value = self.parse_expr()?;
        Ok((field_name, value))
    }

    /// `@ns.member` → a single qualified identifier `"@ns.member"`. The
    /// postfix loop then turns `@ns.member(args)` into an ordinary call, so
    /// the typechecker/compiler/VM see namespaces as just a naming scheme
    /// for builtins (no runtime namespace value exists).
    fn parse_namespace_access(&mut self, ns: String, pos: Pos) -> Result<Expr, ParseError> {
        if !self.check(&TokenKind::Dot) {
            return Err(ParseError::NamespaceWithoutMember { name: ns, line: pos.line, col: pos.col });
        }
        self.advance(); // .
        let member = self.parse_member_name()?;
        Ok(Expr::Ident(format!("@{}.{}", ns, member), pos))
    }

    fn parse_primary(&mut self) -> Result<Expr, ParseError> {
        let pos = self.peek_pos();
        match self.advance().kind {
            TokenKind::Int(n) => Ok(Expr::IntLit(n, pos)),
            TokenKind::Float(f) => Ok(Expr::FloatLit(f, pos)),
            TokenKind::Str(s) => Ok(Expr::StringLit(s, pos)),
            TokenKind::Bool(b) => Ok(Expr::BoolLit(b, pos)),
            TokenKind::NoneLit => Ok(Expr::NoneLit(pos)),
            TokenKind::Namespace(name) => self.parse_namespace_access(name, pos),
            TokenKind::Function => {
                self.expect(TokenKind::LParen, "'(' (anonymous function)")?;
                let mut params = Vec::new();
                if !self.check(&TokenKind::RParen) {
                    params.push(self.parse_param()?);
                    while self.match_kind(&TokenKind::Comma) {
                        params.push(self.parse_param()?);
                    }
                }
                self.expect(TokenKind::RParen, "')'")?;
                let return_type = if self.match_kind(&TokenKind::Arrow) { Some(self.parse_type_expr()?) } else { None };
                let body = self.parse_block()?;
                Ok(Expr::Lambda { params, return_type, body, pos })
            }
            TokenKind::TypeString if self.check(&TokenKind::Dot) => {
                self.advance(); // .
                let member = match &self.peek().kind {
                    TokenKind::Ident(m) => m.clone(),
                    _ => String::new(),
                };
                Err(ParseError::OldStringNamespace { member, line: pos.line, col: pos.col })
            }
            TokenKind::Ident(name) => {
                // `Name {` starts a struct literal, unless we're inside
                // an if/while condition or a for-in iterable — there,
                // the `{` almost always belongs to the following block,
                // not to a struct literal (see `no_struct_literal`'s
                // doc comment). Checking `self.check(&LBrace)` here
                // (rather than in the postfix loop) keeps this ambiguity
                // local to the one token sequence that causes it.
                if !self.no_struct_literal && self.check(&TokenKind::LBrace) {
                    self.parse_struct_lit(name, pos)
                } else {
                    Ok(Expr::Ident(name, pos))
                }
            }
            TokenKind::LParen => {
                let expr = self.parse_expr()?;
                self.expect(TokenKind::RParen, "')'")?;
                Ok(expr)
            }
            TokenKind::LBracket => {
                // `[:]` is the empty map
                if self.match_kind(&TokenKind::Colon) {
                    self.expect(TokenKind::RBracket, "']' (the empty map is written '[:]')")?;
                    return Ok(Expr::MapLit(Vec::new(), pos));
                }
                let mut items = Vec::new();
                if !self.check(&TokenKind::RBracket) {
                    items.push(self.parse_expr()?);
                    // a ':' after the first element makes it a map literal
                    if self.match_kind(&TokenKind::Colon) {
                        let mut entries = Vec::new();
                        let first_key = items.pop().expect("one element was just pushed");
                        entries.push((first_key, self.parse_expr()?));
                        while self.match_kind(&TokenKind::Comma) {
                            let key = self.parse_expr()?;
                            self.expect(TokenKind::Colon, "':' (map entries are 'key: value')")?;
                            entries.push((key, self.parse_expr()?));
                        }
                        self.expect(TokenKind::RBracket, "']'")?;
                        return Ok(Expr::MapLit(entries, pos));
                    }
                    while self.match_kind(&TokenKind::Comma) {
                        items.push(self.parse_expr()?);
                    }
                }
                self.expect(TokenKind::RBracket, "']'")?;
                Ok(Expr::ArrayLit(items, pos))
            }
            TokenKind::DotDot => Err(ParseError::RangeOutsideFor { line: pos.line, col: pos.col }),
            other => Err(ParseError::Unexpected {
                expected: "an expression (literal, identifier, '(' or 'if')".to_string(),
                found: other,
                line: pos.line,
                col: pos.col,
            }),
        }
    }
}

impl ParseError {
    /// the same fact in the friendly tone (`#> friendly-terminal`)
    pub fn friendly(&self) -> String {
        spg_lexer::friendly_sentence(&self.to_string(), spg_lexer::HINT_SYNTAX)
    }
}
