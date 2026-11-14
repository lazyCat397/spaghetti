//! turns the compiler pipeline's failures into flat, positioned
//! diagnostics (file + 1-based line/column + message). boring, but useful.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct Diag {
    pub file: PathBuf,
    /// 1-based, column counted in characters (what the lexer records)
    pub line: usize,
    pub col: usize,
    pub message: String,
}

/// runs resolve -> typecheck -> compile on `entry` (buffers in `overlay`
/// win over disk) and returns everything wrong with it. resolve stops at
/// its first error. the typechecker reports all of its errors. show-off.
pub fn analyze(entry: &Path, overlay: &HashMap<PathBuf, String>) -> Vec<Diag> {
    let mut diags = analyze_pipeline(entry, overlay);
    diags.extend(native_path_diags(entry, overlay));
    diags
}

fn analyze_pipeline(entry: &Path, overlay: &HashMap<PathBuf, String>) -> Vec<Diag> {
    let entry_canon = spg_modules::canon(entry).unwrap_or_else(|_| entry.to_path_buf());
    // a value method still being typed (`int::methods.add`) is blanked for the check: its error
    // would land on the NEXT line and stay until the declaration is whole
    let sanitized = without_unfinished_methods(&entry_canon, entry, overlay);
    let overlay = sanitized.as_ref().unwrap_or(overlay);
    let program = match spg_modules::resolve_with(entry, overlay) {
        Ok(p) => p,
        Err(e) => {
            let l = e.locate();
            // a `#>` line still being typed (`#>`, `#> st`) is not an error yet:
            // the error would be on that very line, and it goes away once the name is whole
            if locate_file(Some(&l.file.to_string_lossy()), &entry_canon) == entry_canon
                && typing_directive_at(&entry_canon, overlay, l.line)
            {
                return Vec::new();
            }
            return vec![Diag { file: l.file, line: l.line, col: l.col, message: l.message }];
        }
    };
    // `#> strict` in the header of the file being edited (the open buffer wins
    // over the disk, so the squiggles follow what you type)
    let strict = entry_spec(&entry_canon, overlay).strict;
    match spg_typecheck::type_check_with(program, strict) {
        Err(errors) => errors
            .iter()
            .map(|e| {
                let pos = e.pos();
                Diag {
                    file: locate_file(pos.file.as_deref(), &entry_canon),
                    line: pos.line,
                    col: pos.col,
                    message: e.message(),
                }
            })
            .collect(),
        Ok(typed) => match spg_compiler::compile(typed) {
            Ok(_) => Vec::new(),
            Err(e) => vec![Diag {
                file: entry_canon,
                line: 1,
                col: 1,
                message: format!("compile error: {}", e),
            }],
        },
    }
}

/// the buffer of `entry` with its unfinished `int::methods.add` lines blanked out. the number of
/// lines does not change, so every other position stays right. `None` when there is nothing to blank.
fn without_unfinished_methods(
    entry_canon: &Path,
    entry: &Path,
    overlay: &HashMap<PathBuf, String>,
) -> Option<HashMap<PathBuf, String>> {
    let text = overlay
        .get(entry_canon)
        .or_else(|| overlay.get(entry))
        .cloned()
        .or_else(|| std::fs::read_to_string(entry).ok())?;
    if !text.lines().any(unfinished_value_method) {
        return None;
    }
    let mut out = String::with_capacity(text.len());
    for line in text.split_inclusive('\n') {
        if unfinished_value_method(line) {
            if line.ends_with('\n') {
                out.push('\n');
            }
        } else {
            out.push_str(line);
        }
    }
    let mut next = overlay.clone();
    next.insert(entry_canon.to_path_buf(), out.clone());
    next.insert(entry.to_path_buf(), out);
    Some(next)
}

