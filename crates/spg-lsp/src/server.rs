//! the LSP loop: framing (`Content-Length`), dispatch, open-buffer
//! bookkeeping, diagnostics publishing and completion. the whole circus.

use std::collections::HashMap;
use std::io::{self, BufRead, Write};
use std::path::PathBuf;

use crate::diag::{self, Diag};
use crate::json::Json;
use crate::uri;

/// speaks LSP over the process's stdin/stdout. returns the exit code the
/// process should use (0 after a clean `shutdown` + `exit`).
pub fn run_stdio() -> i32 {
    let stdin = io::stdin();
    let stdout = io::stdout();
    run(stdin.lock(), stdout.lock())
}

pub fn run<R: BufRead, W: Write>(mut input: R, out: W) -> i32 {
    let mut server = Server::new(out);
    loop {
        match read_message(&mut input) {
            Ok(Some(text)) => match Json::parse(&text) {
                Ok(msg) => {
                    if let Some(code) = server.handle(&msg) {
                        return code;
                    }
                }
                Err(e) => server.send_error(Json::Null, -32700, &format!("parse error: {}", e)),
            },
            // stream closed. clean only if the client asked for shutdown first
            Ok(None) | Err(_) => return if server.shutdown { 0 } else { 1 },
        }
    }
}

fn read_message<R: BufRead>(r: &mut R) -> io::Result<Option<String>> {
    let mut len: Option<usize> = None;
    loop {
        let mut line = String::new();
        if r.read_line(&mut line)? == 0 {
            return Ok(None);
        }
        let t = line.trim_end_matches(&['\r', '\n'][..]);
        if t.is_empty() {
            if len.is_some() {
                break;
            }
            continue;
        }
        if let Some((k, v)) = t.split_once(':') {
            if k.eq_ignore_ascii_case("content-length") {
                len = v.trim().parse().ok();
            }
        }
    }
    let mut buf = vec![0u8; len.unwrap_or(0)];
    r.read_exact(&mut buf)?;
    Ok(Some(String::from_utf8_lossy(&buf).into_owned()))
}

struct Doc {
    /// canonical path. None for documents that aren't files on disk
    path: Option<PathBuf>,
    text: String,
    version: i64,
}

struct Server<W: Write> {
    out: W,
    docs: HashMap<String, Doc>,
    /// unsaved text by canonical path, handed to the module resolver
    overlay: HashMap<PathBuf, String>,
    shutdown: bool,
}

// `#> name` directives: (name, text inserted, one-line help). a test keeps it in
// step with `spg_lexer::DIRECTIVES`
const DIRECTIVE_DOCS: &[(&str, &str, &str)] = &[
    ("friendly-terminal", "friendly-terminal", "#> friendly-terminal: error messages in a friendlier tone"),
    ("timeout", "timeout ", "#> timeout <ms>: stops the program after <ms> milliseconds (exit code 124)"),
    ("strict", "strict", "#> strict: int overflow, hidden names and unreachable code become errors"),
    ("safe-party", "safe-party", "#> safe-party: ps.run and cmd.run are off (cmd.exists still works); ps.runFile only takes a .ps1 inside the script's folder (no absolute paths, no ..)"),
    ("no-jit", "no-jit", "#> no-jit: run this file in the VM only, without the native code"),
    ("stack", "stack ", "#> stack <n>: how many nested calls the program may make, from 1 to 100000"),
    ("seed", "seed ", "#> seed <n>: the random numbers start from this seed, so the runs repeat"),
];

impl<W: Write> Server<W> {
    fn new(out: W) -> Self {
        Server { out, docs: HashMap::new(), overlay: HashMap::new(), shutdown: false }
    }

    // ---------- output (the writing part) ----------

    fn send(&mut self, body: &Json) {
        let s = body.to_string();
        let _ = write!(self.out, "Content-Length: {}\r\n\r\n{}", s.len(), s);
        let _ = self.out.flush();
    }

    fn reply(&mut self, id: Option<Json>, result: Json) {
        if let Some(id) = id {
            self.send(&Json::obj(vec![("jsonrpc", Json::str("2.0")), ("id", id), ("result", result)]));
        }
    }

    fn send_error(&mut self, id: Json, code: i64, message: &str) {
        self.send(&Json::obj(vec![
            ("jsonrpc", Json::str("2.0")),
            ("id", id),
            ("error", Json::obj(vec![("code", Json::int(code)), ("message", Json::str(message))])),
        ]));
    }

    fn notify(&mut self, method: &str, params: Json) {
        self.send(&Json::obj(vec![
            ("jsonrpc", Json::str("2.0")),
            ("method", Json::str(method)),
            ("params", params),
        ]));
    }

    // ---------- dispatch (the switchboard) ----------

    /// `Some(code)` means: exit the process with that code now.
    fn handle(&mut self, msg: &Json) -> Option<i32> {
        // a message without `method` is a response to us. nothing to do
        let method = msg.get("method")?.as_str()?.to_string();
        let id = msg.get("id").cloned();
        let params = msg.get("params").cloned().unwrap_or(Json::Null);

        match method.as_str() {
            "initialize" => {
                let caps = Json::obj(vec![
                    (
                        "textDocumentSync",
                        Json::obj(vec![
                            ("openClose", Json::Bool(true)),
                            ("change", Json::int(1)), // full document on every change
                            ("save", Json::obj(vec![("includeText", Json::Bool(false))])),
                        ]),
                    ),
                    ("completionProvider", Json::obj(vec![("resolveProvider", Json::Bool(false)), ("triggerCharacters", Json::Arr(vec![Json::str("."), Json::str("@"), Json::str(":")]))])),
                    ("hoverProvider", Json::Bool(true)),
                    ("documentFormattingProvider", Json::Bool(true)),
                    ("definitionProvider", Json::Bool(true)),
                    ("signatureHelpProvider", Json::obj(vec![("triggerCharacters", Json::Arr(vec![Json::str("("), Json::str(",")]))])),
                ]);
                self.reply(
                    id,
                    Json::obj(vec![
                        ("capabilities", caps),
                        (
                            "serverInfo",
                            Json::obj(vec![
                                ("name", Json::str("spg")),
                                ("version", Json::str(env!("CARGO_PKG_VERSION"))),
                            ]),
                        ),
                    ]),
                );
            }
            "shutdown" => {
                self.shutdown = true;
                self.reply(id, Json::Null);
            }
            "exit" => return Some(if self.shutdown { 0 } else { 1 }),
            "textDocument/didOpen" => {
                if let Some(td) = params.get("textDocument") {
                    let uri = td.get("uri").and_then(Json::as_str).unwrap_or("").to_string();
                    let text = td.get("text").and_then(Json::as_str).unwrap_or("").to_string();
                    let version = td.get("version").and_then(Json::as_f64).unwrap_or(0.0) as i64;
                    self.set_doc(uri, text, version);
                    self.refresh_all();
                }
            }
            "textDocument/didChange" => {
                let uri = params.get("textDocument").and_then(|t| t.get("uri")).and_then(Json::as_str);
                let version = params
                    .get("textDocument")
                    .and_then(|t| t.get("version"))
                    .and_then(Json::as_f64)
                    .unwrap_or(0.0) as i64;
                let text = params
                    .get("contentChanges")
                    .and_then(Json::as_array)
                    .and_then(|a| a.last())
                    .and_then(|c| c.get("text"))
                    .and_then(Json::as_str);
                if let (Some(uri), Some(text)) = (uri, text) {
                    self.set_doc(uri.to_string(), text.to_string(), version);
                    self.refresh_all();
                }
            }
            "textDocument/didSave" => self.refresh_all(),
            "textDocument/didClose" => {
                if let Some(uri) = params.get("textDocument").and_then(|t| t.get("uri")).and_then(Json::as_str) {
                    if let Some(doc) = self.docs.remove(uri) {
                        if let Some(p) = doc.path {
                            self.overlay.remove(&p);
                        }
                    }
                    // clear what we showed for it
                    self.notify(
                        "textDocument/publishDiagnostics",
                        Json::obj(vec![("uri", Json::str(uri)), ("diagnostics", Json::Arr(vec![]))]),
                    );
                    self.refresh_all();
                }
            }
            "textDocument/formatting" => {
                let uri = params.get("textDocument").and_then(|t| t.get("uri")).and_then(Json::as_str).unwrap_or("");
                // no edits when the file can't be formatted safely (see `spg_fmt::check`)
                let result = self.formatting(uri).unwrap_or_else(|| Json::Arr(Vec::new()));
                self.reply(id, result);
            }
            "textDocument/hover" => {
                let uri = params.get("textDocument").and_then(|t| t.get("uri")).and_then(Json::as_str).unwrap_or("");
                let cursor = params.get("position").and_then(|p| {
                    let line = p.get("line").and_then(Json::as_f64)? as usize;
                    let col = p.get("character").and_then(Json::as_f64)? as usize;
                    Some((line, col))
                });
                let result = self.hover(uri, cursor).unwrap_or(Json::Null);
                self.reply(id, result);
            }
            "textDocument/definition" => {
                let uri = params.get("textDocument").and_then(|t| t.get("uri")).and_then(Json::as_str).unwrap_or("");
                let cursor = params.get("position").and_then(|p| {
                    let line = p.get("line").and_then(Json::as_f64)? as usize;
                    let col = p.get("character").and_then(Json::as_f64)? as usize;
                    Some((line, col))
                });
                let result = self.definition(uri, cursor).unwrap_or(Json::Null);
                self.reply(id, result);
            }
            "textDocument/signatureHelp" => {
                let uri = params.get("textDocument").and_then(|t| t.get("uri")).and_then(Json::as_str).unwrap_or("");
                let cursor = params.get("position").and_then(|p| {
                    let line = p.get("line").and_then(Json::as_f64)? as usize;
                    let col = p.get("character").and_then(Json::as_f64)? as usize;
                    Some((line, col))
                });
                let result = self.signature_help(uri, cursor).unwrap_or(Json::Null);
                self.reply(id, result);
            }
            "textDocument/completion" => {
                let uri = params.get("textDocument").and_then(|t| t.get("uri")).and_then(Json::as_str).unwrap_or("");
                let cursor = params.get("position").and_then(|p| {
                    let line = p.get("line").and_then(Json::as_f64)? as usize;
                    let col = p.get("character").and_then(Json::as_f64)? as usize;
                    Some((line, col))
                });
                // `:` also opens completion, but only `int::` uses it: a lone `:` (struct field, map entry) offers nothing
                let by_colon = params.get("context").and_then(|c| c.get("triggerCharacter")).and_then(Json::as_str) == Some(":");
                let items = if by_colon && !matches!(self.dot_context(uri, cursor), DotContext::TypeProp(_)) {
                    Vec::new()
                } else {
                    self.completion(uri, cursor)
                };
                self.reply(id, Json::Arr(items));
            }
            _ => {
                // unknown REQUEST -> tell the client. unknown notification -> ignore
                if let Some(id) = id {
                    self.send_error(id, -32601, &format!("method not found: {}", method));
                }
            }
        }
        None
    }

    // ---------- documents (the paperwork) ----------

    fn set_doc(&mut self, uri: String, text: String, version: i64) {
        let path = uri::uri_to_path(&uri).and_then(|p| spg_modules::canon(&p).ok());
        if let Some(p) = &path {
            self.overlay.insert(p.clone(), text.clone());
        }
        self.docs.insert(uri, Doc { path, text, version });
    }

    /// re-analyses every open document. one file's edit can break (or fix)
    /// another that imports it. domino effect.
    fn refresh_all(&mut self) {
        let mut uris: Vec<String> = self.docs.keys().cloned().collect();
        uris.sort();
        for u in uris {
            self.publish(&u);
        }
    }

    fn publish(&mut self, uri: &str) {
        let Some(doc) = self.docs.get(uri) else { return };
        let version = doc.version;
        let items: Vec<Json> = match &doc.path {
            None => Vec::new(),
            // `// [ShowErrors-Off]` on its own line: the editor stops drawing
            // error squiggles for this file. editor-only; `spg run`/`check`
            // still report everything. for sketching code that is wrong on purpose.
            Some(_) if hides_errors(&doc.text) => Vec::new(),
            Some(path) => {
                let diags = diag::analyze(path, &self.overlay);
                diags.iter().map(|d| self.to_lsp(d, path, &doc.text)).collect()
            }
        };
        self.notify(
            "textDocument/publishDiagnostics",
            Json::obj(vec![
                ("uri", Json::str(uri)),
                ("version", Json::int(version)),
                ("diagnostics", Json::Arr(items)),
            ]),
        );
    }

