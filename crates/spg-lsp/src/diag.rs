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
    let program = match spg_modules::resolve_with(entry, overlay) {
        Ok(p) => p,
        Err(e) => {
            let l = e.locate();
            return vec![Diag { file: l.file, line: l.line, col: l.col, message: l.message }];
        }
    };
    let entry_canon = spg_modules::canon(entry).unwrap_or_else(|_| entry.to_path_buf());
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strict_header_turns_on_the_strict_rules() {
        let dir = std::env::temp_dir().join("spg_lsp_strict_test");
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("s.spg");
        let src = "let a: int = 1\nfunction f() {\n  let a: int = 2\n  print(a)\n}\nf()\n";
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
