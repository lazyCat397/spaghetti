//! errors raised while resolving a program's import graph. every message
//! is English, per the language's sacred output rule.

use std::fmt;
use std::path::PathBuf;

use spg_parser::ast::Pos;

/// how a file is shown to the user: relative to the current directory
/// when it lives under it (`lib/a.spg`), the full path otherwise. the
/// resolver works with canonical absolute paths internally. those are
/// noise in a diagnostic. nobody reads that.
pub fn friendly(path: &std::path::Path) -> String {
    if let Ok(cwd) = std::env::current_dir() {
        // stripping the root ("/") itself would leave a path that LOOKS
        // relative ("home/x/a.spg") but isn't one. every path is "under"
        // the root, so that prefix carries no info worth cutting
        if cwd.parent().is_some() {
            if let Ok(rel) = path.strip_prefix(&cwd) {
                return rel.display().to_string();
            }
        }
    }
    path.display().to_string()
}

#[derive(Debug)]
pub enum ModuleError {
    /// the entry file (or an imported one) couldn't be read
    Io { path: PathBuf, message: String },

    /// lexing an imported/entry file failed
    Lex { path: PathBuf, message: String, error: spg_lexer::LexError },

    /// `#>` in an imported module. directives belong to the file you run
    DirectiveInImport { path: PathBuf, name: String, line: usize, col: usize },

    /// parsing an imported/entry file failed
    Parse { path: PathBuf, message: String, error: spg_parser::ParseError },

    /// `import ... from "x"` where the path has no `.spg` extension.
    /// the extension is mandatory (it's how a file gets told apart from
    /// a directory). never inferred. no guessing.
    MissingExtension { importer: PathBuf, path: String, pos: Pos },

    /// the import graph loops back on itself. `chain` is the full cycle,
    /// first module repeated at the end (a -> b -> a). a loop, a loop.
    Cycle { chain: Vec<PathBuf> },

    /// `import { x } from "m.spg"` but `m.spg` declares no `x` at all
    UnknownExport { importer: PathBuf, module: String, name: String, pos: Pos },

    /// `x` is in that module's scope only because IT imported it, and
    /// importing doesn't re-export. `imported_at` is that import's line.
    ImportedNotReExported { importer: PathBuf, module: String, name: String, pos: Pos, imported_at: Pos, origin: String },

    /// `x` exists in the module but wasn't marked `export`
    NotExported { importer: PathBuf, module: String, name: String, pos: Pos },

    /// two things in ONE file want the same name. `second` is where the
    /// conflict was detected (the one the error points at). `first` is
    /// the earlier thing that already owned the name (the `note:`). squatter.
    Collision { file: PathBuf, name: String, kind: CollisionKind, second: Pos, first: Pos },

    /// a name two different `install`s both provide, and the file USES it.
    /// importing it by name (`import { x } from "..."`) picks one. pick wisely.
    AmbiguousInstall { file: PathBuf, name: String, first: (Pos, String), second: (Pos, String) },
}

/// what the two colliding things are. decides the wording, since
/// "'x' conflicts with a local declaration" is only true when one side
/// really is a local declaration. honesty in error messages.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum CollisionKind {
    /// `function a` declared twice (or a `let a` next to a `function a`)
    LocalVsLocal,
    /// an `import { a }` whose `a` is also declared in this file
    ImportVsLocal,
    /// the same name imported twice (from the same or different modules)
    ImportVsImport,
    /// `export { a } from ...` next to a `export function a` in the same
    /// file. two different things would both be importable as `a`
    ReExportVsLocal,
    /// the same name re-exported twice. again. stop it.
    ReExportVsReExport,
}

impl fmt::Display for ModuleError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ModuleError::Io { path, message } => {
                write!(f, "could not read '{}': {}", friendly(path), message)
            }
            ModuleError::DirectiveInImport { path, name, line, col } => write!(
                f,
                "directive '#> {}' in '{}' at line {}, column {}: only the file you run can have directives",
                name, friendly(path), line, col
            ),
            ModuleError::Lex { path, message, .. } => {
                write!(f, "lex error in '{}': {}", friendly(path), message)
            }
            ModuleError::Parse { path, message, .. } => {
                write!(f, "parse error in '{}': {}", friendly(path), message)
            }
            ModuleError::MissingExtension { importer, path, pos } => write!(
                f,
                "module path \"{}\" must end in '.spg' (in '{}' at line {}, column {})",
                path,
                friendly(importer),
                pos.line,
                pos.col
            ),
            ModuleError::Cycle { chain } => {
                let names: Vec<String> = chain.iter().map(|p| friendly(p)).collect();
                write!(f, "circular import: {}", names.join(" -> "))
            }
            ModuleError::UnknownExport { importer, module, name, pos } => write!(
                f,
                "module \"{}\" has no declaration named '{}' (in '{}' at line {}, column {})",
                module,
                name,
                friendly(importer),
                pos.line,
                pos.col
            ),
            ModuleError::ImportedNotReExported { importer, module, name, pos, imported_at, origin } => write!(
                f,
                "'{}' is only imported by module \"{}\" (line {}), not exported from it — importing doesn't re-export; add `export {{ {} }} from \"{}\"` there (in '{}' at line {}, column {})",
                name,
                module,
                imported_at.line,
                name,
                origin,
                friendly(importer),
                pos.line,
                pos.col
            ),
            ModuleError::NotExported { importer, module, name, pos } => write!(
                f,
                "'{}' exists in module \"{}\" but is not exported (in '{}' at line {}, column {})",
                name,
                module,
                friendly(importer),
                pos.line,
                pos.col
            ),
            ModuleError::AmbiguousInstall { file, name, first, second } => write!(
                f,
                "'{}' is installed from both \"{}\" (line {}) and \"{}\" (line {}) and used in this file; import it by name to pick one (in '{}' at line {}, column {})",
                name, first.1, first.0.line, second.1, second.0.line,
                friendly(file), second.0.line, second.0.col
            ),
            ModuleError::Collision { file, name, kind, second, first } => {
                let what = match kind {
                    CollisionKind::LocalVsLocal => "is declared more than once",
                    CollisionKind::ImportVsLocal => "conflicts with a local declaration",
                    CollisionKind::ImportVsImport => "is imported more than once",
                    CollisionKind::ReExportVsLocal => "is re-exported here, but this file also exports its own",
                    CollisionKind::ReExportVsReExport => "is re-exported more than once",
                };
                // `import { a, a }`: both sides are the same statement, so
                // "first imported here -> the line we already pointed at"
                // would say nothing. say what actually happened instead.
                let same_line = second.line == first.line;
                let also = match kind {
                    CollisionKind::LocalVsLocal => "is first declared here",
                    CollisionKind::ImportVsLocal => "is also declared here",
                    CollisionKind::ImportVsImport if same_line => "is listed twice in this import",
                    CollisionKind::ImportVsImport => "is first imported here",
                    CollisionKind::ReExportVsLocal => "is declared and exported here",
                    CollisionKind::ReExportVsReExport if same_line => "is listed twice in this re-export",
                    CollisionKind::ReExportVsReExport => "is first re-exported here",
                };
                write!(
                    f,
                    "'{}' {}\n  --> {}:{}\n\nnote: '{}' {}\n  --> {}:{}",
                    name,
                    what,
                    friendly(file),
                    second.line,
                    name,
                    also,
                    friendly(file),
                    first.line
                )
            }
        }
    }
}