    fn to_lsp(&self, d: &Diag, doc_path: &PathBuf, doc_text: &str) -> Json {
        let (line0, start, end, message);
        if &d.file == doc_path {
            line0 = d.line.saturating_sub(1);
            let line_text = doc_text.lines().nth(line0).unwrap_or("");
            let (s, e) = diag::utf16_range(line_text, d.col);
            start = s;
            end = e;
            message = d.message.clone();
        } else {
            // the error lives in ANOTHER file this one imports. pin it to
            // the top of this file, and say where it really is
            line0 = 0;
            start = 0;
            end = 1;
            message = format!("in {}:{}:{}: {}", d.file.display(), d.line, d.col, d.message);
        }
        let pos = |l: usize, c: usize| Json::obj(vec![("line", Json::int(l as i64)), ("character", Json::int(c as i64))]);
        Json::obj(vec![
            ("range", Json::obj(vec![("start", pos(line0, start)), ("end", pos(line0, end))])),
            ("severity", Json::int(1)),
            ("source", Json::str("spg")),
            ("message", Json::Str(message)),
        ])
    }

    // ---------- completion (guessing, but politely) ----------

    /// `attach #` / `attach #ht`: the names of the standard libraries. the native ones (from the
    /// registry) and every `.spg` in the stdlib folder
    fn std_lib_completion(&self, uri: &str, pos: Option<(usize, usize)>) -> Option<Vec<Json>> {
        let before = self.line_before_cursor(uri, pos)?;
        let rest = before.trim_start().strip_prefix("attach")?;
        // `attach #` and `attach#` both work; `installer#` is another word
        if rest.starts_with(|c: char| c.is_alphanumeric() || c == '_') {
            return None;
        }
        let partial = rest.trim_start().strip_prefix('#')?;
        if !partial.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
            return None;
        }
        let item = |label: &str, detail: &str| {
            Json::obj(vec![("label", Json::str(label)), ("kind", Json::int(9)), ("detail", Json::Str(detail.to_string()))])
        };
        let mut items: Vec<Json> = Vec::new();
        let mut seen = std::collections::HashSet::new();
        for m in spg_libs::modules().iter().filter(|m| m.kind == spg_libs::Kind::Attach) {
            if seen.insert(m.name.to_string()) {
                items.push(item(m.name, m.doc));
            }
        }
        if let Some(dir) = spg_modules::std_dir() {
            let mut names: Vec<String> = std::fs::read_dir(dir)
                .into_iter()
                .flatten()
                .flatten()
                .filter_map(|e| e.path().file_stem().map(|s| s.to_string_lossy().into_owned()).filter(|_| e.path().extension().is_some_and(|x| x == "spg")))
                .collect();
            names.sort();
            for name in names {
                if seen.insert(name.clone()) {
                    items.push(item(&name, "standard library"));
                }
            }
        }
        Some(items)
    }

    /// text of the cursor's line before the cursor (LSP columns are UTF-16
    /// units. identifiers here are ASCII, so a char-based prefix is exact for
    /// everything that matters. (the rest is noise.)
    fn line_before_cursor(&self, uri: &str, pos: Option<(usize, usize)>) -> Option<String> {
        let (line, col) = pos?;
        let doc = self.docs.get(uri)?;
        // `split('\n')`, not `lines()`: the empty last line (the cursor right after
        // a final newline, e.g. after pressing Enter) must exist too
        let text_line = doc.text.split('\n').nth(line)?;
        Some(text_line.trim_end_matches('\r').chars().take(col).collect())
    }

    /// what the cursor is right after: `@ns.` / `@ns.partial` (a builtin
    /// namespace), `@` / `@partial` (a namespace name being typed), any other
    /// `.` (struct field, float literal, text inside a string...), or nothing
    /// special
    fn dot_context(&self, uri: &str, pos: Option<(usize, usize)>) -> DotContext {
        let before = match self.line_before_cursor(uri, pos) {
            Some(b) => b,
            None => return DotContext::None,
        };
        let word_free = before.trim_end_matches(|c: char| c.is_ascii_alphanumeric() || c == '_');
        // `@` or `@par`: the namespace name itself is being typed
        if word_free.ends_with('@') {
            return DotContext::NamespaceName;
        }
        // `int::` / `float::`: the name before `::` is the type whose property is picked
        if let Some(head) = word_free.strip_suffix("::") {
            let ty: String = head
                .chars()
                .rev()
                .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
                .collect::<Vec<_>>()
                .into_iter()
                .rev()
                .collect();
            return DotContext::TypeProp(ty);
        }
        // `int::methods.`: the member after `methods` is `add`, which declares a value method
        if let Some(head) = word_free.strip_suffix('.').and_then(|b| b.strip_suffix("::methods")) {
            let ty: String = head
                .chars()
                .rev()
                .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
                .collect::<Vec<_>>()
                .into_iter()
                .rev()
                .collect();
            return DotContext::TypeMethods(ty);
        }
        let before = match word_free.strip_suffix('.') {
            Some(b) => b,
            None => return DotContext::None,
        };
        let ns: String = before
            .chars()
            .rev()
            .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect();
        // a namespace is `@` + name: `p.math.` (a struct field) and a plain
        // `math.` (an ordinary variable) are never one
        let head = &before[..before.len() - ns.len()];
        if head.ends_with('@') && spg_lexer::is_namespace(&ns) {
            DotContext::Namespace(ns)
        } else if !head.ends_with('.') && !head.ends_with('@') && !ns.is_empty() && self.enum_variants(uri, &ns).is_some() {
            // `Color.`: the name of an enum declared in this document
            DotContext::Enum(ns)
        } else if spg_lexer::is_install_namespace(&ns) && !head.ends_with('.') && !head.ends_with('@') && self.installs_native(uri, &ns) {
            // `ps.` / `http.`: a native library's namespace, while this document installs it
            DotContext::Native(ns)
        } else {
            DotContext::Other
        }
    }

    /// whether this document has `attach #name` for a native library
    fn installs_native(&self, uri: &str, name: &str) -> bool {
        use spg_lexer::TokenKind as T;
        let Some(doc) = self.docs.get(uri) else { return false };
        let Ok(tokens) = spg_lexer::Lexer::new(&doc.text).tokenize() else { return false };
        tokens.iter().enumerate().any(|(i, t)| {
            t.kind == T::Attach && tokens.get(i + 1).is_some_and(|u| u.kind == T::StdLib(name.to_string()))
        })
    }

    /// the variants of `enum <name> { .. }` declared in this document, found by
    /// lexing it (`None` when there's no such enum or the text doesn't lex)
    fn enum_variants(&self, uri: &str, name: &str) -> Option<Vec<String>> {
        let doc = self.docs.get(uri)?;
        let tokens = spg_lexer::Lexer::new(&doc.text).tokenize().ok()?;
        if let Some(variants) = variants_in(&tokens, name) {
            return Some(variants);
        }
        // an enum of an installed native library (`Key` of `attach #ui`) is declared in the
        // library's struct text, not in this document
        for module in spg_libs::modules().iter().filter(|m| self.installs_native(uri, m.name)) {
            for source in module.structs {
                if let Ok(lib_tokens) = spg_lexer::Lexer::new(source).tokenize() {
                    if let Some(variants) = variants_in(&lib_tokens, name) {
                        return Some(variants);
                    }
                }
            }
        }
        None
    }

    /// (start column, end column) of the word being typed, widened by one to
    /// swallow a directly preceding `@`. that's what a namespace completion
    /// must replace, because editors don't treat `@` as part of a word
    fn namespace_edit_range(&self, uri: &str, pos: Option<(usize, usize)>) -> Option<(usize, usize, usize)> {
        let (line, col) = pos?;
        let before = self.line_before_cursor(uri, pos)?;
        let chars: Vec<char> = before.chars().collect();
        let mut start = chars.len();
        while start > 0 && (chars[start - 1].is_ascii_alphanumeric() || chars[start - 1] == '_') {
            start -= 1;
        }
        if start > 0 && chars[start - 1] == '@' {
            start -= 1;
        }
        Some((line, start, col))
    }

    /// `#> name` being typed: offers the file directives the file doesn't use
    /// yet, each replacing the partial name already written (so the `-` of
    /// `friendly-terminal` doesn't break the word). `None` when the cursor is
    /// not there
    fn directive_completion(&self, uri: &str, pos: Option<(usize, usize)>) -> Option<Vec<Json>> {
        let (line, col) = pos?;
        let before = self.line_before_cursor(uri, pos)?;
        let chars: Vec<char> = before.chars().collect();
        let hash = chars.iter().position(|c| !c.is_whitespace())?;
        if chars.get(hash) != Some(&'#') || chars.get(hash + 1) != Some(&'>') {
            return None;
        }
        let mut start = hash + 2;
        while start < chars.len() && chars[start] == ' ' {
            start += 1;
        }
        // an arg (`#> timeout 5`) or a comment is being written. no names,
        // and not the global list either (nothing of it fits on a directive line)
        if !chars[start..].iter().all(|c| c.is_ascii_alphanumeric() || *c == '-') {
            return Some(Vec::new());
        }
        let doc = self.docs.get(uri)?;
        let used: Vec<String> = doc
            .text
            .split('\n')
            .enumerate()
            .filter(|(i, _)| *i != line)
            .filter_map(|(_, l)| {
                let rest = l.trim_start().strip_prefix("#>")?;
                Some(rest.split_whitespace().next().unwrap_or("").to_string())
            })
            .collect();
        let at = |c: usize| Json::obj(vec![("line", Json::int(line as i64)), ("character", Json::int(c as i64))]);
        Some(
            DIRECTIVE_DOCS
                .iter()
                .filter(|(name, _, _)| !used.iter().any(|u| u == name))
                .map(|(name, insert, help)| {
                    Json::obj(vec![
                        ("label", Json::str(name)),
                        ("kind", Json::int(14)),
                        ("detail", Json::str(help)),
                        (
                            "textEdit",
                            Json::obj(vec![
                                ("range", Json::obj(vec![("start", at(start)), ("end", at(col))])),
                                ("newText", Json::str(insert)),
                            ]),
                        ),
                    ])
                })
                .collect(),
        )
    }

    fn completion(&self, uri: &str, pos: Option<(usize, usize)>) -> Vec<Json> {
        if let Some(items) = self.directive_completion(uri, pos) {
            return items;
        }
        if let Some(items) = self.std_lib_completion(uri, pos) {
            return items;
        }
        const KEYWORDS: &[&str] = &[
            "let", "const", "fn", "return", "if", "else", "while", "do", "for", "in", "step", "try", "catch", "throw", "break",
            "continue", "when", "elif", "guard", "use", "public", "from", "attach", "struct", "enum", "namespace", "true", "false", "none", "then", "and", "or", "not",
        ];
        const TYPES: &[&str] = &[
            "int", "i64", "i8", "i16", "i32", "u8", "u16", "u32", "u64", "float", "f64", "f32", "bool", "string", "void", "array", "map", "task",
        ];
        let edit_range = self.namespace_edit_range(uri, pos);

        let item = |label: &str, kind: i64, detail: Option<String>, doc: Option<&str>| {
            let mut f = vec![("label", Json::str(label)), ("kind", Json::int(kind))];
            if let Some(d) = detail {
                f.push(("detail", Json::Str(d)));
            }
            if let Some(d) = doc {
                f.push(("documentation", Json::str(d)));
            }
            Json::obj(f)
        };
        // a namespace entry: picking it inserts `@ns.` (replacing the `@` and
        // the partial name already typed) and immediately reopens the
        // suggestion list, which then shows the members of that namespace
        let namespace_item = |ns: &str| {
            let label = format!("@{}", ns);
            let mut f = vec![
                ("label", Json::Str(label.clone())),
                ("kind", Json::int(9)),
                ("detail", Json::Str(format!("namespace {}", label))),
                ("documentation", Json::str(spg_typecheck::docs::namespace_doc(ns).unwrap_or(""))),
                ("insertText", Json::Str(format!("{}.", label))),
                ("filterText", Json::Str(label.clone())),
                (
                    "command",
                    Json::obj(vec![
                        ("title", Json::str("Show namespace members")),
                        ("command", Json::str("editor.action.triggerSuggest")),
                    ]),
                ),
            ];
            if let Some((line, start, end)) = edit_range {
                let p = |c: usize| Json::obj(vec![("line", Json::int(line as i64)), ("character", Json::int(c as i64))]);
                f.push((
                    "textEdit",
                    Json::obj(vec![
                        ("range", Json::obj(vec![("start", p(start)), ("end", p(end))])),
                        ("newText", Json::Str(format!("{}.", label))),
                    ]),
                ));
            }
            Json::obj(f)
        };

        let member_item = |m: &crate::members::Member| {
            // fields first, then methods: the editor orders by this, then by name
            let group = if m.kind == crate::members::KIND_FIELD { "0" } else { "1" };
            let mut f = vec![
                ("label", Json::Str(m.name.clone())),
                ("kind", Json::int(m.kind)),
                ("detail", Json::Str(m.detail.clone())),
                ("sortText", Json::str(group)),
            ];
            if let Some(ins) = &m.insert {
                f.push(("insertText", Json::Str(ins.clone())));
            }
            if let Some(d) = m.doc {
                f.push(("documentation", Json::str(d)));
            }
            Json::obj(f)
        };

        let mut items: Vec<Json> = Vec::new();
        for k in KEYWORDS {
            items.push(with_sort(item(k, 14, None, None), SORT_KEYWORD));
        }
        for t in TYPES {
            items.push(with_sort(item(t, 25, None, None), SORT_TYPE));
        }

        let mut builtins: Vec<(String, String)> = spg_typecheck::builtins::public_builtin_signatures()
            .into_iter()
            .map(|(name, sigs)| {
                let detail = sigs
                    .iter()
                    .map(|s| {
                        let ps: Vec<String> = s.params.iter().map(|p| p.to_string()).collect();
                        format!("{}({}) -> {}", name, ps.join(", "), s.return_type)
                    })
                    .collect::<Vec<_>>()
                    .join("  |  ");
                (name, detail)
            })
            .collect();
        // the builtins the checker handles by name are not in the table, but they exist
        builtins.extend(
            spg_typecheck::builtins::checker_builtins()
                .into_iter()
                .map(|(name, detail)| (name.to_string(), detail.to_string())),
        );
        builtins.sort();

        // right after a `.`: only namespace members make sense. after
        // `@math.` (or `@time.`, ...) offer that namespace's members, inserted
        // without the prefix already typed. after any other dot (a struct
        // field, a float literal, a string), offer nothing instead of
        // dumping the global list. while the name after an `@` is being
        // typed, only the namespaces themselves are offered.
        match self.dot_context(uri, pos) {
            DotContext::TypeProp(ty) => {
                // `int::`: its constants and `methods`, with the value printed as detail
                return spg_typecheck::docs::type_properties(&ty)
                    .into_iter()
                    .map(|(name, value, doc)| item(name, 21, Some(value.to_string()), Some(doc)))
                    .collect();
            }
            DotContext::NamespaceName => {
                return spg_lexer::namespaces().iter().map(|ns| namespace_item(ns)).collect();
            }
            DotContext::TypeMethods(ty) => {
                // `int::methods.`: `add` inserts a whole declaration, with the cursor in the name
                let snippet = format!("add(fn ${{1:nome}}() -> {} {{\n\t$0\n}})", ty);
                return vec![Json::obj(vec![
                    ("label", Json::str("add")),
                    ("kind", Json::int(2)),
                    ("detail", Json::Str(format!("add(fn nome(...) -> R {{ ... }}): a method of {}", ty))),
                    (
                        "documentation",
                        Json::Str(format!(
                            "declares a method of {ty} for every {ty} value: `me` is the value it is called on. only at the top level of the file, and a private method stays in its own file unless it is `public`ed",
                            ty = ty
                        )),
                    ),
                    ("insertText", Json::Str(snippet)),
                    ("insertTextFormat", Json::int(2)),
                ])];
            }
            DotContext::Namespace(ns) => {
                let prefix = format!("@{}.", ns);
                return builtins
                    .into_iter()
                    .filter_map(|(name, detail)| {
                        let member = name.strip_prefix(&prefix)?.to_string();
                        let doc = spg_typecheck::docs::builtin_doc(&name);
                        Some(item(&member, 3, Some(detail), doc))
                    })
                    .collect();
            }
            DotContext::Native(ns) => {
                let prefix = format!("@{}.", ns);
                return builtins
                    .into_iter()
                    .filter_map(|(name, detail)| {
                        let member = name.strip_prefix(&prefix)?.to_string();
                        // `_name` functions are the internals of native objects (`win.width`): not for the program
                        if member.starts_with('_') {
                            return None;
                        }
                        let doc = spg_typecheck::docs::builtin_doc(&name);
                        Some(item(&member, 3, Some(detail), doc))
                    })
                    .collect();
            }
            DotContext::Enum(name) => {
                // `Color.`: its variants, then the functions declared as `fn Color.name(...)`
                let mut items: Vec<Json> = self
                    .enum_variants(uri, &name)
                    .unwrap_or_default()
                    .iter()
                    .map(|v| item(v, 20, Some(format!("{}.{}", name, v)), None))
                    .collect();
                if let Some((_, _, decls)) = self.member_context(uri, pos) {
                    items.extend(crate::members::associated_of(&decls, &name).iter().map(member_item));
                }
                return items;
            }
            DotContext::Other => {
                // `p.` / `a.pos.`: the fields and methods of the struct `p` was
                // declared as (or `value` for an enum), found on the tokens
                // a name chain (`p.`, `a.pos.`) is resolved on the names; anything else
                // (`"text".`, `42.`, `f().`, `xs[0].`, `Point { }.`) on the tokens
                let Some((tokens, at, decls)) = self.member_context(uri, pos) else {
                    return Vec::new();
                };
                let members = match self.receiver_chain(uri, pos) {
                    Some(chain) => crate::members::after_dot(&tokens, at, &chain, &decls),
                    None => crate::members::after_dot_expr(&tokens, at, &decls),
                };
                return members.iter().map(member_item).collect();
            }
            DotContext::None => {
                // `Point { `: the fields of the literal that aren't written yet
                if let Some((tokens, at, decls)) = self.member_context(uri, pos) {
                    if let Some(fields) = crate::members::literal_fields(&tokens, at, &decls) {
                        return fields.iter().map(member_item).collect();
                    }
                }
            }
        }

        // `me` is the struct a method is called on: offered inside a struct body, with the names of this file
        if let Some((tokens, at, _)) = self.member_context(uri, pos) {
            if inside_struct_body(&tokens, at) || crate::members::value_method_body_type(&tokens, at).is_some() {
                let detail = if inside_struct_body(&tokens, at) {
                    "the struct this method is called on"
                } else {
                    "the value this method is called on"
                };
                items.push(with_sort(item("me", 6, Some(detail.to_string()), None), SORT_OWN));
            }
        }

        for (name, detail) in builtins {
            // namespaced members only show up after `ns.`. globally we
            // offer just the un-namespaced builtins (print, len, ...)
            if !name.contains('.') {
                items.push(with_sort(item(&name, 3, Some(detail), spg_typecheck::docs::builtin_doc(&name)), SORT_BUILTIN));
            }
        }
        // the namespaces themselves
        for ns in spg_lexer::namespaces() {
            items.push(with_sort(namespace_item(ns), SORT_NAMESPACE));
        }

        // names the user declared in this document, found by lexing it
        // (a lex error just means we offer no user symbols)
        if let Some(doc) = self.docs.get(uri) {
            if let Ok(tokens) = spg_lexer::Lexer::new(&doc.text).tokenize() {
                use spg_lexer::TokenKind as T;
                let mut seen = std::collections::HashSet::new();
                // the signature of each callable, shown as the completion detail
                let labels: HashMap<String, String> = crate::signature::label_by_name(&tokens).into_iter().collect();
                // a `function` inside a `struct { .. }` body is a METHOD. it is
                // only callable as `value.name()`, so it must not be offered
                // as a free name. track brace depth to know when we're in one.
                let mut depth: i64 = 0;
                let mut struct_body: Option<i64> = None;
                for (i, w) in tokens.windows(2).enumerate() {
                    match w[0].kind {
                        T::LBrace => {
                            depth += 1;
                            let opens_struct = i >= 2
                                && matches!(tokens[i - 1].kind, T::Ident(_))
                                && tokens[i - 2].kind == T::Struct;
                            if opens_struct && struct_body.is_none() {
                                struct_body = Some(depth);
                            }
                        }
                        T::RBrace => {
                            if struct_body == Some(depth) {
                                struct_body = None;
                            }
                            depth -= 1;
                        }
                        _ => {}
                    }
                    let kind = match w[0].kind {
                        T::Function if struct_body == Some(depth) => continue,
                        T::Function => 3,
                        T::Struct => 22,
                        T::Enum => 13,
                        T::Let => 6,
                        T::Const => 21,
                        _ => continue,
                    };
                    if let T::Ident(name) = &w[1].kind {
                        if seen.insert(name.clone()) {
                            items.push(with_sort(item(name, kind, labels.get(name).cloned(), None), SORT_OWN));
                        }
                    }
                }

                // names that come from OTHER files: `use (a, b) from "x"`
                // offers exactly a and b. `attach "x"` offers every export of x.
                // read from the files' own tokens, so it works while the buffer
                // is half-typed (no full resolve needed).
                let dir = doc.path.as_ref().and_then(|p| p.parent().map(|d| d.to_path_buf()));
                // both forms: `use (a, b) from "x"` and `from "x" use (a, b)`
                for (names, path) in crate::navigate::use_clauses(&tokens) {
                    let from = Some(path);
                    let known = match (&dir, &from) {
                        (Some(d), Some(f)) => self.exports_of(&d.join(f)),
                        _ => Vec::new(),
                    };
                    for n in names {
                        if seen.insert(n.clone()) {
                            let kind = known.iter().find(|(k, _)| *k == n).map(|(_, k)| *k).unwrap_or(3);
                            items.push(with_sort(item(&n, kind, from.as_ref().map(|f| format!("from {}", f)), None), SORT_IMPORTED));
                        }
                    }
                }
                let mut i = 0;
                while i < tokens.len() {
                    match &tokens[i].kind {
                        T::Attach => {
                            // `attach #name`: a native library (ps, http) is offered by its
                            // own name, and a standard module offers what it exports (`terminal`)
                            if let Some(T::StdLib(lib)) = tokens.get(i + 1).map(|t| &t.kind) {
                                if spg_lexer::is_install_namespace(lib) {
                                    if seen.insert(lib.clone()) {
                                        items.push(item(lib, 9, Some("installed library".to_string()), None));
                                    }
                                    // its types and enums (`Align`, `Key`, `Window`) are names of the program too
                                    for (n, kind) in native_type_names(lib) {
                                        if seen.insert(n.clone()) {
                                            items.push(with_sort(item(&n, kind, Some(format!("from attach #{}", lib)), None), SORT_IMPORTED));
                                        }
                                    }
                                } else if let Ok(file) = spg_modules::standard_module(lib) {
                                    for (n, kind) in self.exports_of(&file) {
                                        if seen.insert(n.clone()) {
                                            items.push(with_sort(item(&n, kind, Some(format!("installed from {}", lib)), None), SORT_IMPORTED));
                                        }
                                    }
                                }
                            }
                            if let (Some(d), Some(T::Str(f))) = (&dir, tokens.get(i + 1).map(|t| &t.kind)) {
                                for (n, kind) in self.exports_of(&d.join(f)) {
                                    if seen.insert(n.clone()) {
                                        items.push(with_sort(item(&n, kind, Some(format!("installed from {}", f)), None), SORT_IMPORTED));
                                    }
                                }
                            }
                        }
                        _ => {}
                    }
                    i += 1;
                }

                // params of every function and lambda in the file. offered
                // everywhere in the file (like the other names here), typed
                for (name, ty) in crate::signature::all_params(&tokens) {
                    if seen.insert(name.clone()) {
                        items.push(item(&name, 6, Some(format!("parameter: {}", ty)), None));
                    }
                }
            }
        }
        items
    }

    /// the name under the cursor, resolved to what it refers to: the token
    /// list, the declaration (in this file, or in the file it's imported from)
    fn resolve_at(&self, uri: &str, pos: Option<(usize, usize)>) -> Option<(crate::navigate::Target, Vec<spg_lexer::Token>)> {
        let (line, col) = pos?;
        let doc = self.docs.get(uri)?;
        let tokens = spg_lexer::Lexer::new(&doc.text).tokenize().ok()?;
        let at = crate::navigate::word_at(&tokens, line, col)?;
        let decls = crate::navigate::declarations(&tokens);
        let target = crate::navigate::target(&tokens, &decls, at)?;
        Some((target, tokens))
    }

    /// the declaration a target points at, and the file that holds it
    fn locate(&self, uri: &str, target: &crate::navigate::Target) -> Option<(String, crate::navigate::Decl)> {
        use crate::navigate::{declarations, Target};
        match target {
            Target::Local(d) => Some((uri.to_string(), d.clone())),
            Target::Imported { name, file } => {
                let dir = self.docs.get(uri)?.path.as_ref()?.parent()?.to_path_buf();
                // `./util.spg` joins as `dir/./util.spg`. drop the `.` so the URI is clean
                let path: PathBuf = dir.join(file).components().filter(|c| !matches!(c, std::path::Component::CurDir)).collect();
                let text = self.source_of(&path)?;
                let tokens = spg_lexer::Lexer::new(&text).tokenize().ok()?;
                let found = declarations(&tokens)
                    .into_iter()
                    .find(|d| d.name == *name && d.owner.is_none() && matches!(d.kind, crate::navigate::Kind::Function | crate::navigate::Kind::Struct | crate::navigate::Kind::Enum | crate::navigate::Kind::Let))?;
                Some((crate::uri::path_to_uri(&path), found))
            }
        }
    }

    /// `textDocument/formatting`: one edit that replaces the whole document with
    /// its canonical layout, or none when it's already formatted or `spg_fmt`
    /// refuses it (the file doesn't lex, or the result failed its safety proofs)
    fn formatting(&self, uri: &str) -> Option<Json> {
        let text = &self.docs.get(uri)?.text;
        let formatted = spg_fmt::check(text).ok()?;
        if formatted == *text {
            return Some(Json::Arr(Vec::new()));
        }
        // the range ends where the document does. LSP counts UTF-16 units
        let last_line = text.split('\n').count() - 1;
        let end_character: usize = text.split('\n').last().unwrap_or("").chars().map(char::len_utf16).sum();
        let point = |line: usize, character: usize| Json::obj(vec![("line", Json::int(line as i64)), ("character", Json::int(character as i64))]);
        Some(Json::Arr(vec![Json::obj(vec![
            ("range", Json::obj(vec![("start", point(0, 0)), ("end", point(last_line, end_character))])),
            ("newText", Json::Str(formatted)),
        ])]))
    }

    fn hover(&self, uri: &str, pos: Option<(usize, usize)>) -> Option<Json> {
        // a declaration of this program (or an import) wins; otherwise a builtin
        // (`@math.sqrt`) or a keyword (`then`) explains itself
        let value = match self.resolve_at(uri, pos).and_then(|(target, _)| self.locate(uri, &target)) {
            Some((_, decl)) => format!("```spg\n{}\n```", crate::navigate::describe(&decl)),
            None => self.builtin_or_keyword_hover(uri, pos)?,
        };
        Some(Json::obj(vec![(
            "contents",
            Json::obj(vec![("kind", Json::str("markdown")), ("value", Json::Str(value))]),
        )]))
    }

    /// the markdown for the word under the cursor when it is a builtin (`@math.sqrt`,
    /// looked up with its namespace) or a keyword. None for anything else
    fn builtin_or_keyword_hover(&self, uri: &str, pos: Option<(usize, usize)>) -> Option<String> {
        let (line, col) = pos?;
        let text_line = self.docs.get(uri)?.text.split('\n').nth(line)?.trim_end_matches('\r').to_string();
        let chars: Vec<char> = text_line.chars().collect();
        let is_word = |c: char| c.is_ascii_alphanumeric() || c == '_' || c == '@';
        let col = col.min(chars.len());
        let mut start = col;
        while start > 0 && is_word(chars[start - 1]) {
            start -= 1;
        }
        let mut end = col;
        while end < chars.len() && is_word(chars[end]) {
            end += 1;
        }
        if start == end {
            return None;
        }
        let word: String = chars[start..end].iter().collect();
        // `@math.sqrt`: the namespace before the dot is part of the name
        let name = if start >= 2 && chars[start - 1] == '.' {
            let mut ns_start = start - 1;
            while ns_start > 0 && is_word(chars[ns_start - 1]) {
                ns_start -= 1;
            }
            let ns: String = chars[ns_start..start - 1].iter().collect();
            format!("{}.{}", ns, word)
        } else {
            word.clone()
        };
        if let Some(doc) = spg_typecheck::docs::builtin_doc(&name) {
            return Some(format!("```spg\n{}\n```\n\n{}", name, doc));
        }
        keyword_doc(&word).map(|doc| format!("```spg\n{}\n```\n\n{}", word, doc))
    }

    fn definition(&self, uri: &str, pos: Option<(usize, usize)>) -> Option<Json> {
        let (target, _) = self.resolve_at(uri, pos)?;
        let (file_uri, decl) = self.locate(uri, &target)?;
        // the name's span. LSP counts UTF-16 units, so convert the lexer's columns
        let line0 = decl.line.saturating_sub(1);
        let text = if file_uri == uri {
            self.docs.get(uri)?.text.clone()
        } else {
            let path = crate::uri::uri_to_path(&file_uri)?;
            self.source_of(&path)?
        };
        let line_text = text.split('\n').nth(line0)?;
        let utf16 = |chars: usize| -> usize { line_text.chars().take(chars).map(char::len_utf16).sum() };
        let start = utf16(decl.col.saturating_sub(1));
        let end = utf16(decl.col.saturating_sub(1) + decl.name.chars().count());
        let point = |line: usize, character: usize| Json::obj(vec![("line", Json::int(line as i64)), ("character", Json::int(character as i64))]);
        Some(Json::obj(vec![
            ("uri", Json::str(&file_uri)),
            ("range", Json::obj(vec![("start", point(line0, start)), ("end", point(line0, end))])),
        ]))
    }

    /// `textDocument/signatureHelp`: the call around the cursor, its signature,
    /// and which arg the cursor is on
    fn signature_help(&self, uri: &str, pos: Option<(usize, usize)>) -> Option<Json> {
        let (tokens, at, _) = self.member_context(uri, pos)?;
        // `recv.f(`: the method of the receiver's struct wins over a free name `f`
        let decls = crate::navigate::declarations(&tokens);
        let (sig, active) = crate::navigate::method_signature(&tokens, &decls, at)
            .or_else(|| crate::signature::signature_help(&tokens, at))?;
        let parameters: Vec<Json> = sig
            .params
            .iter()
            .map(|(name, ty)| {
                let label = if name.is_empty() { ty.clone() } else { format!("{}: {}", name, ty) };
                Json::obj(vec![("label", Json::Str(label))])
            })
            .collect();
        Some(Json::obj(vec![
            (
                "signatures",
                Json::Arr(vec![Json::obj(vec![
                    ("label", Json::Str(sig.label())),
                    ("parameters", Json::Arr(parameters)),
                ])]),
            ),
            ("activeSignature", Json::int(0)),
            ("activeParameter", Json::int(active as i64)),
        ]))
    }

    /// the text of a source file: an open buffer wins over the disk
    fn source_of(&self, path: &std::path::Path) -> Option<String> {
        let canonical = spg_modules::canon(path).ok()?;
        self.docs
            .values()
            .find(|d| d.path.as_ref() == Some(&canonical))
            .map(|d| d.text.clone())
            .or_else(|| std::fs::read_to_string(&canonical).ok())
    }

    /// the dotted names right before a `.` being completed: `p.` -> [p],
    /// `a.pos.na` -> [a, pos]. `None` when the receiver isn't a plain name
    /// chain (a call, an index, a number, a namespace).
    fn receiver_chain(&self, uri: &str, pos: Option<(usize, usize)>) -> Option<Vec<String>> {
        let before = self.line_before_cursor(uri, pos)?;
        let word_free = before.trim_end_matches(|c: char| c.is_ascii_alphanumeric() || c == '_');
        let chars: Vec<char> = word_free.strip_suffix('.')?.chars().collect();
        let mut start = chars.len();
        while start > 0 && (chars[start - 1].is_ascii_alphanumeric() || chars[start - 1] == '_' || chars[start - 1] == '.') {
            start -= 1;
        }
        if start > 0 && matches!(chars[start - 1], ')' | ']' | '@') {
            return None;
        }
        let chain: String = chars[start..].iter().collect();
        let parts: Vec<String> = chain.split('.').map(String::from).collect();
        let name_like = |p: &String| p.chars().next().map_or(false, |c| c.is_ascii_alphabetic() || c == '_');
        if parts.iter().all(name_like) { Some(parts) } else { None }
    }

    /// what the token-based member completion needs: the tokens of this
    /// document, how many of them come before the word being typed, and the
    /// struct/enum/function declarations of this file plus the files it
    /// imports or installs
    fn member_context(
        &self,
        uri: &str,
        pos: Option<(usize, usize)>,
    ) -> Option<(Vec<spg_lexer::Token>, usize, crate::members::Decls)> {
        use spg_lexer::TokenKind as T;
        let (line, col) = pos?;
        let doc = self.docs.get(uri)?;
        let tokens = spg_lexer::Lexer::new(&doc.text).tokenize().ok()?;
        let before = self.line_before_cursor(uri, pos)?;
        let word_len = before.chars().rev().take_while(|c| c.is_ascii_alphanumeric() || *c == '_').count();
        let word_start = col.saturating_sub(word_len);
        let at = tokens.iter().filter(|t| (t.line.saturating_sub(1), t.col.saturating_sub(1)) < (line, word_start)).count();
        let mut decls = crate::members::Decls::from_tokens(&tokens);
        if let Some(dir) = doc.path.as_ref().and_then(|p| p.parent().map(|d| d.to_path_buf())) {
            for (i, t) in tokens.iter().enumerate() {
                let file = match &t.kind {
                    // `attach "path"`: relative to this file
                    T::Attach => match tokens.get(i + 1).map(|t| &t.kind) {
                        Some(T::Str(f)) => Some(f.clone()),
                        // `attach #name`: a standard module, the same
                        // file the compiler resolves (see spg_modules::standard_module)
                        Some(T::StdLib(name)) => spg_modules::standard_module(name).ok().map(|p| p.to_string_lossy().into_owned()),
                        _ => None,
                    },
                    _ => None,
                };
                if let Some(f) = file {
                    if let Some(text) = self.source_of(&dir.join(f)) {
                        if let Ok(other) = spg_lexer::Lexer::new(&text).tokenize() {
                            decls.merge(crate::members::Decls::from_tokens(&other));
                        }
                    }
                }
            }
            for (_, path) in crate::navigate::use_clauses(&tokens) {
                if let Some(text) = self.source_of(&dir.join(path)) {
                    if let Ok(other) = spg_lexer::Lexer::new(&text).tokenize() {
                        decls.merge(crate::members::Decls::from_tokens(&other));
                    }
                }
            }
        }
        Some((tokens, at, decls))
    }

    /// every name `path` exports, with its completion kind: `export function/
    /// struct/enum/let/const NAME` and `public (a, b) from ".."` (kind
    /// unknown there, so function). an open buffer wins over the disk.
    fn exports_of(&self, path: &std::path::Path) -> Vec<(String, i64)> {
        use spg_lexer::TokenKind as T;
        let Some(text) = self.source_of(path) else { return Vec::new() };
        let Ok(tokens) = spg_lexer::Lexer::new(&text).tokenize() else { return Vec::new() };
        let mut out = Vec::new();
        for (i, t) in tokens.iter().enumerate() {
            if t.kind != T::Public {
                continue;
            }
            let kind = match tokens.get(i + 1).map(|t| &t.kind) {
                Some(T::Function) => 3,
                Some(T::Struct) => 22,
                Some(T::Enum) => 13,
                Some(T::Let) => 6,
                Some(T::Const) => 21,
                // `public (a, b) from "x"` or `public a, b from "x"`: a re-export list
                Some(T::LParen) | Some(T::Ident(_)) => {
                    if let Some((names, end)) = crate::navigate::name_list(&tokens, i + 1) {
                        if tokens.get(end).map(|t| &t.kind) == Some(&T::From) {
                            out.extend(names.into_iter().map(|n| (n, 3)));
                        }
                    }
                    continue;
                }
                _ => continue,
            };
            if let Some(T::Ident(n)) = tokens.get(i + 2).map(|t| &t.kind) {
                out.push((n.clone(), kind));
            }
        }
        out
    }
}