/// a line that starts a value method but is not whole yet: `int::methods`, `int::methods.`,
/// `int::methods.ad`, `int::methods.add(`, or a header `int::methods.add(fn name(...)` with no `{`.
/// a complete one-line method (with its `{`) is not unfinished.
pub fn unfinished_value_method(line: &str) -> bool {
    let t = line.trim();
    let rest = ["int", "float", "bool", "string"].iter().find_map(|ty| t.strip_prefix(ty)?.strip_prefix("::"));
    let Some(rest) = rest.and_then(|r| r.strip_prefix("methods")) else {
        return false;
    };
    // `int::methods` / `int::methods.` / `int::methods.ad`
    let Some(after_dot) = rest.strip_prefix('.') else {
        return rest.trim().is_empty();
    };
    let name_end = after_dot
        .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
        .unwrap_or(after_dot.len());
    let after_name = after_dot[name_end..].trim();
    if after_name.is_empty() {
        return true;
    }
    // `int::methods.add(` and `int::methods.add(fn name(...)` still without its `{`
    let Some(inside) = after_name.strip_prefix('(') else {
        return false;
    };
    let inside = inside.trim_start();
    if inside.is_empty() {
        return true;
    }
    match inside.strip_prefix("fn") {
        Some(h) if h.is_empty() || !h.starts_with(|c: char| c.is_ascii_alphanumeric() || c == '_') => !h.contains('{'),
        _ => false,
    }
}

/// true when line `line` (1-based) of `entry` is a `#>` with a name still being
/// written: empty (`#>`) or a prefix of a directive that isn't complete yet
/// (`#> st` for `strict`). a whole wrong name (`#> strikt`) is not, and stays an error
fn typing_directive_at(entry: &Path, overlay: &HashMap<PathBuf, String>, line: usize) -> bool {
    let src = overlay.get(entry).cloned().or_else(|| std::fs::read_to_string(entry).ok());
    let Some(text) = src else { return false };
    let Some(l) = line.checked_sub(1).and_then(|i| text.lines().nth(i)) else { return false };
    let Some(rest) = l.trim_start().strip_prefix("#>") else { return false };
    let name = rest.trim();
    if !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') {
        return false;
    }
    if spg_lexer::DIRECTIVES.contains(&name) {
        return false;
    }
    spg_lexer::DIRECTIVES.iter().any(|d| d.starts_with(name))
}

/// the `#>` header of `entry` (empty when it can't be read or is broken. the
/// resolver already reports that as a diagnostic of its own)
fn entry_spec(entry: &Path, overlay: &HashMap<PathBuf, String>) -> spg_lexer::FileSpec {
    let src = overlay.get(entry).cloned().or_else(|| std::fs::read_to_string(entry).ok());
    src.and_then(|s| spg_lexer::read_file_spec(&s).ok()).unwrap_or_default()
}

/// `Pos.file` is the "friendly" name (relative to the working dir when it
/// lives under it). turn it back into the canonical path buffers are
/// keyed by. the name game again.
fn locate_file(name: Option<&str>, fallback: &Path) -> PathBuf {
    let Some(name) = name else { return fallback.to_path_buf() };
    let mut p = PathBuf::from(name);
    if p.is_relative() {
        if let Ok(cwd) = std::env::current_dir() {
            p = cwd.join(p);
        }
    }
    spg_modules::canon(&p).unwrap_or(p)
}

/// the UTF-16 `[start, end)` span (LSP's unit) of the token that starts at
/// 1-based character column `col` of `line_text`: a whole identifier, a
/// whole string literal, or a single character. the unit of pain.
pub fn utf16_range(line_text: &str, col: usize) -> (usize, usize) {
    let chars: Vec<char> = line_text.chars().collect();
    let start = col.saturating_sub(1).min(chars.len());
    let mut end = start;
    if start < chars.len() {
        let c = chars[start];
        if c.is_alphanumeric() || c == '_' {
            while end < chars.len() && (chars[end].is_alphanumeric() || chars[end] == '_') {
                end += 1;
            }
        } else if c == '"' {
            end = start + 1;
            while end < chars.len() {
                let d = chars[end];
                end += 1;
                if d == '\\' {
                    end += 1;
                } else if d == '"' {
                    break;
                }
            }
            end = end.min(chars.len());
        } else {
            end = start + 1;
        }
    }
    let u16_of = |n: usize| chars[..n].iter().map(|c| c.len_utf16()).sum::<usize>();
    (u16_of(start), u16_of(end))
}

