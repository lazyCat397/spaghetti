//! turns tokens into an AST. the big translator.
//!
//! classic recursive descent, with operator precedence handled by a
//! chain of functions (parse_or -> parse_and -> parse_equality -> ... ->
//! parse_primary). the staircase.
//!
//! special rule: `if` is parsed as `Expr::IfExpr` when it shows up in
//! expression position (e.g. the right side of a `let`). there, `else`
//! is REQUIRED, and its absence is a parse error, not something left for
//! the typechecker to catch later. no homework for the checker.

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
    /// name: variable, function, param, struct, field, type or import
    ReservedName { name: String, line: usize, col: usize },
    /// a namespace used without `.member` (e.g. `let x = math`). no.
    NamespaceWithoutMember { name: String, line: usize, col: usize },
    /// `a..b` anywhere but a `for .. in` header (`let r = 1..4`)
    RangeOutsideFor { line: usize, col: usize },
    /// `f(x: 1, 2)`: a positional argument after a named one
    PositionalAfterNamed { line: usize, col: usize },
    /// a struct with a field AND a method of the same name (`p.x` would be ambiguous)
    FieldMethodClash { struct_name: String, name: String, line: usize, col: usize },
    /// a `guard` rule the parser can see is broken (write to `me`, `return`, unknown field...)
    GuardError { struct_name: String, reason: String, line: usize, col: usize },
    /// `enum E { A, A }`. twice is too much.
    DuplicateVariant { enum_name: String, name: String, line: usize, col: usize },
    /// `enum E { }`: an enum needs at least one variant. nothing is not an enum.
    EmptyEnum { name: String, line: usize, col: usize },
    /// `enum E { A = 1, B = "x" }`: the variants of one enum share one value type
    EnumValueType { enum_name: String, name: String, found: &'static str, expected: &'static str, line: usize, col: usize },
    /// a variant with no `= value` where no number can be invented for it
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
            ParseError::PositionalAfterNamed { line, col } => write!(
                f,
                "a positional argument cannot follow a named one; put it before the named arguments (line {}, column {})",
                line, col
            ),
            ParseError::GuardError { struct_name, reason, line, col } => write!(
                f,
                "guard of struct '{}': {} (line {}, column {})",
                struct_name, reason, line, col
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
    /// iterable. in those positions, `Ident {` is the following
    /// block's brace, not the start of a struct literal. mirrors the
    /// same ambiguity (and same fix) Rust has with struct
    /// literals in condition position. only `parse_primary`'s struct
    /// literal branch reads this. every other expression parses
    /// identically regardless. fair.
    no_struct_literal: bool,
    /// the methods of the `struct` just parsed, already lowered to plain
    /// `FunctionDecl`s (`P.f`, with `me: P` as the first param). a
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
        loop {
            self.skip_semicolons();
            if self.check(&TokenKind::Eof) {
                break;
            }
            if self.check(&TokenKind::Public) && self.method_add_at(1) {
                self.advance(); // export
                stmts.push(self.parse_method_add(true)?);
                continue;
            }
            if self.at_method_add() {
                stmts.push(self.parse_method_add(false)?);
                continue;
            }
            stmts.push(self.parse_stmt()?);
            stmts.append(&mut self.pending);
        }
        Ok(Program { stmts, natives: Vec::new() })
    }

    // navigation helpers (lost, but found)

    fn peek(&self) -> &Token {
        &self.tokens[self.pos]
    }

    /// whether the token after the current one is `kind` (one token of lookahead)
    fn next_is(&self, kind: &TokenKind) -> bool {
        self.tokens.get(self.pos + 1).is_some_and(|t| std::mem::discriminant(&t.kind) == std::mem::discriminant(kind))
    }

    /// whether the current token sits on a later line than the one before it.
    /// a line break alone never ends a statement, so this is only used where
    /// the next token could also start a new statement: `(`, `[`, `-`.
    fn starts_new_line(&self) -> bool {
        self.pos > 0 && self.peek().line > self.tokens[self.pos - 1].line
    }

    fn peek_pos(&self) -> Pos {
        let t = self.peek();
        Pos { line: t.line, col: t.col, len: t.len, focus: 0, file: None }
    }

    fn check(&self, kind: &TokenKind) -> bool {
        std::mem::discriminant(&self.peek().kind) == std::mem::discriminant(kind)
    }

    /// the span of a compound expression (binary, call, field access): from the start of the
    /// expression on its left (`left`) to the end of the last token consumed, with the caret on
    /// `focus` (the operator, the `(` of a call, the `.` of a field).
    fn expr_span(&self, left: &Expr, focus_pos: Pos) -> Pos {
        let op = focus_pos;
        let start = left.pos().cloned().unwrap_or_else(|| op.clone());
        let last = &self.tokens[self.pos.saturating_sub(1)];
        let len = if last.line == start.line && last.col + last.len > start.col {
            last.col + last.len - start.col
        } else {
            op.len
        };
        let focus = if op.line == start.line { op.col.saturating_sub(start.col) } else { 0 };
        Pos { line: start.line, col: start.col, len, focus, file: start.file.clone() }
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

    /// `;` is an optional statement separator (like JavaScript). any run of
    /// them, including none, may sit between statements. semicolons, optional, like life.
    fn skip_semicolons(&mut self) {
        while self.check(&TokenKind::Semicolon) {
            self.advance();
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

    // statements (the parade)

    fn parse_stmt(&mut self) -> Result<Stmt, ParseError> {
        match &self.peek().kind {
            // `int::methods.add(...)` only at the top of the file: the list of methods
            // must be known to the checker before any code runs
            _ if self.at_method_add() => {
                let pos = self.peek_pos();
                Err(ParseError::Unexpected {
                    expected: "int::methods.add(...) only at the top level of a file (not inside a function or block)".to_string(),
                    found: self.peek().kind.clone(),
                    line: pos.line,
                    col: pos.col,
                })
            }
            TokenKind::Let => self.parse_let(false, false),
            TokenKind::Const => self.parse_let(true, false),
            TokenKind::If => self.parse_if_stmt(),
            TokenKind::When => self.parse_match_stmt(),
            TokenKind::While => self.parse_while(),
            TokenKind::Do => self.parse_do_while(),
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
            // `fn (` at statement start is an anonymous function used as an
            // expression statement (`fn() { ... }()`), not a declaration
            TokenKind::Function if !self.next_is(&TokenKind::LParen) => self.parse_function_decl(false),
            TokenKind::Struct => self.parse_struct_decl(false),
            TokenKind::NamespaceKw => self.parse_namespace(false),
            TokenKind::Enum => self.parse_enum_decl(false),
            TokenKind::Public => self.parse_public(),
            TokenKind::Use => self.parse_use(),
            TokenKind::From => self.parse_from_use(),
            TokenKind::Attach => self.parse_attach(),
            // `x = ...`, `arr[i] = ...`, and their compound forms
            // (`x += ...`, `arr[i] -= ...`) all start as an ordinary
            // expression (Ident, or Ident followed by one or more
            // postfix `[...]`/`(...)`). parse that expression once,
            // then decide from its shape plus the trailing operator
            // which statement this actually is. parsing the target
            // expression exactly once matters here: `arr[f()] += 1`
            // must call `f()` once, not once per desugared
            // occurrence of the index. sneaky side effects, banned.
            _ => {
                let expr = self.parse_expr()?;

                if self.check(&TokenKind::Eq) {
                    let pos = self.peek_pos();
                    self.advance(); // =
                    let value = self.parse_expr()?;
                    return self.assign_stmt_from_target(expr, value, pos);
                }

                // `x++;` / `arr[i]--;` as a statement: the value is thrown away, so
                // it's the prefix form (no extra read). the checker picks 1 or 1.0
                // from the target's type, so the parser can't do it
                if let Expr::IncDec { target, delta, pos, .. } = expr {
                    return Ok(Stmt::ExprStmt(Expr::IncDec { target, delta, prefix: true, pos }));
                }

                if let Some(op) = self.peek_compound_op() {
                    let pos = self.peek_pos();
                    self.advance(); // +=/-=/*=//=/%=
                    let rhs = self.parse_expr()?;
                    return match expr {
                        // plain `x OP= rhs` desugars to `x = x OP rhs`
                        // right here. rereading a local has no side
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
                        // instead. see IndexCompoundAssign's doc comment
                        // for why desugaring here would be wrong
                        Expr::Index { array, index, .. } => {
                            Ok(Stmt::IndexCompoundAssign { array: *array, index: *index, op, value: rhs, pos })
                        }
                        // `obj.field OP= rhs`: same reasoning as the
                        // index case (object is evaluated once). see
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
    /// operators (+=, -=, *=, /=, %=), mapped to the `BinaryOp` it desugars
    /// to.
    fn peek_compound_op(&self) -> Option<BinaryOp> {
        match &self.peek().kind {
            TokenKind::PlusEq => Some(BinaryOp::Add),
            TokenKind::MinusEq => Some(BinaryOp::Sub),
            TokenKind::StarEq => Some(BinaryOp::Mul),
            TokenKind::SlashEq => Some(BinaryOp::Div),
            TokenKind::PercentEq => Some(BinaryOp::Mod),
            _ => None,
        }
    }

    /// builds the right `Stmt::Assign`/`Stmt::IndexAssign` from an
    /// already-parsed target expression. shared by plain `=` and every
    /// desugared compound-assignment operator, so both paths reject the
    /// same set of non-assignable targets the same way. consistency!
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

    /// destructuring `let`, lowered right here to plain lets so the checker, the VM and the
    /// JIT never see a new form. the value is read once into a hidden name (`$let_LINE_COL`,
    /// unique per position, so two lets never clash); each binding then reads its own part:
    /// `(a, b)` reads `$let[0]`, `$let[1]`; `{ x, y }` reads `$let.x`, `$let.y`.
    /// the hidden name is never exported; the bindings are, when the `let` is.
    fn parse_destructuring_let(&mut self, exported: bool, pos: Pos) -> Result<Stmt, ParseError> {
        let is_array = self.check(&TokenKind::LParen);
        let close = if is_array { TokenKind::RParen } else { TokenKind::RBrace };
        self.advance(); // ( or {

        let mut names = Vec::new();
        while !self.check(&close) {
            names.push(self.parse_binding_name()?);
            if !self.match_kind(&TokenKind::Comma) {
                break;
            }
        }
        let found = self.peek().kind.clone();
        let (line, col) = (self.peek().line, self.peek().col);
        if names.is_empty() {
            return Err(ParseError::Unexpected {
                expected: "at least one name to bind".to_string(),
                found,
                line,
                col,
            });
        }
        self.expect(close, if is_array { "')'" } else { "'}'" })?;
        self.expect(TokenKind::Eq, "'='")?;
        let value = self.parse_expr()?;

        let hidden = format!("$let_{}_{}", pos.line, pos.col);
        for (i, name) in names.into_iter().enumerate() {
            let object = Box::new(Expr::Ident(hidden.clone(), pos.clone()));
            let part = if is_array {
                Expr::Index { array: object, index: Box::new(Expr::IntLit(i as i64, pos.clone())), pos: pos.clone() }
            } else {
                Expr::Field { object, field: name.clone(), pos: pos.clone() }
            };
            self.pending.push(Stmt::Let { name, type_ann: None, value: part, exported, pos: pos.clone() });
        }
        Ok(Stmt::Let { name: hidden, type_ann: None, value, exported: false, pos })
    }

    fn parse_let(&mut self, is_const: bool, exported: bool) -> Result<Stmt, ParseError> {
        let pos = self.peek_pos();
        self.advance(); // let/const

        // `let (a, b) = arr;` (array) and `let { x, y } = point;` (struct fields)
        if self.check(&TokenKind::LParen) || self.check(&TokenKind::LBrace) {
            if is_const {
                let found = self.peek().kind.clone();
                return Err(ParseError::Unexpected {
                    expected: "a name after 'const' (destructuring works with 'let' only)".to_string(),
                    found,
                    line: pos.line,
                    col: pos.col,
                });
            }
            return self.parse_destructuring_let(exported, pos);
        }

        let name = self.parse_binding_name()?;

        let type_ann = if self.match_kind(&TokenKind::Colon) {
            Some(self.parse_type_expr()?)
        } else {
            None
        };

        // `let x;` / `let x: T;` (no '=') takes the zero value of its type.
        // const always needs its value: a frozen nothing is useless.
        let value = if !is_const && !self.check(&TokenKind::Eq) {
            self.zero_value(type_ann.as_ref(), &pos)?
        } else {
            self.expect(TokenKind::Eq, "'='")?;
            self.parse_expr()?
        };

        if is_const {
            Ok(Stmt::Const { name, type_ann, value, exported, pos })
        } else {
            Ok(Stmt::Let { name, type_ann, value, exported, pos })
        }
    }

    /// the zero value a `let` without `=` starts with. no annotation means
    /// int 0. struct, enum, union and function types have no zero, so they
    /// refuse: a struct is an empty shell, nothing to default to.
    fn zero_value(&self, type_ann: Option<&TypeExpr>, pos: &Pos) -> Result<Expr, ParseError> {
        let p = pos.clone();
        let zero = match type_ann {
            None | Some(TypeExpr::Int) | Some(TypeExpr::I8) | Some(TypeExpr::I16) | Some(TypeExpr::I32) | Some(TypeExpr::U8) | Some(TypeExpr::U16) | Some(TypeExpr::U32) | Some(TypeExpr::U64) => Some(Expr::IntLit(0, p.clone())),
            Some(TypeExpr::Float) | Some(TypeExpr::F32) => Some(Expr::FloatLit(0.0, p.clone())),
            Some(TypeExpr::Bool) => Some(Expr::BoolLit(false, p.clone())),
            Some(TypeExpr::String) => Some(Expr::StringLit(String::new(), p.clone())),
            Some(TypeExpr::Optional(_)) => Some(Expr::NoneLit(p.clone())),
            Some(TypeExpr::Array(_)) => Some(Expr::ArrayLit(Vec::new(), p.clone())),
            Some(TypeExpr::Map(_, _)) => Some(Expr::MapLit(Vec::new(), p.clone())),
            Some(_) => None,
        };
        zero.ok_or_else(|| ParseError::Unexpected {
            expected: "'=' with a value (only int, float, bool, string, arrays, maps and optionals default to a zero value)".to_string(),
            found: self.peek().kind.clone(),
            line: pos.line,
            col: pos.col,
        })
    }

    /// `if (cond) { } elif (cond2) { } elif (cond3) { } else { }`. `elif`
    /// is the else-if chain keyword (`when` is now the match statement).
    /// structurally there's no new AST node. each `when` just becomes a
    /// nested `IfStmt` living inside the previous branch's `else_branch`
    /// (the same trick "else if" chains use under the hood in most
    /// languages). so spg-typecheck and spg-compiler need zero changes.
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

    /// `when (subject) { "a", "b" { } 3 { } else { } }`: a match. each arm lists
    /// one or more values (comma separated) and a body. an optional `else`
    /// block closes it. exhaustiveness and value types get checked later.
    fn parse_match_stmt(&mut self) -> Result<Stmt, ParseError> {
        let pos = self.peek_pos();
        self.advance(); // when
        self.expect(TokenKind::LParen, "'('")?;
        let subject = self.parse_expr_no_struct_literal()?;
        self.expect(TokenKind::RParen, "')'")?;
        self.expect(TokenKind::LBrace, "'{'")?;

        let mut arms = Vec::new();
        let mut else_branch = None;
        while !self.check(&TokenKind::RBrace) {
            if self.match_kind(&TokenKind::Else) {
                else_branch = Some(self.parse_block()?);
                continue;
            }
            let arm_pos = self.peek_pos();
            let mut values = vec![self.parse_expr_no_struct_literal()?];
            while self.match_kind(&TokenKind::Comma) {
                values.push(self.parse_expr_no_struct_literal()?);
            }
            let body = self.parse_block()?;
            arms.push(MatchArm { values, body, pos: arm_pos });
        }
        self.expect(TokenKind::RBrace, "'}'")?;

        Ok(Stmt::Match { subject, arms, else_branch, pos })
    }

    /// parses whatever comes after an `if`'s (or a previous `elif`'s)
    /// body: another `elif (cond) { }`, which recurses, wrapping itself
    /// in a one-statement Block so it fits `else_branch: Option<Block>`
    /// unchanged and keeps the chain going. or a trailing `else { }`,
    /// or neither. the choose-your-own-adventure ending.
    fn parse_when_or_else_tail(&mut self) -> Result<Option<Block>, ParseError> {
        if self.check(&TokenKind::Elif) {
            let when_pos = self.peek_pos();
            self.advance(); // elif
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

    fn parse_do_while(&mut self) -> Result<Stmt, ParseError> {
        let pos = self.peek_pos();
        self.advance(); // do
        let body = self.parse_block()?;
        self.expect(TokenKind::While, "'while'")?;
        self.expect(TokenKind::LParen, "'('")?;
        let cond = self.parse_expr()?;
        self.expect(TokenKind::RParen, "')'")?;
        Ok(Stmt::DoWhile { body, cond, pos })
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
        self.expect(TokenKind::LParen, "'(' after 'for'")?;

        // `for (x in arr)` or `for (i, x in arr)`: with two names, the first is the
        // position (or the key of a map) and the second the element
        let first = self.parse_binding_name()?;
        let (index_var, var_name) = if self.match_kind(&TokenKind::Comma) {
            let second = self.parse_binding_name()?;
            (Some(first), second)
        } else {
            (None, first)
        };
        self.expect(TokenKind::In, "'in'")?;
        let iterable = self.parse_expr_no_struct_literal()?;

        // `a..b` / `a..b step s`: the first expression was the START
        if self.match_kind(&TokenKind::DotDot) {
            if index_var.is_some() {
                return Err(ParseError::Unexpected {
                    expected: "a single loop variable for a range (`for i in a..b`); the `i, v` form is for arrays".to_string(),
                    found: self.peek().kind.clone(),
                    line: pos.line,
                    col: pos.col,
                });
            }
            let end = self.parse_expr_no_struct_literal()?;
            let step = if matches!(&self.peek().kind, TokenKind::Ident(w) if w == "step") {
                self.advance(); // step
                Some(self.parse_expr_no_struct_literal()?)
            } else {
                None
            };
            self.expect(TokenKind::RParen, "')' to close the 'for' header")?;
            let body = self.parse_block()?;
            return Ok(Stmt::ForRange { var_name, start: iterable, end, step, body, pos });
        }

        self.expect(TokenKind::RParen, "')' to close the 'for' header")?;
        let body = self.parse_block()?;

        Ok(Stmt::ForIn { var_name, index_var, iterable, body, pos })
    }

    fn parse_return(&mut self) -> Result<Stmt, ParseError> {
        let pos = self.peek_pos();
        self.advance(); // return

        // no value: the next token is '}', ';' or the end of the file
        let value = if self.check(&TokenKind::RBrace)
            || self.check(&TokenKind::Semicolon)
            || self.check(&TokenKind::Eof)
            || self.starts_new_line()
        {
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

    fn parse_public(&mut self) -> Result<Stmt, ParseError> {
        let export_pos = self.peek_pos();
        self.advance(); // export
        match &self.peek().kind {
            TokenKind::Function => self.parse_function_decl(true),
            TokenKind::Struct => self.parse_struct_decl(true),
            TokenKind::NamespaceKw => self.parse_namespace(true),
            TokenKind::Enum => self.parse_enum_decl(true),
            TokenKind::Let => self.parse_let(false, true),
            TokenKind::Const => self.parse_let(true, true),
            // `public (a, b) from "x"`
            TokenKind::LParen => self.parse_re_export(export_pos),
            _ => {
                let t = self.peek();
                Err(ParseError::Unexpected {
                    expected: "'function', 'struct', 'enum', 'let', 'const' or '(' after 'public'".to_string(),
                    found: t.kind.clone(),
                    line: t.line,
                    col: t.col,
                })
            }
        }
    }

    fn parse_use(&mut self) -> Result<Stmt, ParseError> {
        let pos = self.peek_pos();
        self.advance(); // import
        let (names, from) = self.parse_names_from()?;
        Ok(Stmt::Use { names, from, pos })
    }

    /// `from "./x.spg" use (a, b)`: the path comes first, then the names.
    /// same meaning as `use (a, b) from "./x.spg"`, the two forms are equivalent.
    fn parse_from_use(&mut self) -> Result<Stmt, ParseError> {
        let pos = self.peek_pos();
        self.advance(); // from

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

        self.expect(TokenKind::Use, "'use'")?;
        let names = self.parse_names()?;
        Ok(Stmt::Use { names, from, pos })
    }

    /// `attach "./file.spg"` (every export of the file, no name list) or `attach #http`
    /// (a standard library: native, or from the stdlib folder)
    fn parse_attach(&mut self) -> Result<Stmt, ParseError> {
        let pos = self.peek_pos();
        self.advance(); // install
        let from = match self.advance().kind {
            TokenKind::StdLib(name) => return Ok(Stmt::Attach { from: name, pos, std: true }),
            TokenKind::Str(s) => s,
            other => {
                return Err(ParseError::Unexpected {
                    expected: "a string with the module path, or `#name` for a standard library (`attach #http`)".to_string(),
                    found: other,
                    line: pos.line,
                    col: pos.col,
                })
            }
        };
        Ok(Stmt::Attach { from, pos, std: false })
    }

    /// `public (a, b) from "./y.spg"`. the `public` keyword is already
    /// consumed, the cursor sits on `{`.
    fn parse_re_export(&mut self, pos: Pos) -> Result<Stmt, ParseError> {
        let (names, from) = self.parse_names_from()?;
        Ok(Stmt::ReExport { names, from, pos })
    }

    /// the names of a `use` or `public` line: always in parentheses, `(a, b)`. `()` is an
    /// empty list. one form only, so a name list reads the same everywhere.
    fn parse_names(&mut self) -> Result<Vec<String>, ParseError> {
        self.expect(TokenKind::LParen, "'(' before the names (write `use (a, b) from \"x\"`)")?;
        let mut names = Vec::new();
        if self.match_kind(&TokenKind::RParen) {
            return Ok(names);
        }
        names.push(self.parse_binding_name()?);
        while self.match_kind(&TokenKind::Comma) {
            names.push(self.parse_binding_name()?);
        }
        self.expect(TokenKind::RParen, "')'")?;
        Ok(names)
    }

    /// the `names from "path"` tail that `use` and `public` have in common
    fn parse_names_from(&mut self) -> Result<(Vec<String>, String), ParseError> {
        let names = self.parse_names()?;
        self.expect(TokenKind::From, "'from'")?;

        let pos = self.peek_pos();
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

    /// `int::methods.add(` / `string::methods.add(` ...: the start of a method
    /// registration. `methods` is only a member of a value type here.
    fn at_method_add(&self) -> bool {
        self.method_add_at(0)
    }

    /// the same check, `off` tokens ahead of the current one (`public` sits in front)
    fn method_add_at(&self, off: usize) -> bool {
        let at = |k: usize| self.tokens.get(self.pos + off + k).map(|t| &t.kind);
        value_type_token(at(0)).is_some()
            && matches!(at(1), Some(TokenKind::DoubleColon))
            && matches!(at(2), Some(TokenKind::Ident(n)) if n == "methods")
            && matches!(at(3), Some(TokenKind::Dot))
    }

    /// `int::methods.add(fn dobro() -> int { return me * 2 })`, top level only.
    /// lowered to the plain function `int.dobro(me: int) -> int { .. }`, the same
    /// shape a struct method gets. so the checker, the VM and the modules see
    /// nothing new. `me` is implicit: it is always the first parameter, typed as the value.
    fn parse_method_add(&mut self, exported: bool) -> Result<Stmt, ParseError> {
        let pos = self.peek_pos();
        let (ty, type_ann) = match value_type_token(Some(&self.advance().kind)) {
            Some(found) => found,
            None => unreachable!("parse_method_add runs only after method_add_at matched a value type"),
        };
        self.advance(); // ::
        self.advance(); // methods
        self.advance(); // .
        match self.peek().kind.clone() {
            TokenKind::Ident(n) if n == "add" => { self.advance(); }
            _ => {
                return Err(ParseError::Unexpected {
                    expected: "'add' (int::methods.add(fn ...))".to_string(),
                    found: self.peek().kind.clone(),
                    line: pos.line,
                    col: pos.col,
                });
            }
        }
        self.expect(TokenKind::LParen, "'('")?;
        if !self.check(&TokenKind::Function) {
            return Err(ParseError::Unexpected {
                expected: "a named function: int::methods.add(fn nome(...) { ... }) (no anonymous functions here)".to_string(),
                found: self.peek().kind.clone(),
                line: pos.line,
                col: pos.col,
            });
        }
        let decl = self.parse_function_decl(exported)?;
        self.expect(TokenKind::RParen, "')'")?;
        match decl {
            Stmt::FunctionDecl { name, mut params, return_type, body, exported, pos: fpos } => {
                params.insert(0, Param { name: "me".to_string(), type_ann, default: None });
                Ok(Stmt::FunctionDecl {
                    name: format!("{}.{}", ty, name),
                    params,
                    return_type,
                    body,
                    exported,
                    pos: fpos,
                })
            }
            _ => unreachable!("parse_function_decl always returns a FunctionDecl"),
        }
    }

    fn parse_function_decl(&mut self, exported: bool) -> Result<Stmt, ParseError> {
        let pos = self.peek_pos();
        self.advance(); // function

        // `int::methods.add(fn name(...))` is lowered to `int.name`: a method on a value type. the type
        // keyword is the name here, and the dot after it is required
        let mut name = match self.peek().kind.clone() {
            TokenKind::TypeInt => { self.advance(); "int".to_string() }
            TokenKind::TypeFloat => { self.advance(); "float".to_string() }
            TokenKind::TypeF32 => { self.advance(); "f32".to_string() }
            TokenKind::TypeBool => { self.advance(); "bool".to_string() }
            TokenKind::TypeString => { self.advance(); "string".to_string() }
            _ => self.parse_binding_name()?,
        };
        if crate::ast::is_value_type_name(&name) {
            return Err(ParseError::Unexpected {
                expected: "a method on a value type is declared as int::methods.add(fn nome(...) { ... })".to_string(),
                found: self.peek().kind.clone(),
                line: pos.line,
                col: pos.col,
            });
        }
        // `function Type.name(...)`: an associated function (no `me`). the name keeps the
        // dot, so the typechecker and the VM see it as `Type.name`, like a method
        if self.match_kind(&TokenKind::Dot) {
            let member = self.parse_binding_name()?;
            name = format!("{}.{}", name, member);
        }
        self.expect(TokenKind::LParen, "'('")?;

        let mut params = Vec::new();
        if !self.check(&TokenKind::RParen) {
            params.push(self.parse_param(true)?);
            while self.match_kind(&TokenKind::Comma) {
                params.push(self.parse_param(true)?);
            }
        }
        self.expect(TokenKind::RParen, "')'")?;
        // an associated function has no `me`. with one it would be a method declared
        // outside its struct, which is not how methods are written
        if name.contains('.') && params.first().map_or(false, |p| p.name == "me") {
            return Err(ParseError::Unexpected {
                expected: "no `me` parameter here (methods go inside the struct; associated functions have none)".to_string(),
                found: TokenKind::RParen,
                line: pos.line,
                col: pos.col,
            });
        }
        // a param with a default may only be followed by others that have one,
        // otherwise a call couldn't tell which arg was left out
        if let Some(gap) = params.iter().position(|p| p.default.is_some()) {
            if let Some(bad) = params[gap..].iter().find(|p| p.default.is_none()) {
                return Err(ParseError::Unexpected {
                    expected: format!("a default value for parameter '{}' (parameters after one with a default need one too)", bad.name),
                    found: self.peek().kind.clone(),
                    line: self.peek_pos().line,
                    col: self.peek_pos().col,
                });
            }
        }

        // trailing return type is optional: `-> Type` present keeps the
        // old explicit behavior. absent, the typechecker infers it from
        // the body's return statements instead
        let return_type = if self.match_kind(&TokenKind::Arrow) {
            Some(self.parse_type_expr()?)
        } else {
            None
        };

        let body = self.parse_block()?;

        Ok(Stmt::FunctionDecl { name, params, return_type, body, exported, pos })
    }

    /// `namespace som { let calls; fn add(...) { } }`: a struct with no instances.
    /// the members become `som.calls` and `som.add`, so the rest of the pipeline
    /// reads them the same way it already reads `fn Struct.name` and `Struct.field`.
    /// statics go in first, so the functions that read them come after.
    fn parse_namespace(&mut self, exported: bool) -> Result<Stmt, ParseError> {
        let pos = self.peek_pos();
        self.advance(); // namespace

        let name = self.parse_binding_name()?;
        self.expect(TokenKind::LBrace, "'{'")?;

        let mut statics: Vec<Stmt> = Vec::new();
        let mut functions: Vec<Stmt> = Vec::new();
        while !self.check(&TokenKind::RBrace) && !self.check(&TokenKind::Eof) {
            // `public` is per member: `public namespace` exports the type, and only the members marked
            // `public` leave the file. the others are still visible to the rest of this file
            let member_exported = self.match_kind(&TokenKind::Public);
            match self.peek().kind.clone() {
                TokenKind::Function => {
                    let f = self.parse_function_decl(member_exported)?;
                    functions.push(qualify_member(&name, f));
                }
                TokenKind::Let | TokenKind::Const => {
                    let let_pos = self.peek_pos();
                    let is_const = self.check(&TokenKind::Const);
                    let before = self.pending.len();
                    let s = self.parse_let(is_const, member_exported)?;
                    let plain = matches!(s, Stmt::Let { .. } | Stmt::Const { .. });
                    if self.pending.len() != before || !plain {
                        return Err(ParseError::Unexpected {
                            expected: "a single name inside a namespace (no destructuring)".to_string(),
                            found: TokenKind::Let,
                            line: let_pos.line,
                            col: let_pos.col,
                        });
                    }
                    self.match_kind(&TokenKind::Semicolon); // `let calls;` ends with a `;` like any statement
                    statics.push(qualify_member(&name, s));
                }
                _ => {
                    let t = self.peek();
                    return Err(ParseError::Unexpected {
                        expected: "'fn', 'let' or 'const' inside a namespace (optionally after 'public')".to_string(),
                        found: t.kind.clone(),
                        line: t.line,
                        col: t.col,
                    });
                }
            }
        }
        self.expect(TokenKind::RBrace, "'}'")?;

        self.pending.extend(statics);
        self.pending.extend(functions);
        // no fields and no `me`: nothing is ever built from it
        Ok(Stmt::StructDecl { name, fields: Vec::new(), exported, namespace: true, guarded: false, snapshot: None, pos })
    }

    fn parse_struct_decl(&mut self, exported: bool) -> Result<Stmt, ParseError> {
        let pos = self.peek_pos();
        self.advance(); // struct

        let name = self.parse_binding_name()?;
        self.expect(TokenKind::LBrace, "'{'")?;

        // the body mixes fields (`x: int`) and methods (`function f() { }`).
        // two fields need a comma between them (a trailing comma is fine).
        // a method needs no separator on either side. methods are free-spirited.
        let mut fields: Vec<Param> = Vec::new();
        let mut methods: Vec<Stmt> = Vec::new();
        let mut guards: Vec<GuardSpec> = Vec::new();
        while !self.check(&TokenKind::RBrace) && !self.check(&TokenKind::Eof) {
            if self.check(&TokenKind::Function) {
                methods.push(self.parse_method(&name, exported)?);
                self.match_kind(&TokenKind::Comma);
            } else if self.check(&TokenKind::Guard) {
                guards.push(self.parse_guard()?);
                self.match_kind(&TokenKind::Comma);
            } else {
                fields.push(self.parse_param(false)?);
                if !self.match_kind(&TokenKind::Comma)
                    && !self.check(&TokenKind::RBrace)
                    && !self.check(&TokenKind::Function)
                    && !self.check(&TokenKind::Guard)
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
        let guarded = !guards.is_empty();
        let mut snapshot = None;
        if guarded {
            crate::guard::check_guards(&name, &fields, &guards)?;
            snapshot = crate::guard::snapshot_fields(&guards);
            self.pending.extend(crate::guard::guard_functions(&name, exported, &guards, &pos));
        }
        self.pending.extend(methods);

        Ok(Stmt::StructDecl { name, fields, exported, namespace: false, guarded, snapshot, pos })
    }

    /// `guard (me.field as alias) { ... }` or `guard { ... }`, inside a struct body.
    /// the alias is optional: `me.field` works without it.
    fn parse_guard(&mut self) -> Result<GuardSpec, ParseError> {
        let pos = self.peek_pos();
        self.advance(); // guard

        // `guard (me.a as x, me.b)`: each entry names a field, with an optional alias
        let mut target = Vec::new();
        if self.match_kind(&TokenKind::LParen) {
            loop {
                let (line, col) = (self.peek().line, self.peek().col);
                let me = self.parse_ident_name()?;
                if me != "me" {
                    return Err(ParseError::Unexpected {
                        expected: "'me' (a field guard starts with me.field)".to_string(),
                        found: TokenKind::Ident(me),
                        line,
                        col,
                    });
                }
                self.expect(TokenKind::Dot, "'.'")?;
                let field = self.parse_ident_name()?;
                let alias = if matches!(&self.peek().kind, TokenKind::Ident(s) if s == "as") {
                    self.advance();
                    Some(self.parse_ident_name()?)
                } else {
                    None
                };
                target.push((field, alias));
                if !self.match_kind(&TokenKind::Comma) {
                    break;
                }
            }
            self.expect(TokenKind::RParen, "')'")?;
        }

        let body = self.parse_block()?;
        Ok(GuardSpec { target, body, pos })
    }

    /// `enum Color { Red, Green, Blue }`: variants are plain names separated
    /// by commas (a trailing comma is fine). a variant may carry a literal value
    /// (`Ok = 200`, `Name = "x"`, `On = true`, `Half = 0.5`, negative numbers too).
    /// without any `=` the variants are the ints 0, 1, 2...; after an int value the
    /// next variant without `=` is that value + 1. repeating a VALUE is fine,
    /// repeating a NAME is not. names are sacred.
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
    /// plain function `P.f(me: P, a: int) -> T { .. }`. the `.` can never appear
    /// in a name the user writes, so these functions can't clash with anything,
    /// and the module renamer mangles `P` and `P.f` to `<tag>::P` / `<tag>::P.f`
    /// (the typechecker relies on exactly that to find a method from its struct).
    fn parse_method(&mut self, struct_name: &str, exported: bool) -> Result<Stmt, ParseError> {
        match self.parse_function_decl(exported)? {
            Stmt::FunctionDecl { name, .. } if name.contains('.') => {
                let pos = self.peek_pos();
                Err(ParseError::Unexpected {
                    expected: "a method name inside the struct (associated functions go outside: fn Type.name())".to_string(),
                    found: TokenKind::Dot,
                    line: pos.line,
                    col: pos.col,
                })
            }
            Stmt::FunctionDecl { name, mut params, return_type, body, exported, pos } => {
                params.insert(0, Param { name: "me".to_string(), type_ann: TypeExpr::Named(struct_name.to_string()), default: None });
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

    /// param or struct field. since namespaces now carry an `@`, both
    /// are plain identifiers (`math`, `time`, ... are ordinary names again).
    fn parse_param(&mut self, allow_default: bool) -> Result<Param, ParseError> {
        let name = self.parse_binding_name()?;
        self.expect(TokenKind::Colon, "':' (type required in parameter/field)")?;
        let type_ann = self.parse_type_expr()?;
        let default = if self.check(&TokenKind::Eq) {
            let eq_pos = self.peek_pos();
            self.advance();
            if !allow_default {
                return Err(ParseError::Unexpected {
                    expected: "no default value here (defaults are only for named function parameters)".to_string(),
                    found: TokenKind::Eq,
                    line: eq_pos.line,
                    col: eq_pos.col,
                });
            }
            Some(self.parse_default_literal()?)
        } else {
            None
        };
        Ok(Param { name, type_ann, default })
    }

    /// the value of `n: int = <value>`: an int, float, string or bool literal,
    /// optionally with a leading `-` on a number
    fn parse_default_literal(&mut self) -> Result<Expr, ParseError> {
        let negative = self.match_kind(&TokenKind::Minus);
        let pos = self.peek_pos();
        let lit = self.parse_primary()?;
        match (negative, lit) {
            (false, lit @ (Expr::IntLit(..) | Expr::FloatLit(..) | Expr::StringLit(..) | Expr::BoolLit(..))) => Ok(lit),
            (true, Expr::IntLit(n, p)) => Ok(Expr::IntLit(-n, p)),
            (true, Expr::FloatLit(f, p)) => Ok(Expr::FloatLit(-f, p)),
            _ => Err(ParseError::Unexpected {
                expected: "a literal default value (a number, string or bool, like `= 10`)".to_string(),
                found: self.peek().kind.clone(),
                line: pos.line,
                col: pos.col,
            }),
        }
    }

    fn parse_block(&mut self) -> Result<Block, ParseError> {
        let pos = self.peek_pos();
        self.expect(TokenKind::LBrace, "'{'")?;

        let mut stmts = Vec::new();
        loop {
            self.skip_semicolons();
            if self.check(&TokenKind::RBrace) || self.check(&TokenKind::Eof) {
                break;
            }
            stmts.push(self.parse_stmt()?);
            stmts.append(&mut self.pending);
        }

        self.expect(TokenKind::RBrace, "'}'")?;
        Ok(Block { stmts, pos })
    }

    /// the name-introducing places (let/const, function, struct, params,
    /// for/catch variables, imports) all go through here. a `@namespace` in
    /// such a spot is rejected by `parse_ident_name` as a reserved name. no.
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

    /// the `member` in `@ns.member`. besides plain identifiers it accepts the
    /// primitive-type keywords, because `@into.int`, `@into.float` and
    /// `@into.string` are spelled exactly like them. the sneaky exception.
    fn parse_member_name(&mut self) -> Result<String, ParseError> {
        let t = self.peek().clone();
        let name = match &t.kind {
            TokenKind::Ident(s) => s.clone(),
            TokenKind::TypeInt => "int".to_string(),
            TokenKind::TypeI8 => "i8".to_string(),
            TokenKind::TypeI16 => "i16".to_string(),
            TokenKind::TypeI32 => "i32".to_string(),
            TokenKind::TypeU8 => "u8".to_string(),
            TokenKind::TypeU16 => "u16".to_string(),
            TokenKind::TypeU32 => "u32".to_string(),
            TokenKind::TypeU64 => "u64".to_string(),
            TokenKind::TypeFloat => "float".to_string(),
            TokenKind::TypeF32 => "f32".to_string(),
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

    // types

    /// a type, with an optional `?` after it: `int?`, `array<int>?`, `Point?`
    /// a type, or several separated by `or` (the same token as `||`): `string or float`
    fn parse_type_expr(&mut self) -> Result<TypeExpr, ParseError> {
        let mut members = vec![self.parse_type_member()?];
        while self.check(&TokenKind::OrOr) {
            self.advance();
            members.push(self.parse_type_member()?);
        }
        if members.len() == 1 {
            Ok(members.remove(0))
        } else {
            Ok(TypeExpr::Union(members))
        }
    }

    /// one member of a union: a type, with its own `?` if it has one
    fn parse_type_member(&mut self) -> Result<TypeExpr, ParseError> {
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
            TokenKind::TypeI8 => Ok(TypeExpr::I8),
            TokenKind::TypeI16 => Ok(TypeExpr::I16),
            TokenKind::TypeI32 => Ok(TypeExpr::I32),
            TokenKind::TypeU8 => Ok(TypeExpr::U8),
            TokenKind::TypeU16 => Ok(TypeExpr::U16),
            TokenKind::TypeU32 => Ok(TypeExpr::U32),
            TokenKind::TypeU64 => Ok(TypeExpr::U64),
            TokenKind::TypeFloat => Ok(TypeExpr::Float),
            TokenKind::TypeF32 => Ok(TypeExpr::F32),
            TokenKind::TypeBool => Ok(TypeExpr::Bool),
            TokenKind::TypeString => Ok(TypeExpr::String),
            TokenKind::TypeVoid => Ok(TypeExpr::Void),
            // `(A or B)`: a group, so `(A or B)?` is an optional union
            TokenKind::LParen => {
                let inner = self.parse_type_expr()?;
                self.expect(TokenKind::RParen, "')' (closes the type group)")?;
                Ok(inner)
            }
            TokenKind::Function => {
                self.expect(TokenKind::LParen, "'(' (fn(T, ...) -> R)")?;
                let mut params = Vec::new();
                if !self.check(&TokenKind::RParen) {
                    params.push(self.parse_type_expr()?);
                    while self.match_kind(&TokenKind::Comma) {
                        params.push(self.parse_type_expr()?);
                    }
                }
                self.expect(TokenKind::RParen, "')' (fn(T, ...) -> R)")?;
                self.expect(TokenKind::Arrow, "'->' (fn(T, ...) -> R)")?;
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
            TokenKind::Ident(name) if name == "task" && self.check(&TokenKind::Lt) => {
                self.advance(); // '<'
                let inner = self.parse_type_expr()?;
                self.expect(TokenKind::Gt, "'>' (task<T>)")?;
                Ok(TypeExpr::Task(Box::new(inner)))
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

    // expressions. precedence chain
    // (or -> and -> equality -> comparison -> term -> factor -> unary -> call -> primary)

    fn parse_expr(&mut self) -> Result<Expr, ParseError> {
        // `if` in expression position is caught right here, at the top
        // of the chain
        if self.check(&TokenKind::If) {
            return self.parse_if_expr();
        }
        self.parse_pipe()
    }

    /// `x |> f(a)` and `x then f(a)`: the pipe. loosest of all the binary
    /// forms, left-associative. the left value becomes the FIRST argument,
    /// so `x |> f(a)` is `f(x, a)` and `a + b |> f()` is `f(a + b)`. both
    /// spellings build the same call node, so nothing downstream knows
    /// the difference. the right side must be a call: `x |> f` alone is
    /// rejected, since there is nothing to apply.
    fn parse_pipe(&mut self) -> Result<Expr, ParseError> {
        let mut left = self.parse_coalesce()?;
        while self.check(&TokenKind::PipeArrow) {
            self.advance();
            let rhs = self.peek().clone();
            let right = self.parse_coalesce()?;
            match right {
                Expr::Call { callee, mut args, pos } => {
                    args.insert(0, left);
                    left = Expr::Call { callee, args, pos };
                }
                _ => {
                    return Err(ParseError::Unexpected {
                        expected: "a function call after the pipe, like `f()`".to_string(),
                        found: rhs.kind,
                        line: rhs.line,
                        col: rhs.col,
                    });
                }
            }
        }
        Ok(left)
    }

    /// `a ?? b ?? c`: the loosest binary operator, right-associative
    fn parse_coalesce(&mut self) -> Result<Expr, ParseError> {
        let left = self.parse_or()?;
        if self.check(&TokenKind::QuestionQuestion) {
            let pos = self.peek_pos();
            self.advance();
            let right = self.parse_coalesce()?;
            let span = self.expr_span(&left, pos);
            return Ok(Expr::Binary { left: Box::new(left), op: BinaryOp::Coalesce, right: Box::new(right), pos: span });
        }
        Ok(left)
    }

    /// parses one expression with struct literals disabled. used for
    /// `if`/`while` conditions and `for .. in` iterables, where `Ident {`
    /// would otherwise be read as a struct literal instead of the
    /// following block's opening brace. saves/restores the previous
    /// flag value (rather than clearing it back to
    /// `false` unconditionally), so nesting works: parsing the condition of an `if`
    /// that sits inside another `if`'s condition still disables struct
    /// literals for the inner one too. nested ifs, nested suffering.
    fn parse_expr_no_struct_literal(&mut self) -> Result<Expr, ParseError> {
        let prev = self.no_struct_literal;
        self.no_struct_literal = true;
        let result = self.parse_expr();
        self.no_struct_literal = prev;
        result
    }

    /// `if (cond) { } when (cond2) { } else { }` as an EXPRESSION (used
    /// as a value, e.g. `let x = if (a) {1} when (b) {2} else {3}`).
    /// same desugaring trick as the statement version: each `when`
    /// becomes a nested `IfExpr` wrapped in a one-statement `Block`, so it
    /// fits the existing `else_branch: Box<Block>` field unchanged. the
    /// difference from the statement version: `else` stays
    /// mandatory at the end of the chain. an if-EXPRESSION must always
    /// produce a value on every path, so the recursion here has no
    /// "no else, no problem" exit. it's `when` all the way down to a
    /// required final `else`. no escape hatch.
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
    /// `else { }`. there's no "neither" case here, unlike the
    /// statement version, since an if-expression always needs a value.
    fn parse_when_or_else_tail_expr(&mut self) -> Result<Block, ParseError> {
        if self.check(&TokenKind::Elif) {
            let when_pos = self.peek_pos();
            self.advance(); // elif
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
            let span = self.expr_span(&left, pos);
            left = Expr::Binary { left: Box::new(left), op: BinaryOp::Or, right: Box::new(right), pos: span };
        }
        Ok(left)
    }

    fn parse_and(&mut self) -> Result<Expr, ParseError> {
        let mut left = self.parse_equality()?;
        while self.check(&TokenKind::AndAnd) {
            let pos = self.peek_pos();
            self.advance();
            let right = self.parse_equality()?;
            let span = self.expr_span(&left, pos);
            left = Expr::Binary { left: Box::new(left), op: BinaryOp::And, right: Box::new(right), pos: span };
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
            let span = self.expr_span(&left, pos);
            left = Expr::Binary { left: Box::new(left), op, right: Box::new(right), pos: span };
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
                TokenKind::In => BinaryOp::In,
                _ => break,
            };
            let pos = self.peek_pos();
            self.advance();
            let right = self.parse_term()?;
            let span = self.expr_span(&left, pos);
            left = Expr::Binary { left: Box::new(left), op, right: Box::new(right), pos: span };
        }
        Ok(left)
    }

    fn parse_term(&mut self) -> Result<Expr, ParseError> {
        let mut left = self.parse_factor()?;
        loop {
            let op = match &self.peek().kind {
                TokenKind::Plus => BinaryOp::Add,
                // `-` at the start of a line is a new statement (`a` / `-1`),
                // not `a - 1`. write `a - 1` or `a -\n 1` to subtract
                TokenKind::Minus if !self.starts_new_line() => BinaryOp::Sub,
                _ => break,
            };
            let pos = self.peek_pos();
            self.advance();
            let right = self.parse_factor()?;
            let span = self.expr_span(&left, pos);
            left = Expr::Binary { left: Box::new(left), op, right: Box::new(right), pos: span };
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
            let span = self.expr_span(&left, pos);
            left = Expr::Binary { left: Box::new(left), op, right: Box::new(right), pos: span };
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
            // `++x` / `--x`: prefix increment/decrement, an expression that
            // yields the NEW value. the target is checked later (see IncDec)
            TokenKind::PlusPlus | TokenKind::MinusMinus => {
                let delta = if self.check(&TokenKind::PlusPlus) { 1 } else { -1 };
                self.advance();
                let target = self.parse_call()?;
                Ok(Expr::IncDec { target: Box::new(target), delta, prefix: true, pos })
            }
            _ => self.parse_power(),
        }
    }

    /// `**` binds tighter than `* / %` (standard math convention) and,
    /// unlike them, is RIGHT-associative: `2 ** 3 ** 2` parses as
    /// `2 ** (3 ** 2)` = 512, not `(2 ** 3) ** 2` = 64. matching every
    /// other language that has `**` (Python, JS, Ruby...). that's why
    /// this recurses back into itself on the right side, instead of
    /// looping like parse_factor does for its left-associative operators.
    fn parse_power(&mut self) -> Result<Expr, ParseError> {
        let base = self.parse_call()?;
        if self.check(&TokenKind::StarStar) {
            let pos = self.peek_pos();
            self.advance();
            let exponent = self.parse_power()?; // right-associative recursion
            let span = self.expr_span(&base, pos);
            return Ok(Expr::Binary { left: Box::new(base), op: BinaryOp::Pow, right: Box::new(exponent), pos: span });
        }
        Ok(base)
    }

    fn parse_call(&mut self) -> Result<Expr, ParseError> {
        let mut expr = self.parse_primary()?;

        loop {
            // `(` and `[` on a new line start a new statement, not a call or
            // an index of the line above (`f` / `(0)` is two statements). newlines matter.
            let breaks_line = self.starts_new_line();
            if self.check(&TokenKind::LParen) && !breaks_line {
                let pos = self.peek_pos();
                self.advance(); // (

                let mut args = Vec::new();
                if !self.check(&TokenKind::RParen) {
                    args.push(self.parse_call_arg()?);
                    while self.match_kind(&TokenKind::Comma) {
                        args.push(self.parse_call_arg()?);
                    }
                }
                self.expect(TokenKind::RParen, "')'")?;
                // once an argument is named, every later one must be too: `f(1, x: 2, 3)` has no order
                let mut seen_named = false;
                for arg in &args {
                    match arg {
                        Expr::NamedArg { .. } => seen_named = true,
                        other if seen_named => {
                            let (line, col) = other.pos().map(|p| (p.line, p.col)).unwrap_or((0, 0));
                            return Err(ParseError::PositionalAfterNamed { line, col });
                        }
                        _ => {}
                    }
                }

                let span = self.expr_span(&expr, pos);
                expr = Expr::Call { callee: Box::new(expr), args, pos: span };
            } else if self.check(&TokenKind::LBracket) && !breaks_line {
                let pos = self.peek_pos();
                self.advance(); // [
                let index = self.parse_expr()?;
                self.expect(TokenKind::RBracket, "']'")?;

                let span = self.expr_span(&expr, pos);
                expr = Expr::Index { array: Box::new(expr), index: Box::new(index), pos: span };
            } else if self.check(&TokenKind::Dot) {
                let pos = self.peek_pos();
                self.advance(); // .
                // `x.string()`, `x.int()`, `x.float()`: the type keywords are also
                // method names after a dot (other names aren't affected)
                let field = match self.peek().kind {
                    TokenKind::TypeInt | TokenKind::TypeFloat | TokenKind::TypeF32 | TokenKind::TypeString | TokenKind::TypeBool
                    | TokenKind::TypeI8 | TokenKind::TypeI16 | TokenKind::TypeI32 | TokenKind::TypeU8 | TokenKind::TypeU16
                    | TokenKind::TypeU32 | TokenKind::TypeU64 => {
                        let name = self.parse_member_name()?;
                        name
                    }
                    _ => self.parse_ident_name()?,
                };

                let span = self.expr_span(&expr, pos);
                expr = Expr::Field { object: Box::new(expr), field, pos: span };
            } else if (self.check(&TokenKind::PlusPlus) || self.check(&TokenKind::MinusMinus)) && !breaks_line {
                // `i++` / `i--`: postfix, yields the OLD value. same line only,
                // so a `++` starting the next line is a prefix of the next statement
                let delta = if self.check(&TokenKind::PlusPlus) { 1 } else { -1 };
                let pos = self.peek_pos();
                self.advance();
                expr = Expr::IncDec { target: Box::new(expr), delta, prefix: false, pos };
                break;
            } else {
                break;
            }
        }

        Ok(expr)
    }

    /// parses `{ field: value, ... }` for a struct literal whose type
    /// name and position were already eaten by `parse_primary`.
    /// struct literals are allowed again inside the field values, even if
    /// the literal itself was reached with `no_struct_literal` cleared
    /// by an enclosing call. once we've committed to `Name {`, we're no
    /// longer in the ambiguous position, so nested field values (e.g. a
    /// field that is itself a struct literal) parse under the ordinary
    /// rule. back to normal.
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

    /// one argument of a call: `expr`, or `name: expr` (a named argument). `Ident :` is
    /// only ever a named argument here, since a call argument is never a type or a map key.
    fn parse_call_arg(&mut self) -> Result<Expr, ParseError> {
        if matches!(self.peek().kind, TokenKind::Ident(_)) && self.next_is(&TokenKind::Colon) {
            let pos = self.peek_pos();
            let name = self.parse_ident_name()?;
            self.expect(TokenKind::Colon, "':'")?;
            let value = self.parse_expr()?;
            return Ok(Expr::NamedArg { name, value: Box::new(value), pos });
        }
        self.parse_expr()
    }

    fn parse_struct_field(&mut self) -> Result<(String, Expr), ParseError> {
        let field_name = self.parse_ident_name()?;
        self.expect(TokenKind::Colon, "':'")?;
        let value = self.parse_expr()?;
        Ok((field_name, value))
    }

    /// `@ns.member` → a single qualified identifier `"@ns.member"`. the
    /// postfix loop then turns `@ns.member(args)` into an ordinary call, so
    /// the typechecker/compiler/VM see namespaces as just a naming scheme
    /// for builtins (no runtime namespace value exists. there's nothing to see).
    fn parse_namespace_access(&mut self, ns: String, pos: Pos) -> Result<Expr, ParseError> {
        if !self.check(&TokenKind::Dot) {
            return Err(ParseError::NamespaceWithoutMember { name: ns, line: pos.line, col: pos.col });
        }
        self.advance(); // .
        let member = self.parse_member_name()?;
        Ok(Expr::Ident(format!("@{}.{}", ns, member), pos))
    }

    /// `int::max`: the type's name and position were already eaten by `parse_primary`.
    /// the property name is checked later, by the typechecker, against the type's list.
    fn parse_type_prop(&mut self, ty: &str, pos: Pos) -> Result<Expr, ParseError> {
        self.advance(); // ::
        let name = self.parse_ident_name()?;
        Ok(Expr::TypeProp { ty: ty.to_string(), name, pos })
    }

    fn parse_primary(&mut self) -> Result<Expr, ParseError> {
        let pos = self.peek_pos();
        match self.advance().kind {
            TokenKind::Int(n) => Ok(Expr::IntLit(n, pos)),
            TokenKind::UInt(n) => Ok(Expr::UIntLit(n, pos)),
            TokenKind::Float(f) => Ok(Expr::FloatLit(f, pos)),
            TokenKind::Str(s) => Ok(Expr::StringLit(s, pos)),
            TokenKind::Bool(b) => Ok(Expr::BoolLit(b, pos)),
            TokenKind::NoneLit => Ok(Expr::NoneLit(pos)),
            TokenKind::Namespace(name) => self.parse_namespace_access(name, pos),
            TokenKind::Function => {
                self.expect(TokenKind::LParen, "'(' (anonymous function)")?;
                let mut params = Vec::new();
                if !self.check(&TokenKind::RParen) {
                    params.push(self.parse_param(false)?);
                    while self.match_kind(&TokenKind::Comma) {
                        params.push(self.parse_param(false)?);
                    }
                }
                self.expect(TokenKind::RParen, "')'")?;
                let return_type = if self.match_kind(&TokenKind::Arrow) { Some(self.parse_type_expr()?) } else { None };
                let body = self.parse_block()?;
                Ok(Expr::Lambda { params, return_type, body, pos })
            }
            TokenKind::TypeInt if self.check(&TokenKind::DoubleColon) => self.parse_type_prop("int", pos),
            TokenKind::TypeFloat if self.check(&TokenKind::DoubleColon) => self.parse_type_prop("float", pos),
            TokenKind::TypeF32 if self.check(&TokenKind::DoubleColon) => self.parse_type_prop("f32", pos),
            TokenKind::TypeI8 if self.check(&TokenKind::DoubleColon) => self.parse_type_prop("i8", pos),
            TokenKind::TypeI16 if self.check(&TokenKind::DoubleColon) => self.parse_type_prop("i16", pos),
            TokenKind::TypeI32 if self.check(&TokenKind::DoubleColon) => self.parse_type_prop("i32", pos),
            TokenKind::TypeU8 if self.check(&TokenKind::DoubleColon) => self.parse_type_prop("u8", pos),
            TokenKind::TypeU16 if self.check(&TokenKind::DoubleColon) => self.parse_type_prop("u16", pos),
            TokenKind::TypeU32 if self.check(&TokenKind::DoubleColon) => self.parse_type_prop("u32", pos),
            TokenKind::TypeU64 if self.check(&TokenKind::DoubleColon) => self.parse_type_prop("u64", pos),
            TokenKind::TypeBool if self.check(&TokenKind::DoubleColon) => self.parse_type_prop("bool", pos),
            TokenKind::TypeString if self.check(&TokenKind::DoubleColon) => self.parse_type_prop("string", pos),
            TokenKind::TypeArray if self.check(&TokenKind::DoubleColon) => self.parse_type_prop("array", pos),
            TokenKind::Ident(name) => {
                // `Name {` starts a struct literal, unless we're inside
                // an if/while condition or a for-in iterable. there,
                // the `{` almost always belongs to the following block,
                // not to a struct literal (see `no_struct_literal`'s
                // doc comment). checking `self.check(&LBrace)` here
                // (rather than in the postfix loop) keeps this ambiguity
                // local to the one token sequence that causes it. contained mess.
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
    /// the same fact, friendly tone (`#> friendly-terminal`). the nice version.
    pub fn friendly(&self) -> String {
        self.friendly_text(None)
    }

    /// the friendly message for an error in `file` (the parser's own file name)
    pub fn friendly_in(&self, file: &str) -> String {
        self.friendly_text(Some(file))
    }

    fn friendly_text(&self, file: Option<&str>) -> String {
        let place = |line: &usize, col: &usize| match file {
            Some(f) => format!("on line {}, column {} in `{}`", line, col, f),
            None => format!("on line {}, column {}", line, col),
        };
        match self {
            ParseError::Unexpected { expected, found, line, col } => {
                let hint = if matches!(found, TokenKind::Eof) { spg_lexer::HINT_EOF } else { spg_lexer::HINT_SYNTAX };
                format!(
                    "Oops! {} I expected {}, but I found {}. {}",
                    place(line, col), expected, describe_token(found), hint
                )
            }
            ParseError::UnexpectedEof { expected } => match file {
                Some(f) => format!("Oops! the file `{}` ended too early, while I was still waiting for {}. {}", f, expected, spg_lexer::HINT_EOF),
                None => format!("Oops! the file ended too early, while I was still waiting for {}. {}", expected, spg_lexer::HINT_EOF),
            },
            ParseError::MissingElseInIfExpr { line, col } => format!(
                "Oops! {} an 'if' used as a value needs an 'else' too. {}",
                place(line, col), spg_lexer::HINT_ELSE
            ),
            // the rarer ones keep their own wording, with the general hint
            _ => {
                let fact = match file {
                    Some(f) => format!("{} in `{}`", self, f),
                    None => self.to_string(),
                };
                spg_lexer::friendly_sentence(&fact, spg_lexer::HINT_SYNTAX)
            }
        }
    }
}

/// a token as a person would say it: `the name 'x'`, `the number 4`, `'(' `, `the end of the file`
fn describe_token(k: &TokenKind) -> String {
    match k {
        TokenKind::Eof => "the end of the file".to_string(),
        TokenKind::Newline => "the end of the line".to_string(),
        TokenKind::Ident(s) => format!("the name '{}'", s),
        TokenKind::Int(n) => format!("the number {}", n),
        TokenKind::UInt(n) => format!("the number {}", n),
        TokenKind::Float(x) => format!("the number {}", x),
        TokenKind::Str(_) => "a text".to_string(),
        TokenKind::Bool(b) => format!("'{}'", b),
        TokenKind::NoneLit => "'none'".to_string(),
        other => format!("'{}'", symbol_text(other)),
    }
}

/// the written form of a symbol or keyword token (for messages)
fn symbol_text(k: &TokenKind) -> &'static str {
    match k {
        TokenKind::Plus => "+",
        TokenKind::Minus => "-",
        TokenKind::Star => "*",
        TokenKind::Slash => "/",
        TokenKind::Percent => "%",
        TokenKind::StarStar => "**",
        TokenKind::Eq => "=",
        TokenKind::EqEq => "==",
        TokenKind::NotEq => "!=",
        TokenKind::PlusEq => "+=",
        TokenKind::MinusEq => "-=",
        TokenKind::StarEq => "*=",
        TokenKind::SlashEq => "/=",
        TokenKind::PercentEq => "%=",
        TokenKind::PlusPlus => "++",
        TokenKind::MinusMinus => "--",
        TokenKind::Lt => "<",
        TokenKind::Gt => ">",
        TokenKind::LtEq => "<=",
        TokenKind::GtEq => ">=",
        TokenKind::AndAnd => "&&",
        TokenKind::OrOr => "||",
        TokenKind::PipeArrow => "|>",
        TokenKind::Question => "?",
        TokenKind::QuestionQuestion => "??",
        TokenKind::Not => "!",
        TokenKind::Arrow => "->",
        TokenKind::Colon => ":",
        TokenKind::Comma => ",",
        TokenKind::Dot => ".",
        TokenKind::DoubleColon => "::",
        TokenKind::DotDot => "..",
        TokenKind::LParen => "(",
        TokenKind::RParen => ")",
        TokenKind::LBrace => "{",
        TokenKind::RBrace => "}",
        TokenKind::LAngle => "<",
        TokenKind::RAngle => ">",
        TokenKind::LBracket => "[",
        TokenKind::RBracket => "]",
        TokenKind::Semicolon => ";",
        TokenKind::Let => "let",
        TokenKind::Const => "const",
        TokenKind::Function => "fn",
        TokenKind::Return => "return",
        TokenKind::If => "if",
        TokenKind::Else => "else",
        TokenKind::While => "while",
        TokenKind::Do => "do",
        TokenKind::For => "for",
        TokenKind::In => "in",
        TokenKind::Try => "try",
        TokenKind::Catch => "catch",
        TokenKind::Throw => "throw",
        TokenKind::Break => "break",
        TokenKind::Continue => "continue",
        TokenKind::When => "when",
        TokenKind::Elif => "elif",
        TokenKind::Use => "use",
        TokenKind::Attach => "attach",
        TokenKind::Public => "public",
        TokenKind::From => "from",
        TokenKind::Struct => "struct",
        TokenKind::NamespaceKw => "namespace",
        TokenKind::Enum => "enum",
        TokenKind::TypeInt => "int",
        TokenKind::TypeI8 => "i8",
        TokenKind::TypeI16 => "i16",
        TokenKind::TypeI32 => "i32",
        TokenKind::TypeU8 => "u8",
        TokenKind::TypeU16 => "u16",
        TokenKind::TypeU32 => "u32",
        TokenKind::TypeU64 => "u64",
        TokenKind::TypeFloat => "float",
        TokenKind::TypeF32 => "f32",
        TokenKind::TypeBool => "bool",
        TokenKind::TypeString => "string",
        TokenKind::TypeVoid => "void",
        TokenKind::TypeArray => "array",
        _ => "this part of the code",
    }
}

/// `som` + `add` -> `som.add`, for a member declared inside `namespace som { }`
fn qualify_member(ns: &str, mut stmt: Stmt) -> Stmt {
    match &mut stmt {
        Stmt::Let { name, .. } | Stmt::Const { name, .. } | Stmt::FunctionDecl { name, .. } => {
            let full = format!("{}.{}", ns, name);
            *name = full;
        }
        _ => {}
    }
    stmt
}

impl ParseError {
    /// the place of the error, when the error carries one: (line, column)
    pub fn spot(&self) -> Option<(usize, usize)> {
        match self {
            ParseError::Unexpected { line, col, .. }
            | ParseError::MissingElseInIfExpr { line, col }
            | ParseError::ReservedName { line, col, .. }
            | ParseError::NamespaceWithoutMember { line, col, .. }
            | ParseError::RangeOutsideFor { line, col }
            | ParseError::PositionalAfterNamed { line, col }
            | ParseError::FieldMethodClash { line, col, .. }
            | ParseError::GuardError { line, col, .. }
            | ParseError::DuplicateVariant { line, col, .. }
            | ParseError::EmptyEnum { line, col, .. }
            | ParseError::EnumValueType { line, col, .. }
            | ParseError::EnumValueNeeded { line, col, .. } => Some((*line, *col)),
            ParseError::UnexpectedEof { .. } => None,
        }
    }

    /// stable code for the frame header (`error[E0100]`)
    pub fn code(&self) -> &'static str {
        match self {
            ParseError::Unexpected { .. } => "E0100",
            ParseError::MissingElseInIfExpr { .. } => "E0101",
            ParseError::UnexpectedEof { .. } => "E0102",
            ParseError::ReservedName { .. } => "E0103",
            ParseError::NamespaceWithoutMember { .. } => "E0104",
            ParseError::RangeOutsideFor { .. } => "E0105",
            ParseError::PositionalAfterNamed { .. } => "E0106",
            ParseError::FieldMethodClash { .. } => "E0107",
            ParseError::GuardError { .. } => "E0108",
            ParseError::DuplicateVariant { .. } => "E0109",
            ParseError::EmptyEnum { .. } => "E0110",
            ParseError::EnumValueType { .. } => "E0111",
            ParseError::EnumValueNeeded { .. } => "E0112",
        }
    }

    /// the bare fact for the frame header
    pub fn fact(&self) -> String {
        match self {
            ParseError::Unexpected { expected, found, .. } => {
                format!("expected {}, found {}", expected, spg_lexer::token::describe(found))
            }
            ParseError::MissingElseInIfExpr { .. } => "'if' used as a value needs an 'else'".to_string(),
            ParseError::UnexpectedEof { expected } => format!("unexpected end of file, expected {}", expected),
            ParseError::ReservedName { name, .. } => format!("'@{}' is a library namespace, not a name", name.trim_start_matches('@')),
            ParseError::NamespaceWithoutMember { name, .. } => format!("namespace '@{}' used without a member", name),
            ParseError::RangeOutsideFor { .. } => "a range (a..b) is only allowed in a 'for' header".to_string(),
            ParseError::PositionalAfterNamed { .. } => "a positional argument cannot follow a named one".to_string(),
            ParseError::FieldMethodClash { struct_name, name, .. } => {
                format!("'{}' is both a field and a method of '{}'", name, struct_name)
            }
            ParseError::GuardError { struct_name, reason, .. } => {
                format!("invalid guard rule in '{}': {}", struct_name, reason)
            }
            ParseError::DuplicateVariant { enum_name, name, .. } => {
                format!("variant '{}' is declared twice in enum '{}'", name, enum_name)
            }
            ParseError::EmptyEnum { name, .. } => format!("enum '{}' needs at least one variant", name),
            ParseError::EnumValueType { enum_name, name, found, expected, .. } => format!(
                "variant '{}' of enum '{}' is {}, but its other variants are {}",
                name, enum_name, found, expected
            ),
            ParseError::EnumValueNeeded { enum_name, name, .. } => {
                format!("variant '{}' of enum '{}' needs a value", name, enum_name)
            }
        }
    }

    /// why the parser can't go on, in one sentence (none when the fact says it all)
    pub fn why(&self) -> Option<String> {
        let text = match self {
            ParseError::Unexpected { .. } => "the statement is not complete or the text is not what the grammar expects here".to_string(),
            ParseError::MissingElseInIfExpr { .. } => "an 'if' that gives a value must give one on every path".to_string(),
            ParseError::UnexpectedEof { .. } => "the file ends while a block, a bracket or a statement is still open".to_string(),
            ParseError::ReservedName { .. } => "namespaces (@math, @time...) are reserved: they can't name variables, functions, types or fields".to_string(),
            ParseError::NamespaceWithoutMember { .. } => "a namespace is a group of functions, so it must be followed by '.name'".to_string(),
            ParseError::RangeOutsideFor { .. } => "ranges exist to walk through values in a for loop".to_string(),
            ParseError::PositionalAfterNamed { .. } => "once one argument is named, every later one must be named too".to_string(),
            ParseError::FieldMethodClash { name, .. } => format!("'p.{}' would not say which one it means", name),
            ParseError::GuardError { .. } => return None,
            ParseError::DuplicateVariant { .. } => "each variant of an enum needs its own name".to_string(),
            ParseError::EmptyEnum { .. } => "an enum is a set of values, and an empty set has nothing to choose from".to_string(),
            ParseError::EnumValueType { .. } => "the variants of one enum share one value type".to_string(),
            ParseError::EnumValueNeeded { .. } => "only int enums are numbered automatically".to_string(),
        };
        Some(text)
    }

    /// concrete ways to fix it, built from the text of this error
    pub fn help(&self) -> Vec<String> {
        match self {
            ParseError::Unexpected { expected, found, .. } => {
                let mut out = Vec::new();
                if expected.contains("')'") {
                    out.push("close the '(' opened before this with ')'".to_string());
                } else if expected.contains("']'") {
                    out.push("close the '[' opened before this with ']'".to_string());
                } else if expected.contains("'}'") {
                    out.push("close the block opened with '{' with '}'".to_string());
                } else if expected.contains("'{'") {
                    out.push("a block must start with '{' after its header".to_string());
                } else if matches!(found, TokenKind::Ident(_)) {
                    out.push("two names side by side: check for a missing operator, comma or ')' just before it".to_string());
                }
                out
            }
            ParseError::MissingElseInIfExpr { .. } => {
                vec!["add an else branch that gives a value of the same type, for example: else { 0 }".to_string()]
            }
            ParseError::UnexpectedEof { .. } => {
                vec!["a '{', '(' or '[' was opened and never closed: look for the one that is still open".to_string()]
            }
            ParseError::ReservedName { name, .. } => {
                vec![format!("choose another name, for example: {}Value", name.trim_start_matches('@'))]
            }
            ParseError::NamespaceWithoutMember { name, .. } => {
                vec![format!("use one of its members, for example: @{}.<name>(...)", name)]
            }
            ParseError::RangeOutsideFor { .. } => vec![
                "use it in a for loop: for (i in 0..10) { ... }".to_string(),
                "or build the list with @array.range(1, 4)".to_string(),
            ],
            ParseError::PositionalAfterNamed { .. } => vec![
                "name it too, for example: f(x: 1, y: 2)".to_string(),
                "or put the positional arguments before the named ones".to_string(),
            ],
            ParseError::FieldMethodClash { name, .. } => {
                vec![format!("rename the field or the method '{}'", name)]
            }
            ParseError::GuardError { .. } => vec![],
            ParseError::DuplicateVariant { name, .. } => {
                vec![format!("remove one of the two '{}', or give the second one another name", name)]
            }
            ParseError::EmptyEnum { name, .. } => {
                vec![format!("add at least one variant, for example: enum {} {{ First }}", name)]
            }
            ParseError::EnumValueType { expected, .. } => {
                vec![format!("give it a value of type {}, like the others", expected)]
            }
            ParseError::EnumValueNeeded { .. } => {
                vec!["give it a value of the enum's type, for example: Name = \"text\"".to_string()]
            }
        }
    }
}

/// a type keyword that names a type with value methods (`int`, `i8`, `u64`, `f32`...), with its
/// name and the type expression of `me`. `i64` and `f64` lex as `int` and `float`, so they are here too.
fn value_type_token(kind: Option<&TokenKind>) -> Option<(&'static str, TypeExpr)> {
    Some(match kind? {
        TokenKind::TypeInt => ("int", TypeExpr::Int),
        TokenKind::TypeFloat => ("float", TypeExpr::Float),
        TokenKind::TypeF32 => ("f32", TypeExpr::F32),
        TokenKind::TypeBool => ("bool", TypeExpr::Bool),
        TokenKind::TypeString => ("string", TypeExpr::String),
        TokenKind::TypeI8 => ("i8", TypeExpr::I8),
        TokenKind::TypeI16 => ("i16", TypeExpr::I16),
        TokenKind::TypeI32 => ("i32", TypeExpr::I32),
        TokenKind::TypeU8 => ("u8", TypeExpr::U8),
        TokenKind::TypeU16 => ("u16", TypeExpr::U16),
        TokenKind::TypeU32 => ("u32", TypeExpr::U32),
        TokenKind::TypeU64 => ("u64", TypeExpr::U64),
        _ => return None,
    })
}