/// what precedes the cursor in a completion request (see `dot_context`)
enum DotContext {
    None,
    /// right after `@` (plus any partial name): a namespace is being typed
    NamespaceName,
    Namespace(String),
    /// right after `int::` or `float::` (any name before `::`): a type's property
    TypeProp(String),
    /// right after `int::methods.` (any name before `::`): the only member is `add`
    TypeMethods(String),
    /// right after `Name.` where `Name` is an enum declared in the document
    Enum(String),
    /// right after `ps.` in a document that runs `attach #powershell`
    Native(String),
    Other,
}

/// the variants of `enum <name> { .. }` in a token stream (`None` when it isn't declared there)
/// the types a native library declares in its text (`struct Window`, `enum Key`, `enum Align`),
/// with their completion kinds: 22 for a struct, 13 for an enum
fn native_type_names(lib: &str) -> Vec<(String, i64)> {
    use spg_lexer::TokenKind as T;
    let mut out = Vec::new();
    let Some(module) = spg_libs::module(lib) else { return out };
    for source in module.structs {
        let Ok(tokens) = spg_lexer::Lexer::new(source).tokenize() else { continue };
        for w in tokens.windows(2) {
            let kind = match w[0].kind {
                T::Struct => 22,
                T::Enum => 13,
                _ => continue,
            };
            if let T::Ident(name) = &w[1].kind {
                out.push((name.clone(), kind));
            }
        }
    }
    out
}