/// `label.font = "./f.ttf"` and `win.image(x, y, "./p.png", w, h)` name files too, but the
/// compiler only sees strings there, so a missing file shows up only when the program runs.
/// the literal ones are checked here, the way `use` and `attach` already are: relative
/// to the file being edited. `font = ""` is valid (it goes back to the bitmap font), so it
/// is not checked; `image("")` has no file at all, so it is an error
fn native_path_diags(entry: &Path, overlay: &HashMap<PathBuf, String>) -> Vec<Diag> {
    use spg_lexer::TokenKind;
    let Some(text) = overlay.get(entry).cloned().or_else(|| std::fs::read_to_string(entry).ok()) else {
        return Vec::new();
    };
    let Ok(tokens) = spg_lexer::Lexer::new(&text).tokenize() else { return Vec::new() };
    let dir = entry.parent().unwrap_or(Path::new("."));
    let mut out = Vec::new();
    for i in 0..tokens.len() {
        if tokens[i].kind != TokenKind::Dot {
            continue;
        }
        let Some(TokenKind::Ident(name)) = tokens.get(i + 1).map(|t| &t.kind) else { continue };
        let found: Option<(&spg_lexer::Token, &str, &str, &[&str], bool)> = match name.as_str() {
            "font" if tokens.get(i + 2).map(|t| &t.kind) == Some(&TokenKind::Eq) => match tokens.get(i + 3) {
                // `font = "a" + x` is not a plain literal: leave it to the runtime
                Some(t) if matches!(tokens.get(i + 4).map(|n| &n.kind), None | Some(TokenKind::Eof) | Some(TokenKind::Newline) | Some(TokenKind::Semicolon) | Some(TokenKind::RBrace)) => {
                    if let TokenKind::Str(path) = &t.kind {
                        Some((t, path.as_str(), "font", &["ttf", "otf"], false))
                    } else {
                        None
                    }
                }
                _ => None,
            },
            "image" if tokens.get(i + 2).map(|t| &t.kind) == Some(&TokenKind::LParen) => {
                third_argument(&tokens, i + 2).map(|(t, path)| (t, path, "image", &["png"][..], true))
            }
            _ => None,
        };
        let Some((tok, path, what, exts, is_image)) = found else { continue };
        let problem = if path.is_empty() {
            if is_image { Some(format!("the {what} needs a file path")) } else { None }
        } else {
            let ext_ok = Path::new(path)
                .extension()
                .and_then(|e| e.to_str())
                .map_or(false, |e| exts.iter().any(|x| e.eq_ignore_ascii_case(x)));
            if ext_ok && !dir.join(path).is_file() {
                Some(format!("cannot find the {what} \"{path}\" (paths are relative to this file)"))
            } else {
                None
            }
        };
        if let Some(message) = problem {
            out.push(Diag { file: entry.to_path_buf(), line: tok.line, col: tok.col, message });
        }
    }
    out
}