impl std::error::Error for ModuleError {}

/// a module error boiled down to what an editor needs: which file, where
/// (1-based line/column, like everywhere else in the compiler) and a message
/// without the "(in 'file' at line X, column Y)" tail the CLI text carries
#[derive(Debug, Clone)]
pub struct Located {
    pub file: PathBuf,
    pub line: usize,
    pub col: usize,
    pub message: String,
}

impl ModuleError {
    pub fn locate(&self) -> Located {
        let full = self.to_string();
        // the tail is redundant once the position is structured
        let clean = |s: &str| match s.rfind(" (in '") {
            Some(i) => s[..i].to_string(),
            None => s.to_string(),
        };
        match self {
            ModuleError::Io { path, message } => Located {
                file: path.clone(), line: 1, col: 1,
                message: format!("could not read file: {}", message),
            },
            ModuleError::DirectiveInImport { path, name, line, col } => Located {
                file: path.clone(), line: *line, col: *col,
                message: format!("directive '#> {}' only works in the file you run; remove it from this module", name),
            },
            ModuleError::Lex { path, message, .. } | ModuleError::Parse { path, message, .. } => {
                let (line, col) = line_col_in(message).unwrap_or((1, 1));
                Located { file: path.clone(), line, col, message: strip_pos(message) }
            }
            ModuleError::MissingExtension { importer, pos, .. }
            | ModuleError::UnknownExport { importer, pos, .. }
            | ModuleError::ImportedNotReExported { importer, pos, .. }
            | ModuleError::NotExported { importer, pos, .. } => Located {
                file: importer.clone(), line: pos.line, col: pos.col, message: clean(&full),
            },
            ModuleError::Cycle { chain } => Located {
                file: chain.first().cloned().unwrap_or_default(), line: 1, col: 1, message: full,
            },
            ModuleError::AmbiguousInstall { file, second, .. } => Located {
                file: file.clone(), line: second.0.line, col: second.0.col, message: clean(&full),
            },
            ModuleError::Collision { file, name, kind, second, .. } => {
                let what = match kind {
                    CollisionKind::LocalVsLocal => "is declared more than once",
                    CollisionKind::ImportVsLocal => "conflicts with a local declaration",
                    CollisionKind::ImportVsImport => "is imported more than once",
                    CollisionKind::ReExportVsLocal => "is re-exported here, but this file also exports its own",
                    CollisionKind::ReExportVsReExport => "is re-exported more than once",
                };
                Located {
                    file: file.clone(), line: second.line, col: second.col,
                    message: format!("'{}' {}", name, what),
                }
            }
        }
    }
}

/// finds "line N, column M" anywhere in a lexer/parser message
fn line_col_in(msg: &str) -> Option<(usize, usize)> {
    let i = msg.rfind("line ")?;
    let rest = &msg[i + 5..];
    let line: usize = rest.chars().take_while(|c| c.is_ascii_digit()).collect::<String>().parse().ok()?;
    let j = rest.find("column ")?;
    let col: usize = rest[j + 7..].chars().take_while(|c| c.is_ascii_digit()).collect::<String>().parse().ok()?;
    Some((line, col))
}

/// drops the " at line N, column M" / " (line N, column M)" tail
fn strip_pos(msg: &str) -> String {
    if let Some(i) = msg.rfind(" (line ") {
        return msg[..i].to_string();
    }
    if let Some(i) = msg.rfind(" at line ") {
        return msg[..i].to_string();
    }
    msg.to_string()
}

impl ModuleError {
    /// the same fact, friendly tone (`#> friendly-terminal`). the nice version.
    pub fn friendly(&self) -> String {
        match self {
            ModuleError::Lex { path, error, .. } => error.friendly(&friendly(path)),
            ModuleError::Parse { path, error, .. } => error.friendly_in(&friendly(path)),
            _ => spg_lexer::friendly_sentence(&self.to_string(), spg_lexer::HINT_MODULE),
        }
    }
}