fn variants_in(tokens: &[spg_lexer::Token], name: &str) -> Option<Vec<String>> {
    use spg_lexer::TokenKind as T;
    for (i, w) in tokens.windows(3).enumerate() {
        if w[0].kind == T::Enum && w[1].kind == T::Ident(name.to_string()) && w[2].kind == T::LBrace {
            let mut out = Vec::new();
            for t in &tokens[i + 3..] {
                match &t.kind {
                    T::Ident(v) => out.push(v.clone()),
                    T::Comma | T::Newline => {}
                    _ => break,
                }
            }
            return Some(out);
        }
    }
    None
}

/// true when some line of the buffer is exactly the `// [ShowErrors-Off]`
/// marker (surrounding spaces ignored, case ignored). whole-line only, so the
/// text can't trigger it by accident inside a string.
fn hides_errors(text: &str) -> bool {
    text.lines().any(|l| l.trim().eq_ignore_ascii_case("// [ShowErrors-Off]"))
}

/// the sort group of a global completion item. the editor orders items by this, then by name, so
/// the names of the file come first, then the imports, the builtins, the namespaces, the keywords
/// and the types
const SORT_OWN: &str = "0";
const SORT_IMPORTED: &str = "1";
const SORT_BUILTIN: &str = "2";
const SORT_NAMESPACE: &str = "3";
const SORT_KEYWORD: &str = "4";
const SORT_TYPE: &str = "5";