/// the third argument of the call whose `(` is at `lparen`, when it is a single plain string
fn third_argument(tokens: &[spg_lexer::Token], lparen: usize) -> Option<(&spg_lexer::Token, &str)> {
    use spg_lexer::TokenKind;
    let mut args: Vec<Vec<&spg_lexer::Token>> = vec![Vec::new()];
    let mut depth = 0usize;
    for t in &tokens[lparen + 1..] {
        match t.kind {
            TokenKind::LParen | TokenKind::LBracket | TokenKind::LBrace => {
                depth += 1;
                args.last_mut()?.push(t);
            }
            TokenKind::RParen | TokenKind::RBracket | TokenKind::RBrace => {
                if depth == 0 {
                    break;
                }
                depth -= 1;
                args.last_mut()?.push(t);
            }
            TokenKind::Comma if depth == 0 => args.push(Vec::new()),
            TokenKind::Newline => {}
            _ => args.last_mut()?.push(t),
        }
    }
    let arg = args.get(2)?;
    match arg.as_slice() {
        [only] => match &only.kind {
            TokenKind::Str(path) => Some((*only, path.as_str())),
            _ => None,
        },
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strict_header_turns_on_the_strict_rules() {
        let dir = std::env::temp_dir().join("spg_lsp_strict_test");
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("s.spg");
        let src = "let a: int = 1\nfn f() {\n  let a: int = 2\n  print(a)\n}\nf()\n";
        std::fs::write(&file, src).unwrap();
        let canon = spg_modules::canon(&file).unwrap();
        let mut overlay = HashMap::new();
        overlay.insert(canon.clone(), src.to_string());
        assert!(analyze(&file, &overlay).is_empty(), "without the header the same code is fine");
        overlay.insert(canon, format!("#> strict\n{}", src));
        let diags = analyze(&file, &overlay);
        assert!(diags.iter().any(|d| d.message.contains("hides a name")), "{:?}", diags);
    }

    #[test]
    fn range_covers_identifier() {
        // let total = 1  -> `total` starts at column 5
        assert_eq!(utf16_range("let total = 1", 5), (4, 9));
    }

    #[test]
    fn range_covers_string_literal() {
        assert_eq!(utf16_range("let s = \"a b\" + 1", 9), (8, 13));
    }

    #[test]
    fn range_after_accent_uses_utf16() {
        // `é` = 1 char / 1 UTF-16 unit. the emoji = 1 char / 2 units. unicode is a lifestyle.
        assert_eq!(utf16_range("\"é\" + x", 7), (6, 7));
        assert_eq!(utf16_range("\"😀\" + x", 7), (7, 8));
    }

    #[test]
    fn range_past_end_is_empty_not_a_panic() {
        assert_eq!(utf16_range("abc", 99), (3, 3));
        assert_eq!(utf16_range("", 1), (0, 0));
    }
}

#[cfg(test)]
mod typing_directive_tests {
    use super::*;

    fn run(name: &str, text: &str) -> Vec<Diag> {
        let dir = std::env::temp_dir().join(format!("spg_diag_{}", name));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("main.spg");
        std::fs::write(&path, text).unwrap();
        let mut overlay = HashMap::new();
        overlay.insert(spg_modules::canon(&path).unwrap(), text.to_string());
        analyze(&path, &overlay)
    }

    #[test]
    fn directive_being_typed_is_not_an_error_but_a_wrong_name_is() {
        // `#>` and a prefix of `strict` are still being written: no squiggle yet
        assert!(run("typing_empty", "#>\nprint(1)\n").is_empty());
        assert!(run("typing_prefix", "#> st\nprint(1)\n").is_empty());
        // a whole wrong name is an error, as before
        assert!(!run("wrong_name", "#> strikt\nprint(1)\n").is_empty());
        // a complete directive without its argument is an error too
        assert!(!run("timeout_bare", "#> timeout\nprint(1)\n").is_empty());
    }
}

#[cfg(test)]
mod native_path_tests {
    use super::*;

    /// a folder with the given files (empty), the main file's text in the overlay
    fn run(name: &str, files: &[&str], text: &str) -> Vec<Diag> {
        let dir = std::env::temp_dir().join(format!("spg_native_paths_{}", name));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        for f in files {
            let p = dir.join(f);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(&p, "").unwrap();
        }
        let path = dir.join("main.spg");
        std::fs::write(&path, text).unwrap();
        let mut overlay = HashMap::new();
        overlay.insert(spg_modules::canon(&path).unwrap(), text.to_string());
        native_path_diags(&spg_modules::canon(&path).unwrap(), &overlay)
    }

    fn msgs(d: &[Diag]) -> Vec<String> {
        d.iter().map(|x| x.message.clone()).collect()
    }

    #[test]
    fn missing_font_file_is_an_error_and_an_existing_one_is_not() {
        let src = "label.font = \"./sans.ttf\"\n";
        let bad = run("font_missing", &[], src);
        assert_eq!(bad.len(), 1, "{:?}", msgs(&bad));
        assert!(bad[0].message.contains("cannot find the font \"./sans.ttf\""));
        assert_eq!((bad[0].line, bad[0].col), (1, 14));
        assert!(run("font_there", &["sans.ttf"], src).is_empty());
    }

    #[test]
    fn empty_font_goes_back_to_the_bitmap_and_is_fine() {
        assert!(run("font_empty", &[], "label.font = \"\"\n").is_empty());
    }

    #[test]
    fn image_checks_the_third_argument_only() {
        let src = "win.image(0, 0, \"./pic.png\", 10, 10)\n";
        let bad = run("image_missing", &[], src);
        assert_eq!(bad.len(), 1, "{:?}", msgs(&bad));
        assert!(bad[0].message.contains("cannot find the image \"./pic.png\""));
        assert!(run("image_there", &["pic.png"], src).is_empty());
    }

    #[test]
    fn empty_image_path_is_an_error() {
        let bad = run("image_empty", &[], "win.image(0, 0, \"\", 0, 0)\n");
        assert_eq!(bad.len(), 1, "{:?}", msgs(&bad));
        assert!(bad[0].message.contains("needs a file path"));
    }

    #[test]
    fn computed_paths_are_left_to_the_runtime() {
        assert!(run("font_expr", &[], "label.font = \"./a.ttf\" + x\n").is_empty());
        assert!(run("image_expr", &[], "win.image(0, 0, name, 1, 1)\n").is_empty());
    }

    #[test]
    fn dot_and_dot_dot_paths_resolve_from_the_file_folder() {
        assert!(run("dotdot", &["assets/sans.ttf", "sub/k"], "label.font = \"./sub/../assets/sans.ttf\"\n").is_empty());
        assert!(run("dotdot_ok", &["x/y.png"], "win.image(0, 0, \"./x/../x/y.png\", 1, 1)\n").is_empty());
    }
}

#[cfg(test)]
mod unfinished_method_tests {
    use super::*;

    fn run(name: &str, text: &str) -> Vec<Diag> {
        let dir = std::env::temp_dir().join(format!("spg_diag_{}", name));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("main.spg");
        std::fs::write(&path, text).unwrap();
        let mut overlay = HashMap::new();
        overlay.insert(spg_modules::canon(&path).unwrap(), text.to_string());
        analyze(&path, &overlay)
    }

    #[test]
    fn only_the_unfinished_forms_are_recognised() {
        for line in ["int::methods", "int::methods.", "int::methods.ad", "int::methods.add", "  float::methods.add(", "string::methods.add(fn shout() -> string"] {
            assert!(unfinished_value_method(line), "should be unfinished: {:?}", line);
        }
        for line in ["int::methods.add(fn dobro() -> int { return me * 2 })", "print(int::bits)", "let n = int::max", "int::methods.add(fn x() -> int {"] {
            assert!(!unfinished_value_method(line), "should be left alone: {:?}", line);
        }
    }

    #[test]
    fn a_bare_add_does_not_squiggle_the_next_line() {
        // the error used to land on `let`, the line after the half-written `add`
        assert!(run("bare_add", "int::methods.add\nlet x = 1\nprint(x)\n").is_empty());
        assert!(run("bare_dot", "int::methods.\nprint(1)\n").is_empty());
        assert!(run("partial_header", "int::methods.add(fn dobro() -> int\nprint(1)\n").is_empty());
    }

    #[test]
    fn a_real_error_elsewhere_is_still_reported() {
        assert!(!run("real_error", "int::methods.add\nlet x: int = \"oi\"\n").is_empty());
    }

    #[test]
    fn a_finished_method_still_works_and_is_checked() {
        assert!(run("finished", "int::methods.add(fn dobro() -> int { return me * 2 })\nprint(2.dobro())\n").is_empty());
    }
}