fn with_sort(item: Json, group: &str) -> Json {
    match item {
        Json::Obj(mut pairs) => {
            pairs.push(("sortText".to_string(), Json::str(group)));
            Json::Obj(pairs)
        }
        other => other,
    }
}

/// true when the token at `at` sits inside a `struct { .. }` body: the braces before it are
/// tracked, and the one that opened a struct counts
fn inside_struct_body(tokens: &[spg_lexer::Token], at: usize) -> bool {
    use spg_lexer::TokenKind as T;
    let mut stack: Vec<bool> = Vec::new();
    for (i, t) in tokens.iter().enumerate().take(at) {
        match t.kind {
            T::LBrace => {
                let opens_struct = i >= 2 && matches!(tokens[i - 1].kind, T::Ident(_)) && tokens[i - 2].kind == T::Struct;
                stack.push(opens_struct);
            }
            T::RBrace => {
                stack.pop();
            }
            _ => {}
        }
    }
    stack.iter().any(|&s| s)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    fn frame(v: &Json) -> String {
        let s = v.to_string();
        format!("Content-Length: {}\r\n\r\n{}", s.len(), s)
    }

    /// drives the server with `msgs`, returns every JSON message it wrote
    fn session(msgs: Vec<Json>) -> (i32, Vec<Json>) {
        let input: String = msgs.iter().map(frame).collect();
        let mut out: Vec<u8> = Vec::new();
        let code = run(Cursor::new(input.into_bytes()), &mut out);
        let text = String::from_utf8(out).unwrap();
        let mut replies = Vec::new();
        let mut rest = text.as_str();
        while let Some(i) = rest.find("\r\n\r\n") {
            let len: usize = rest[..i].trim_start_matches("Content-Length:").trim().parse().unwrap();
            let body = &rest[i + 4..i + 4 + len];
            replies.push(Json::parse(body).unwrap());
            rest = &rest[i + 4 + len..];
        }
        (code, replies)
    }

    fn req(id: i64, method: &str, params: Json) -> Json {
        Json::obj(vec![("jsonrpc", Json::str("2.0")), ("id", Json::int(id)), ("method", Json::str(method)), ("params", params)])
    }
    fn note(method: &str, params: Json) -> Json {
        Json::obj(vec![("jsonrpc", Json::str("2.0")), ("method", Json::str(method)), ("params", params)])
    }

    #[test]
    fn handshake_and_clean_exit() {
        let (code, out) = session(vec![
            req(1, "initialize", Json::obj(vec![])),
            req(2, "shutdown", Json::Null),
            note("exit", Json::Null),
        ]);
        assert_eq!(code, 0);
        assert_eq!(out[0].get("id").unwrap().as_f64(), Some(1.0));
        assert!(out[0].get("result").unwrap().get("capabilities").is_some());
    }

    #[test]
    fn exit_without_shutdown_is_error_code() {
        let (code, _) = session(vec![note("exit", Json::Null)]);
        assert_eq!(code, 1);
    }

    #[test]
    fn unknown_request_gets_method_not_found() {
        let (_, out) = session(vec![req(7, "nope/nada", Json::Null), note("exit", Json::Null)]);
        assert_eq!(out[0].get("error").unwrap().get("code").unwrap().as_f64(), Some(-32601.0));
    }

    fn tmp_file(name: &str, body: &str) -> (PathBuf, String) {
        let dir = std::env::temp_dir().join(format!("spg-lsp-test-{}-{}", std::process::id(), name));
        std::fs::create_dir_all(&dir).unwrap();
        let f = dir.join("main.spg");
        std::fs::write(&f, body).unwrap();
        let u = uri::path_to_uri(&f);
        (f, u)
    }

    fn diagnostics_of(out: &[Json], uri: &str) -> Vec<Json> {
        // last publish for that uri wins
        out.iter()
            .filter(|m| m.get("method").and_then(Json::as_str) == Some("textDocument/publishDiagnostics"))
            .filter(|m| m.get("params").unwrap().get("uri").unwrap().as_str() == Some(uri))
            .last()
            .unwrap()
            .get("params").unwrap().get("diagnostics").unwrap().as_array().unwrap().clone()
    }

    #[test]
    fn reports_type_error_with_exact_range_using_unsaved_text() {
        // disk is fine. the BUFFER has the error -> overlay must win
        let (_f, uri) = tmp_file("typeerr", "let x = 1\n");
        let buffer = "let ação: int = 1\nlet y: int = \"texto\"\n";
        let (_, out) = session(vec![
            req(1, "initialize", Json::obj(vec![])),
            note("textDocument/didOpen", Json::obj(vec![(
                "textDocument", Json::obj(vec![("uri", Json::str(&uri)), ("text", Json::str(buffer)), ("version", Json::int(3))]))])),
            note("exit", Json::Null),
        ]);
        let ds = diagnostics_of(&out, &uri);
        assert_eq!(ds.len(), 1, "exactly one diagnostic, got {:?}", ds.iter().map(|d| d.to_string()).collect::<Vec<_>>());
        let d = &ds[0];
        assert_eq!(d.get("severity").unwrap().as_f64(), Some(1.0));
        assert!(d.get("message").unwrap().as_str().unwrap().contains("declared type is int but value is string"));
        let start = d.get("range").unwrap().get("start").unwrap();
        assert_eq!(start.get("line").unwrap().as_f64(), Some(1.0)); // second line, 0-based
    }

    #[test]
    fn clean_buffer_clears_and_close_publishes_empty() {
        let (_f, uri) = tmp_file("clean", "let x: int = \"bad\"\n");
        let open = |text: &str| note("textDocument/didOpen", Json::obj(vec![(
            "textDocument", Json::obj(vec![("uri", Json::str(&uri)), ("text", Json::str(text)), ("version", Json::int(1))]))]));
        let change = note("textDocument/didChange", Json::obj(vec![
            ("textDocument", Json::obj(vec![("uri", Json::str(&uri)), ("version", Json::int(2))])),
            ("contentChanges", Json::Arr(vec![Json::obj(vec![("text", Json::str("let x: int = 5\n"))])])),
        ]));
        let close = note("textDocument/didClose", Json::obj(vec![("textDocument", Json::obj(vec![("uri", Json::str(&uri))]))]));
        let (_, out) = session(vec![open("let x: int = \"bad\"\n"), change, close, note("exit", Json::Null)]);
        let pubs: Vec<usize> = out.iter()
            .filter(|m| m.get("method").and_then(Json::as_str) == Some("textDocument/publishDiagnostics"))
            .map(|m| m.get("params").unwrap().get("diagnostics").unwrap().as_array().unwrap().len())
            .collect();
        assert_eq!(pubs, vec![1, 0, 0], "error, then fixed, then cleared on close");
    }

    #[test]
    fn show_errors_off_marker_hides_diagnostics() {
        let bad = "let a: int = \"oops\"\n";
        let (_f, uri) = tmp_file("marker", bad);
        let buffer = format!("// [showerrors-off]\n{}", bad);
        let (_, out) = session(vec![
            note("textDocument/didOpen", Json::obj(vec![(
                "textDocument", Json::obj(vec![("uri", Json::str(&uri)), ("text", Json::str(&buffer)), ("version", Json::int(1))]))])),
            note("exit", Json::Null),
        ]);
        assert!(diagnostics_of(&out, &uri).is_empty(), "marker must hide every error");
        assert!(hides_errors("x\n  // [ShowErrors-Off]  \ny"));
        assert!(!hides_errors("let s = \"// [ShowErrors-Off]\""));
    }

    #[test]
    fn syntax_error_is_positioned() {
        let (_f, uri) = tmp_file("syntax", "let x = 1\n");
        let (_, out) = session(vec![
            note("textDocument/didOpen", Json::obj(vec![(
                "textDocument", Json::obj(vec![("uri", Json::str(&uri)), ("text", Json::str("let a = 1\nlet b = = 2\n")), ("version", Json::int(1))]))])),
            note("exit", Json::Null),
        ]);
        let ds = diagnostics_of(&out, &uri);
        assert_eq!(ds.len(), 1);
        assert_eq!(ds[0].get("range").unwrap().get("start").unwrap().get("line").unwrap().as_f64(), Some(1.0));
    }

    #[test]
    fn completion_offers_keywords_builtins_and_user_symbols() {
        let (_f, uri) = tmp_file("compl", "let x = 1\n");
        let (_, out) = session(vec![
            note("textDocument/didOpen", Json::obj(vec![(
                "textDocument", Json::obj(vec![("uri", Json::str(&uri)), ("text", Json::str("fn somaTudo(a: int) -> int { return a }\n")), ("version", Json::int(1))]))])),
            req(9, "textDocument/completion", Json::obj(vec![("textDocument", Json::obj(vec![("uri", Json::str(&uri))]))])),
            note("exit", Json::Null),
        ]);
        let resp = out.iter().find(|m| m.get("id").and_then(Json::as_f64) == Some(9.0)).unwrap();
        let labels: Vec<&str> = resp.get("result").unwrap().as_array().unwrap().iter()
            .filter_map(|i| i.get("label").and_then(Json::as_str)).collect();
        for want in ["fn", "int", "print", "somaTudo"] {
            assert!(labels.contains(&want), "missing {} in completion", want);
        }
    }

    #[test]
    fn struct_methods_are_not_offered_as_free_names() {
        let (_f, uri) = tmp_file("compl_methods", "let x = 1\n");
        let text = "struct Point {\n    x: int\n\n    fn secretMethod() -> int { return me.x }\n}\nfn freeFn() -> int { return 1 }\n";
        let (_, out) = session(vec![
            note("textDocument/didOpen", Json::obj(vec![(
                "textDocument", Json::obj(vec![("uri", Json::str(&uri)), ("text", Json::str(text)), ("version", Json::int(1))]))])),
            req(9, "textDocument/completion", Json::obj(vec![("textDocument", Json::obj(vec![("uri", Json::str(&uri))]))])),
            note("exit", Json::Null),
        ]);
        let resp = out.iter().find(|m| m.get("id").and_then(Json::as_f64) == Some(9.0)).unwrap();
        let labels: Vec<&str> = resp.get("result").unwrap().as_array().unwrap().iter()
            .filter_map(|i| i.get("label").and_then(Json::as_str)).collect();
        assert!(labels.contains(&"Point") && labels.contains(&"freeFn"), "real declarations must still be offered");
        assert!(!labels.contains(&"secretMethod"), "a method leaked into the free-name list");
    }

    #[test]
    fn hover_explains_builtins_and_keywords() {
        let (_f, uri) = tmp_file("hover_builtin", "let x = 1\n");
        let text = "let r = @math.sqrt(4.0)\nlet t = 1 then\n";
        let at = |id: i64, line: i64, character: i64| {
            req(id, "textDocument/hover", Json::obj(vec![
                ("textDocument", Json::obj(vec![("uri", Json::str(&uri))])),
                ("position", Json::obj(vec![("line", Json::int(line)), ("character", Json::int(character))])),
            ]))
        };
        let (_, out) = session(vec![
            note("textDocument/didOpen", Json::obj(vec![(
                "textDocument", Json::obj(vec![("uri", Json::str(&uri)), ("text", Json::str(text)), ("version", Json::int(1))]))])),
            at(9, 0, 16),
            at(10, 1, 11),
            note("exit", Json::Null),
        ]);
        let value = |id: f64| -> String {
            let resp = out.iter().find(|m| m.get("id").and_then(Json::as_f64) == Some(id)).unwrap();
            resp.get("result").unwrap().get("contents").unwrap().get("value").unwrap().as_str().unwrap().to_string()
        };
        assert!(value(9.0).contains("square root"), "builtin hover: {}", value(9.0));
        assert!(value(10.0).contains("pipe"), "keyword hover: {}", value(10.0));
    }

    #[test]
    fn completion_after_enum_dot_lists_variants_and_associated_functions() {
        let text = "enum Cor { Vermelho, Verde }\nfn Cor.padrao() -> Cor {\n    return Cor.Verde\n}\nlet c = Cor.\n";
        let items = complete_at("compl_enum", text, 4, 12);
        let labels: Vec<String> = items.iter().filter_map(|i| i.get("label").and_then(Json::as_str).map(String::from)).collect();
        assert!(labels.iter().any(|l| l == "Verde"), "variants missing: {:?}", labels);
        assert!(labels.iter().any(|l| l == "padrao"), "associated function missing: {:?}", labels);
    }

    #[test]
    fn type_properties_offer_bits_and_array_methods() {
        let int_items = complete_at("compl_int_bits", "let x = int::\n", 0, 13);
        assert!(labels_of(&int_items).iter().any(|l| l == "bits"), "int:: lacks bits: {:?}", labels_of(&int_items));
        let arr_items = complete_at("compl_array_prop", "let a = array::\n", 0, 15);
        assert!(labels_of(&arr_items).iter().any(|l| l == "methods"), "array:: lacks methods: {:?}", labels_of(&arr_items));
    }

    #[test]
    fn methods_after_a_type_dot_offers_add_as_a_snippet() {
        let items = complete_at("compl_methods_add", "int::methods.\n", 0, 13);
        let add = items.iter().find(|i| i.get("label").and_then(Json::as_str) == Some("add"))
            .expect("int::methods. should offer add");
        assert_eq!(add.get("insertTextFormat").and_then(Json::as_f64), Some(2.0), "add is a snippet");
        let text = add.get("insertText").and_then(Json::as_str).unwrap();
        assert!(text.starts_with("add(fn ${1:nome}() -> int {"), "snippet opens the declaration: {}", text);
    }

    #[test]
    fn completion_after_namespace_dot_lists_only_members() {
        let (_f, uri) = tmp_file("compl_ns", "let x = 1\n");
        let text = "let a: float = @math.\n";
        let (_, out) = session(vec![
            note("textDocument/didOpen", Json::obj(vec![(
                "textDocument", Json::obj(vec![("uri", Json::str(&uri)), ("text", Json::str(text)), ("version", Json::int(1))]))])),
            req(9, "textDocument/completion", Json::obj(vec![
                ("textDocument", Json::obj(vec![("uri", Json::str(&uri))])),
                ("position", Json::obj(vec![("line", Json::int(0)), ("character", Json::int(21))])),
            ])),
            req(10, "textDocument/completion", Json::obj(vec![("textDocument", Json::obj(vec![("uri", Json::str(&uri))]))])),
            note("exit", Json::Null),
        ]);
        let labels = |id: f64| -> Vec<String> {
            let resp = out.iter().find(|m| m.get("id").and_then(Json::as_f64) == Some(id)).unwrap();
            resp.get("result").unwrap().as_array().unwrap().iter()
                .filter_map(|i| i.get("label").and_then(Json::as_str).map(String::from)).collect()
        };
        let after_dot = labels(9.0);
        for want in ["sqrt", "abs", "random", "pi", "e"] {
            assert!(after_dot.iter().any(|l| l == want), "missing {} after '@math.'", want);
        }
        assert!(!after_dot.iter().any(|l| l == "print" || l == "toUpper" || l == "now"), "leaked non-math items");
        let global = labels(10.0);
        for want in ["print", "len", "@math", "@time", "@array", "@into", "@str"] {
            assert!(global.iter().any(|l| l == want), "missing {} in global completion", want);
        }
        assert!(!global.iter().any(|l| l.contains('.')), "qualified names must not be offered globally");
    }

    #[test]
    fn completion_after_type_colons_lists_its_properties() {
        // `int::` offers its constants, with the value as detail; no global names leak in
        let items = complete_at("compl_int_prop", "let x = int::\n", 0, 13);
        let labels = labels_of(&items);
        assert!(labels.iter().any(|l| l == "max") && labels.iter().any(|l| l == "min"), "{:?}", labels);
        assert!(!labels.iter().any(|l| l == "print"), "global names leaked: {:?}", labels);
        let max = items.iter().find(|i| i.get("label").and_then(Json::as_str) == Some("max")).unwrap();
        assert_eq!(max.get("detail").and_then(Json::as_str), Some("9223372036854775807"));
        // `float::` has three
        let labels = labels_of(&complete_at("compl_float_prop", "let x = float::\n", 0, 15));
        for want in ["max", "min", "epsilon"] {
            assert!(labels.iter().any(|l| l == want), "missing {want}: {:?}", labels);
        }
        // every value type offers `methods`, and only that: no global names leak in
        assert!(labels_of(&items).iter().any(|l| l == "methods"), "int:: lacks methods");
        let labels = labels_of(&complete_at("compl_string_prop", "let x = string::\n", 0, 16));
        for want in ["methods", "newline", "tab", "empty", "digits", "lower", "upper"] {
            assert!(labels.iter().any(|l| l == want), "string:: lacks {}: {:?}", want, labels);
        }
        assert_eq!(labels.len(), 7, "string:: offers exactly its properties: {:?}", labels);
    }

    #[test]
    fn builtins_carry_a_one_line_description() {
        let items = complete_at("doc_math", "@math.\n", 0, 6);
        let abs = items.iter().find(|i| i.get("label").and_then(Json::as_str) == Some("abs")).expect("@math.abs is offered");
        let doc = abs.get("documentation").and_then(Json::as_str).expect("abs has a description");
        assert!(doc.contains("absolute value"), "{}", doc);
        let ns = complete_at("doc_ns", "@\n", 0, 1);
        let math = ns.iter().find(|i| i.get("label").and_then(Json::as_str) == Some("@math")).expect("@math is offered");
        assert!(math.get("documentation").and_then(Json::as_str).is_some_and(|d| !d.is_empty()));
    }

    /// completion result items for `text` with the cursor at (line, col)
    #[test]
    fn installed_libraries_are_offered_as_names() {
        std::env::set_var("SPG_STD", concat!(env!("CARGO_MANIFEST_DIR"), "/../../stdlib"));
        let text = "attach #http\nattach #ps\nattach #terminal\n\n";
        let labels = labels_of(&complete_at("inst_names_on", text, 3, 0));
        // a native library is offered by its name; a standard module offers what it exports
        for want in ["http", "ps", "askString", "rule", "box"] {
            assert!(labels.iter().any(|l| l == want), "missing {want}: {:?}", labels);
        }
        let labels = labels_of(&complete_at("inst_names_off", "\n", 0, 0));
        assert!(!labels.iter().any(|l| l == "http" || l == "ps" || l == "askString"), "{:?}", labels);
    }

    #[test]
    fn ps_dot_lists_the_native_members() {
        // `ps` is an installed library: with the install line its members show up after `ps.`
        let labels = labels_of(&complete_at("ps_dot_on", "attach #ps\nps.\n", 1, 3));
        for member in ["run", "runFile", "version", "isAvailable"] {
            assert!(labels.iter().any(|l| l == member), "missing {member}: {:?}", labels);
        }
        // without the install line `ps.` offers nothing, and `@ps.` is not a namespace anymore
        let labels = labels_of(&complete_at("ps_dot_off", "ps.\n", 0, 3));
        assert!(!labels.iter().any(|l| l == "run"), "{:?}", labels);
        let labels = labels_of(&complete_at("ps_dot_at", "@ps.\n", 0, 4));
        assert!(!labels.iter().any(|l| l == "run"), "{:?}", labels);
    }

    #[test]
    fn me_is_offered_only_inside_a_struct_body() {
        let inside = complete_at("compl_me_in", "struct Pt {\n    x: int\n    fn len() {\n        \n    }\n}\n", 3, 8);
        assert!(labels_of(&inside).iter().any(|l| l == "me"));
        let outside = complete_at("compl_me_out", "let a = 1\n\n", 1, 0);
        assert!(!labels_of(&outside).iter().any(|l| l == "me"));
    }

    #[test]
    fn sized_and_f32_types_offer_their_properties_after_colons() {
        // every type that takes methods offers its constants and `methods` after `::`; the aliases
        // `i64` and `f64` are the same types as `int` and `float`, so they offer the same list
        let cases: [(&str, &[&str]); 7] = [
            ("i8", &["max", "min", "bits", "methods"]),
            ("u64", &["max", "min", "bits", "methods"]),
            ("u16", &["max", "min", "bits", "methods"]),
            ("f32", &["max", "min", "epsilon", "inf", "nan", "methods"]),
            ("i64", &["max", "min", "bits", "methods"]),
            ("f64", &["max", "epsilon", "methods"]),
            ("bool", &["methods"]),
        ];
        for (ty, wants) in cases {
            let text = format!("let x = {}::\n", ty);
            let col = (text.find("::").unwrap() + 2) as i64;
            let labels = labels_of(&complete_at(&format!("compl_prop_{}", ty), &text, 0, col));
            for want in wants {
                assert!(labels.iter().any(|l| l == want), "{}:: lacks {}: {:?}", ty, want, labels);
            }
        }
    }

    #[test]
    fn a_sized_method_body_offers_me_and_the_method_after_a_value() {
        // inside `i8::methods.add(fn ...) { }` the value itself is `me`
        let inside = complete_at("compl_me_sized", "i8::methods.add(fn lol() -> i8 {\n    \n})\n", 1, 4);
        assert!(labels_of(&inside).iter().any(|l| l == "me"), "me missing in an i8 method: {:?}", labels_of(&inside));
        // and the method is offered after a value of that type, as the user's own method
        let after = complete_at(
            "compl_sized_dot",
            "i8::methods.add(fn lol() -> i8 { return me })\nlet x: i8 = 5\nx.\n",
            2,
            2,
        );
        assert!(labels_of(&after).iter().any(|l| l == "lol"), "i8 value lacks lol: {:?}", labels_of(&after));
    }

    #[test]
    fn an_int_value_offers_its_f32_and_f64_conversions() {
        // `x.f32()` and `x.f64()` sit with the other conversions of an int, like `u8()`
        let items = complete_at("compl_int_into", "let a: int = 5\na.\n", 1, 2);
        let labels = labels_of(&items);
        for want in ["f32", "f64", "float", "u8"] {
            assert!(labels.iter().any(|l| l == want), "int. lacks {}: {:?}", want, labels);
        }
    }

    #[test]
    fn me_dot_in_a_string_method_offers_the_string_methods() {
        // `me` in `string::methods.add(...)` has the type of the method's value
        let items = complete_at(
            "compl_me_dot_string",
            "string::methods.add(fn shout() -> string {\n    return me.\n})\n",
            1,
            14,
        );
        assert!(labels_of(&items).iter().any(|l| l == "trim"), "me. in a string method lacks trim: {:?}", labels_of(&items));
    }

    #[test]
    fn global_items_come_in_groups() {
        let items = complete_at("compl_groups", "let mine = 1\n\n", 1, 0);
        let group = |label: &str| {
            items.iter().find(|i| i.get("label").and_then(Json::as_str) == Some(label)).and_then(|i| i.get("sortText")).and_then(Json::as_str).map(String::from)
        };
        assert_eq!(group("mine").as_deref(), Some(SORT_OWN));
        assert_eq!(group("print").as_deref(), Some(SORT_BUILTIN));
        assert_eq!(group("while").as_deref(), Some(SORT_KEYWORD));
        assert_eq!(group("@math").as_deref(), Some(SORT_NAMESPACE));
    }

    fn complete_at(name: &str, text: &str, line: i64, col: i64) -> Vec<Json> {
        let (_f, uri) = tmp_file(name, "let x = 1\n");
        let (_, out) = session(vec![
            note("textDocument/didOpen", Json::obj(vec![(
                "textDocument", Json::obj(vec![("uri", Json::str(&uri)), ("text", Json::str(text)), ("version", Json::int(1))]))])),
            req(9, "textDocument/completion", Json::obj(vec![
                ("textDocument", Json::obj(vec![("uri", Json::str(&uri))])),
                ("position", Json::obj(vec![("line", Json::int(line)), ("character", Json::int(col))])),
            ])),
            note("exit", Json::Null),
        ]);
        let resp = out.iter().find(|m| m.get("id").and_then(Json::as_f64) == Some(9.0)).unwrap();
        resp.get("result").unwrap().as_array().unwrap().to_vec()
    }

    /// the completion request an editor sends when the user types `:` as a trigger character
    fn complete_after_colon_typed(name: &str, text: &str, line: i64, col: i64) -> Vec<Json> {
        let (_f, uri) = tmp_file(name, "let x = 1\n");
        let (_, out) = session(vec![
            note("textDocument/didOpen", Json::obj(vec![(
                "textDocument", Json::obj(vec![("uri", Json::str(&uri)), ("text", Json::str(text)), ("version", Json::int(1))]))])),
            req(9, "textDocument/completion", Json::obj(vec![
                ("textDocument", Json::obj(vec![("uri", Json::str(&uri))])),
                ("position", Json::obj(vec![("line", Json::int(line)), ("character", Json::int(col))])),
                ("context", Json::obj(vec![("triggerKind", Json::int(2)), ("triggerCharacter", Json::str(":"))])),
            ])),
            note("exit", Json::Null),
        ]);
        let resp = out.iter().find(|m| m.get("id").and_then(Json::as_f64) == Some(9.0)).unwrap();
        resp.get("result").unwrap().as_array().unwrap().to_vec()
    }

    #[test]
    fn second_colon_opens_the_type_property_list_at_once() {
        // the text the editor holds right after the second `:` of `int::`
        let items = complete_after_colon_typed("compl_colon_trigger", "let n = int::\n", 0, 14);
        let labels = labels_of(&items);
        assert!(labels.iter().any(|l| l == "methods"), "no list after `int::` typed: {:?}", labels);
    }

    #[test]
    fn after_a_dot_fields_come_before_methods() {
        let items = complete_at("compl_win_groups", "attach #ui\nlet win = ui.window(\"t\", 8, 8)\nwin.\n", 2, 4);
        let sort_of = |label: &str| -> String {
            items
                .iter()
                .find(|i| i.get("label").and_then(Json::as_str) == Some(label))
                .and_then(|i| i.get("sortText"))
                .and_then(Json::as_str)
                .map(String::from)
                .unwrap_or_else(|| panic!("no sortText for {}", label))
        };
        assert_eq!(sort_of("background"), "0", "a field goes in the first group");
        assert_eq!(sort_of("height"), "0", "a field goes in the first group");
        assert_eq!(sort_of("ellipse"), "1", "a method goes in the second group");
        assert_eq!(sort_of("close"), "1", "a method goes in the second group");
    }

    #[test]
    fn the_new_builtin_type_and_the_image_object_are_suggested() {
        // `type` is a global builtin: it comes from the typechecker's table, like `print`
        let globals = labels_of(&complete_at("compl_type_builtin", "let y = ty\n", 0, 11));
        assert!(globals.iter().any(|l| l == "type"), "missing type in {:?}", globals);
        // the `ui` window gets `image` as a method, and an image gets its fields and kill
        let window = labels_of(&complete_at("compl_win_image", "attach #ui\nlet win = ui.window(\"t\", 8, 8)\nwin.\n", 2, 4));
        assert!(window.iter().any(|l| l == "image"), "missing image in {:?}", window);
        let image = labels_of(&complete_at("compl_image_fields", "attach #ui\nlet win = ui.window(\"t\", 8, 8)\nlet pic = win.image(0.0, 0.0, \"a.png\", 0, 0)\npic.\n", 3, 4));
        for want in ["x", "y", "width", "height", "visible", "layer", "kill"] {
            assert!(image.iter().any(|l| l == want), "missing {} in {:?}", want, image);
        }
    }

    #[test]
    fn a_native_library_enum_lists_its_variants_after_its_name() {
        // `Key` is declared in the text of the `ui` library, not in this document
        let labels = labels_of(&complete_at("compl_native_enum", "attach #ui\nKey.\n", 1, 4));
        for want in ["Space", "Left", "A", "Num7", "F12"] {
            assert!(labels.iter().any(|l| l == want), "missing {} in {:?}", want, labels);
        }
        // only when the document installs it: without the install line there is no `Key`
        assert!(complete_at("compl_native_enum_off", "Key.\n", 0, 4).is_empty());
    }

    #[test]
    fn a_native_library_offers_its_enums_and_types_as_plain_names() {
        // `Align` is typed as a name (not after a dot) once the document installs `ui`
        let labels = labels_of(&complete_at("compl_native_type_name", "attach #ui\nAl\n", 1, 2));
        for want in ["Align", "Key", "Window", "Label"] {
            assert!(labels.iter().any(|l| l == want), "missing {} in {:?}", want, labels);
        }
        // without the install line those names do not exist for the program
        let off = labels_of(&complete_at("compl_native_type_name_off", "Al\n", 0, 2));
        assert!(!off.iter().any(|l| l == "Align"), "offered without install: {:?}", off);
    }

    #[test]
    fn the_members_of_a_native_library_hide_the_internals() {
        // `ui.` offers the functions, but not the `_get`/`_set` ones that serve the object fields
        let labels = labels_of(&complete_at("compl_ui_members", "attach #ui\nui.\n", 1, 3));
        assert!(labels.iter().any(|l| l == "window"), "missing window in {:?}", labels);
        assert!(labels.iter().any(|l| l == "rect"), "missing rect in {:?}", labels);
        assert!(!labels.iter().any(|l| l.starts_with('_')), "internals offered: {:?}", labels);
    }

    #[test]
    fn a_native_object_lists_its_fields_and_methods_not_its_handle() {
        let text = "attach #ui\nlet win = ui.window(\"a\", 1, 1)\nwin.\n";
        let labels = labels_of(&complete_at("compl_native_obj", text, 2, 4));
        for want in ["background", "width", "height", "rect", "nextFrame", "isDown", "onClose"] {
            assert!(labels.iter().any(|l| l == want), "missing {} in {:?}", want, labels);
        }
        assert!(!labels.iter().any(|l| l == "id"), "the handle leaks: {:?}", labels);
    }

    #[test]
    fn a_shape_made_by_a_method_call_lists_its_fields_and_methods() {
        // `win.rect(...)` gives a Shape: its members come from the Shape's descriptor
        let text = "attach #ui\nlet win = ui.window(\"a\", 1, 1)\nlet red = win.rect(1.0, 1.0, 2.0, 2.0, 5)\nred.\n";
        let labels = labels_of(&complete_at("compl_native_shape", text, 3, 4));
        for want in ["x", "y", "width", "height", "color", "visible", "layer", "kill"] {
            assert!(labels.iter().any(|l| l == want), "missing {} in {:?}", want, labels);
        }
        assert!(!labels.iter().any(|l| l == "id" || l == "win"), "the handle leaks: {:?}", labels);
    }

    fn labels_of(items: &[Json]) -> Vec<String> {
        items.iter().filter_map(|i| i.get("label").and_then(Json::as_str).map(String::from)).collect()
    }

    #[test]
    fn hash_gt_offers_the_file_directives_not_yet_used() {
        let all = labels_of(&complete_at("compl_dir_all", "#> \nprint(1)\n", 0, 3));
        for d in spg_lexer::DIRECTIVES {
            assert!(all.iter().any(|l| l == d), "missing {} in {:?}", d, all);
        }
        assert_eq!(all.len(), spg_lexer::DIRECTIVES.len(), "only directives after '#>': {:?}", all);

        let rest = complete_at("compl_dir_rest", "#> strict\n#> st\n", 1, 5);
        let labels = labels_of(&rest);
        assert!(!labels.iter().any(|l| l == "strict") && labels.iter().any(|l| l == "timeout"), "{:?}", labels);
        // the edit replaces what was typed after '#>' (here "st": columns 3..5)
        let range = rest[0].get("textEdit").and_then(|e| e.get("range")).expect("a textEdit with a range");
        assert_eq!(range.get("start").unwrap().get("character").and_then(Json::as_f64), Some(3.0));
        assert_eq!(range.get("end").unwrap().get("character").and_then(Json::as_f64), Some(5.0));

        // an arg is being typed, or it's not a directive line at all
        assert!(complete_at("compl_dir_arg", "#> timeout 5\n", 0, 12).is_empty());
        let normal = labels_of(&complete_at("compl_dir_none", "let a = 1\n", 0, 3));
        assert!(!normal.iter().any(|l| l == "strict" || l == "timeout"), "{:?}", normal);
    }

    #[test]
    fn every_directive_has_its_completion_help() {
        for d in spg_lexer::DIRECTIVES {
            assert!(DIRECTIVE_DOCS.iter().any(|(n, _, _)| n == &d), "no completion entry for #> {}", d);
        }
    }

    #[test]
    fn at_map_dot_offers_the_map_members() {
        let items = complete_at("compl_map", "let a = @map.\n", 0, 13);
        let labels = labels_of(&items);
        for member in ["has", "get", "remove", "set", "len", "keys", "values", "keysOf", "map", "filter"] {
            assert!(labels.iter().any(|l| l == member), "missing {} in {:?}", member, labels);
        }
        assert_eq!(items.len(), 10, "{:?}", labels);
    }

    #[test]
    fn picking_a_namespace_inserts_the_dot_and_reopens_the_list() {
        let items = complete_at("compl_pick", "let a = @ma\n", 0, 11);
        // while the name after `@` is typed, only namespaces are offered
        assert_eq!(items.len(), spg_lexer::namespaces().len(), "{:?}", labels_of(&items));
        for ns in ["str", "array", "map", "into", "math", "time", "file", "sys", "json"] {
            let label = format!("@{}", ns);
            let it = items.iter()
                .find(|i| i.get("label").and_then(Json::as_str) == Some(label.as_str()) && i.get("kind").and_then(Json::as_f64) == Some(9.0))
                .unwrap_or_else(|| panic!("no namespace entry for {}", label));
            assert_eq!(it.get("insertText").and_then(Json::as_str), Some(format!("{}.", label).as_str()));
            // the edit swallows the `@` and the partial name: columns 8..11 of `let a = @ma`
            let edit = it.get("textEdit").expect("namespace entry must carry a textEdit");
            let range = edit.get("range").unwrap();
            assert_eq!(range.get("start").unwrap().get("character").and_then(Json::as_f64), Some(8.0));
            assert_eq!(range.get("end").unwrap().get("character").and_then(Json::as_f64), Some(11.0));
            assert_eq!(edit.get("newText").and_then(Json::as_str), Some(format!("{}.", label).as_str()));
            let cmd = it.get("command").expect("namespace entry must carry a command");
            assert_eq!(cmd.get("command").and_then(Json::as_str), Some("editor.action.triggerSuggest"));
        }
        // a plain word still gets the ordinary list. ordinary entries must NOT
        // gain a dot or a command, and the namespaces are still offered
        let items = complete_at("compl_pick_plain", "let a = pr\n", 0, 10);
        assert!(labels_of(&items).iter().any(|l| l == "@math"));
        let print = items.iter().find(|i| i.get("label").and_then(Json::as_str) == Some("print")).unwrap();
        assert!(print.get("insertText").is_none() && print.get("command").is_none());
        let kw = items.iter().find(|i| i.get("label").and_then(Json::as_str) == Some("while")).unwrap();
        assert!(kw.get("insertText").is_none() && kw.get("command").is_none());
    }

    #[test]
    fn string_is_only_a_type_now() {
        // `string` used to be both a type and a namespace. with `@str` it's just the type
        let items = complete_at("compl_str_only", "let a: st\n", 0, 9);
        let strings: Vec<&Json> = items.iter().filter(|i| i.get("label").and_then(Json::as_str) == Some("string")).collect();
        assert_eq!(strings.len(), 1);
        assert_eq!(strings[0].get("kind").and_then(Json::as_f64), Some(25.0));
        assert_eq!(strings[0].get("sortText").and_then(Json::as_str), Some(SORT_TYPE));
    }

    #[test]
    fn dot_after_a_namespace_lists_members_with_or_without_a_partial_name() {
        for (n, text, col) in [("a", "let a = @time.\n", 14), ("b", "let a = @time.no\n", 16), ("c", "  print(@str.\n", 14)] {
            let items = complete_at(&format!("compl_dot_{}", n), text, 0, col);
            let labels = labels_of(&items);
            assert!(!labels.is_empty(), "no members for {:?}", text);
            assert!(items.iter().all(|i| i.get("insertText").is_none()), "members must insert their own label");
            if text.contains("@time.") {
                for want in ["now", "unix", "elapsed", "sleep"] { assert!(labels.iter().any(|l| l == want), "missing {} in {:?}", want, text); }
                assert!(!labels.iter().any(|l| l == "run" || l == "sqrt" || l == "print"), "leak or stale member in {:?}", text);
            } else {
                for want in ["toUpper", "split", "trim"] { assert!(labels.iter().any(|l| l == want), "missing {} in {:?}", want, text); }
                assert!(!labels.iter().any(|l| l == "now"), "leak in {:?}", text);
            }
        }
        // `@into` has its conversions: the ones for float, int, string, f32, and each sized int with its `w` name
        // (`i64` is `int`, so there is no `@into.i64` of its own)
        let into = labels_of(&complete_at("compl_into", "let a = @into.\n", 0, 14));
        let mut sorted = into.clone();
        sorted.sort();
        assert_eq!(
            sorted,
            vec!["f32", "float", "i16", "i16w", "i32", "i32w", "i8", "i8w", "int", "string",
                 "u16", "u16w", "u32", "u32w", "u64", "u64w", "u8", "u8w"]
        );
    }

    #[test]
    fn completion_after_enum_dot_lists_only_its_variants() {
        let text = "enum Color { Red, Green,\n    Blue }\nstruct P { x: int }\nlet a = Color.\nlet b = P.\nlet c = Color.Gr\n";
        let items = complete_at("compl_enum_dot", text, 3, 14);
        assert_eq!(labels_of(&items), vec!["Red", "Green", "Blue"]);
        // a struct name, and a name that isn't an enum, still offer nothing
        assert!(complete_at("compl_enum_struct", text, 4, 10).is_empty());
        // `enum` is a keyword, and the enum's own name is offered as a symbol
        let plain = labels_of(&complete_at("compl_enum_plain", "enum Color { Red }\nlet a = \n", 1, 8));
        assert!(plain.iter().any(|l| l == "enum") && plain.iter().any(|l| l == "Color"));
    }

    #[test]
    fn dot_after_a_standard_module_lists_its_members() {
        // the standard library folder of this checkout (the compiler finds it the same way)
        std::env::set_var("SPG_STD", concat!(env!("CARGO_MANIFEST_DIR"), "/../../stdlib"));
        let labels = labels_of(&complete_at("compl_std_terminal", "attach #terminal\nterminal.\n", 1, 9));
        for want in ["box", "center", "rule", "askInt", "askYesNo"] {
            assert!(labels.iter().any(|l| l == want), "missing {} in {:?}", want, labels);
        }
        // typing the name after `attach #` offers the standard libraries
        let names = labels_of(&complete_at("compl_std_names", "attach #\n", 0, 9));
        for want in ["ps", "http", "terminal"] {
            assert!(names.iter().any(|l| l == want), "missing {} in {:?}", want, names);
        }
        // the same without the space (`attach#`), and another word that starts with install
        let tight = labels_of(&complete_at("compl_std_tight", "attach#\n", 0, 8));
        assert!(tight.iter().any(|l| l == "ps"), "missing ps in {:?}", tight);
        let word = labels_of(&complete_at("compl_std_word", "installer#\n", 0, 10));
        assert!(!word.iter().any(|l| l == "ps"), "library list offered for installer#: {:?}", word);
        // private members (`_v`, the struct's filler field) aren't offered
        assert!(!labels.iter().any(|l| l.starts_with('_')), "private member offered: {:?}", labels);
        // a name that isn't a standard module offers nothing (and doesn't crash)
        assert!(complete_at("compl_std_none", "attach #nope\nterminal.\n", 1, 9).is_empty());
    }

    #[test]
    fn completion_offers_the_array_and_map_loops() {
        let labels = labels_of(&complete_at("compl_array_fns", "let a = @array.\n", 0, 15));
        for want in ["map", "filter", "reduce", "first", "last"] {
            assert!(labels.iter().any(|l| l == want), "missing @array.{} in {:?}", want, labels);
        }
        let labels = labels_of(&complete_at("compl_map_fns", "let a = @map.\n", 0, 13));
        for want in ["map", "filter"] {
            assert!(labels.iter().any(|l| l == want), "missing @map.{} in {:?}", want, labels);
        }
    }

    #[test]
    fn completion_offers_names_from_import_and_install() {
        let (main, uri) = tmp_file("compl_imports", "let x = 1\n");
        let dir = main.parent().unwrap().to_path_buf();
        std::fs::write(dir.join("lib.spg"), "public fn libFn() -> int { return 1 }\npublic struct Pt { x: int }\npublic (viaFacade) from \"./o.spg\"\nfn hidden() -> int { return 2 }\n").unwrap();
        std::fs::write(dir.join("o.spg"), "public fn viaFacade() -> int { return 3 }\n").unwrap();
        let text = "use (other) from \"./o.spg\"\nattach \"./lib.spg\"\n";
        let (_, out) = session(vec![
            note("textDocument/didOpen", Json::obj(vec![(
                "textDocument", Json::obj(vec![("uri", Json::str(&uri)), ("text", Json::str(text)), ("version", Json::int(1))]))])),
            req(9, "textDocument/completion", Json::obj(vec![("textDocument", Json::obj(vec![("uri", Json::str(&uri))]))])),
            note("exit", Json::Null),
        ]);
        let resp = out.iter().find(|m| m.get("id").and_then(Json::as_f64) == Some(9.0)).unwrap();
        let items = resp.get("result").unwrap().as_array().unwrap().to_vec();
        let labels = labels_of(&items);
        for want in ["other", "libFn", "Pt", "viaFacade"] {
            assert!(labels.iter().any(|l| l == want), "missing {} in {:?}", want, labels);
        }
        assert!(!labels.iter().any(|l| l == "hidden"), "a non-exported name leaked");
        // a struct from another file keeps the struct kind
        let pt = items.iter().find(|i| i.get("label").and_then(Json::as_str) == Some("Pt")).unwrap();
        assert_eq!(pt.get("kind").and_then(Json::as_f64), Some(22.0));
    }

    #[test]
    fn struct_members_come_from_the_declaring_file_and_literals_offer_missing_fields() {
        let (main, uri) = tmp_file("compl_members", "let x = 1\n");
        let dir = main.parent().unwrap().to_path_buf();
        std::fs::write(
            dir.join("items.spg"),
            "public struct Item {\n    name: string,\n    qty: int\n    fn total(n: int) -> int { return me.qty * n }\n}\n",
        )
        .unwrap();
        let text = "use (Item) from \"./items.spg\"\nlet it = Item { name: \"x\", qty: 1 }\nit.\nlet z = Item { name: \"y\", \n";
        let at = |line: i64, col: i64| -> Vec<Json> {
            let (_, out) = session(vec![
                note("textDocument/didOpen", Json::obj(vec![(
                    "textDocument", Json::obj(vec![("uri", Json::str(&uri)), ("text", Json::str(text)), ("version", Json::int(1))]))])),
                req(9, "textDocument/completion", Json::obj(vec![
                    ("textDocument", Json::obj(vec![("uri", Json::str(&uri))])),
                    ("position", Json::obj(vec![("line", Json::int(line)), ("character", Json::int(col))])),
                ])),
                note("exit", Json::Null),
            ]);
            let resp = out.iter().find(|m| m.get("id").and_then(Json::as_f64) == Some(9.0)).unwrap();
            resp.get("result").unwrap().as_array().unwrap().to_vec()
        };
        // `it.` -> the fields and the method of the imported struct, with their kinds
        let items = at(2, 3);
        assert_eq!(labels_of(&items), ["name", "qty", "total", "string", "copy"]);
        let kind = |l: &str| items.iter().find(|i| i.get("label").and_then(Json::as_str) == Some(l)).unwrap().get("kind").and_then(Json::as_f64);
        assert_eq!((kind("name"), kind("total")), (Some(5.0), Some(2.0)));
        // `Item { name: "y", ` -> only the field not written yet, inserted as `qty: `
        let fields = at(4, 0);
        assert_eq!(labels_of(&fields), ["qty"]);
        assert_eq!(fields[0].get("insertText").and_then(Json::as_str), Some("qty: "));
    }

    #[test]
    fn dot_after_a_literal_or_a_call_offers_its_type_members() {
        // `"abc".` is a string, `f().` is what f returns: the same members as a variable of that type
        let lit = labels_of(&complete_at("compl_lit", "let a = \"abc\".\n", 0, 14));
        assert!(lit.iter().any(|l| l == "trim") && lit.iter().any(|l| l == "padStart"), "{:?}", lit);
        let call = labels_of(&complete_at("compl_call", "fn f() -> string { return \"x\" }\nlet a = f().\n", 1, 12));
        assert!(call.iter().any(|l| l == "trim"), "{:?}", call);
    }

    #[test]
    fn dot_after_anything_else_offers_nothing() {
        // struct field access, a call with no known result, and a dot inside a string must not
        // dump the global list now that `.` is a trigger character. (`1.`, `"a".`, `f().` offer
        // their type's members now, see members.rs)
        for (n, text, col) in [("a", "let a = p.\n", 10), ("b", "let a = p.xy\n", 12), ("d", "print(\"hi.\")\n", 10), ("e", "let a = f(x).\n", 14),
                               // a struct field named like a namespace is a field, and so is a plain
                               // variable: only the `@` spelling is a namespace
                               ("f", "let a = p.time.\n", 15), ("g", "let a = q.r.math.\n", 17), ("h", "let a = math.\n", 13)] {
            let items = complete_at(&format!("compl_other_{}", n), text, 0, col);
            assert!(items.is_empty(), "expected nothing for {:?}, got {:?}", text, labels_of(&items));
        }
        // ...and a plain identifier (no dot) still gets the full list
        let plain = labels_of(&complete_at("compl_plain", "let a = pr\n", 0, 10));
        assert!(plain.iter().any(|l| l == "print") && plain.iter().any(|l| l == "while"));
    }
}

/// one line on what each keyword does, for hover
fn keyword_doc(word: &str) -> Option<&'static str> {
    Some(match word {
        "let" => "declares a variable",
        "const" => "declares a constant, fixed for the whole program",
        "fn" => "declares a function. `fn Type.name()` declares an associated function of a struct or enum",
        "return" => "leaves the function with a value",
        "if" => "runs a block when the condition holds (the condition goes in parentheses)",
        "elif" | "else" => "the other branches of an `if`",
        "while" => "repeats a block while the condition holds",
        "do" => "runs its block once, then repeats it while the condition holds: `do { } while (cond)`",
        "in" => "in a `for`: the item (or the index and item) of the array, map or string being walked",
        "step" => "in a range loop: how much each round moves, as in `for (i in 0..10 step 2)`",
        "me" => "inside a struct method or guard: the struct itself",
        "i8" => "signed 8-bit integer, from -128 to 127",
        "i16" => "signed 16-bit integer, from -32768 to 32767",
        "i32" => "signed 32-bit integer, from -2147483648 to 2147483647",
        "i64" => "signed 64-bit integer. the same type as `int`",
        "u8" => "unsigned 8-bit integer, from 0 to 255",
        "u16" => "unsigned 16-bit integer, from 0 to 65535",
        "u32" => "unsigned 32-bit integer, from 0 to 4294967295",
        "u64" => "unsigned 64-bit integer, from 0 to 18446744073709551615",
        "f32" => "32-bit float. rounds at each operation",
        "f64" => "64-bit float. the same type as `float`",
        "guard" => "inside a struct: a rule checked after every write to it. `guard (me.field as name) { }` for one field, `guard { }` for the whole struct. a `throw` refuses the write",
        "for" => "walks the items of an array, map or range",
        "try" => "runs a block and catches the error it may throw",
        "catch" => "receives the message of an error raised inside its `try`. always a string",
        "throw" => "raises an error with a message",
        "break" | "continue" => "leaves or skips the current loop iteration",
        "when" => "matches a value against cases",
        "use" | "from" | "public" => "shares names between files",
        "attach" => "brings another file or a standard module into the program",
        "struct" => "declares a record with named fields; its methods go inside the block",
        "enum" => "declares a type with a fixed set of variants",
        "namespace" => "groups functions and statics under one name, no instances: call them as Name.member()",
        "then" => "pipe: `x then f()` is `f(x)`. the same as `|>`",
        "and" => "logical and, the same as `&&`",
        "or" => "logical or, the same as `||`",
        "not" => "logical not, the same as `!`",
        "true" | "false" => "the boolean values",
        "none" => "the absence of a value",
        _ => return None,
    })
}
