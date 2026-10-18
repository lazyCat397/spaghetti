//! spg-modules: turns an entry `.spg` file and everything it imports
//! into ONE merged `spg_parser::Program`. one big file, eventually.
//!
//! runs between the parser and the typechecker. that's the whole point
//! of the design. after this crate, typecheck/compiler/VM see a single
//! flat program and never need to know several files existed. blissful ignorance.
//!
//! how it stays collision-free: every top-level symbol of every module
//! except the entry file gets a mangled name (`<module path>::<name>`),
//! and every use of it (in its own module or in an importer) gets
//! rewritten to match (see `rename`). the entry file keeps its names
//! as-is, so single-file programs behave exactly as before, and error
//! messages about the user's own code stay readable. for the user, not us.
//!
//! fixed language decisions this implements (no arguing):
//! - only named imports: `import { a, b } from "./x.spg"`
//! - an import cycle is always a fatal error, never "resolved"
//! - the path must carry an explicit `.spg` extension
//! - a module is loaded ONCE even if several files import it (diamond)

pub mod annotate;
pub mod error;
pub mod rename;

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use spg_lexer::Lexer;
use spg_parser::ast::{Program, Stmt};
use spg_parser::Parser;

pub use error::{CollisionKind, ModuleError};
use rename::{NameMap, Renamer};

/// what a module exposes to importers
struct ModuleInfo {
    /// every top-level symbol DECLARED in the module (exported or not.
    /// a non-exported one is still looked up, to say "not exported"
    /// instead of "no such name"). precise sadness.
    symbols: HashMap<String, Symbol>,
    /// names re-exported with `export { x } from "./y.spg"`. kept apart
    /// from `symbols` on purpose: a re-export is importable FROM this
    /// module, but it's not a name INSIDE it, so it must never leak into
    /// this module's own scope. the mangled name is the ORIGINAL
    /// module's, so `a.dois` and `b.dois` (re-exported) are one symbol.
    reexports: HashMap<String, Symbol>,
    /// names this module IMPORTED (not declared, not re-exported). only
    /// used to tell someone who asks for one of them that importing
    /// doesn't re-export it. a much better hint than "no such name"
    imported: HashMap<String, (spg_parser::ast::Pos, String)>,
}

#[derive(Clone)]
struct Symbol {
    mangled: String,
    exported: bool,
    /// where this symbol was declared. the `note:` line of a collision
    pos: spg_parser::ast::Pos,
}

/// libraries written in Rust, enabled by `install <std: "name">` (no file gets read)
const NATIVE_LIBS: [&str; 2] = ["ps", "http"];

/// the declarations a native library needs in the program (the struct its calls return)
fn native_prelude(natives: &[String]) -> Vec<Stmt> {
    let mut stmts = Vec::new();
    if natives.iter().any(|n| n == "ps") {
        let source = spg_lexer::PS_RESULT_SOURCE;
        let tokens = Lexer::new(source).tokenize().expect("the PowerShell prelude lexes");
        let parsed = Parser::new(tokens).parse_program().expect("the PowerShell prelude parses");
        stmts.extend(parsed.stmts);
    }
    if natives.iter().any(|n| n == "http") {
        let tokens = Lexer::new(spg_lexer::HTTP_RESPONSE_SOURCE).tokenize().expect("the HTTP prelude lexes");
        let parsed = Parser::new(tokens).parse_program().expect("the HTTP prelude parses");
        stmts.extend(parsed.stmts);
    }
    stmts
}

struct Resolver {
    /// native libraries installed anywhere in the program
    natives: Vec<String>,
    /// canonical path -> already-resolved module
    loaded: HashMap<PathBuf, ModuleInfo>,
    /// modules currently being resolved, outermost first. finding a
    /// path already in here means the import graph has a cycle
    in_progress: Vec<PathBuf>,
    /// merged output, in dependency order (a module's statements land
    /// before anything that imports it)
    merged: Vec<Stmt>,
    /// the entry file's canonical path (its symbols never get mangled)
    entry: PathBuf,
    /// source text that beats the disk, keyed by canonical
    /// path. how an editor's unsaved buffers get analysed
    overlay: HashMap<PathBuf, String>,
    /// every file read for this program, in load order, with its text. what
    /// `spg build` embeds in the executable
    sources: Vec<(PathBuf, String)>,
}

/// resolves `entry` and everything it transitively imports.
/// folder with the standard library. order: the `SPG_STD` environment
/// variable, then `stdlib/` next to the `spg` executable, then the same
/// folder two levels up (running from the workspace's target/release).
pub fn std_dir() -> Option<PathBuf> {
    if let Some(dir) = std::env::var_os("SPG_STD") {
        return Some(PathBuf::from(dir));
    }
    let exe = std::env::current_exe().ok()?;
    let bin = exe.parent()?;
    [bin.join("stdlib"), bin.join("../../stdlib"), bin.join("../stdlib")]
        .into_iter()
        .find(|p| p.is_dir())
        .and_then(|p| p.canonicalize().ok())
}

/// path of a standard module (`<std: "name">` -> `<std dir>/name.spg`),
/// or a message saying where it was looked for. the search party report.
pub fn standard_module(name: &str) -> Result<PathBuf, String> {
    let dir = std_dir().ok_or_else(|| {
        "standard library folder not found (set SPG_STD, or put a 'stdlib' folder next to spg)".to_string()
    })?;
    // the name is written without the extension (`<std: "terminal">`). `"terminal.spg"` works too
    let stem = name.strip_suffix(".spg").unwrap_or(name);
    let file = dir.join(format!("{stem}.spg"));
    if file.is_file() {
        Ok(file)
    } else {
        Err(format!("no standard module named '{name}' (looked for {})", file.display()))
    }
}

pub fn resolve(entry: &Path) -> Result<Program, ModuleError> {
    resolve_with(entry, &HashMap::new())
}

/// same as `resolve`, but any file in `overlay` (keyed by its
/// canonical path, see `canon`) gets read from there instead of the disk.
pub fn resolve_with(entry: &Path, overlay: &HashMap<PathBuf, String>) -> Result<Program, ModuleError> {
    resolve_collect(entry, overlay).map(|(program, _)| program)
}

/// same as `resolve_with`, and also returns every source file the program
/// was built from (path and text, entry first). an overlay file doesn't need
/// to exist on disk. the path is taken as-is, which is how a bundled program
/// (`spg build`) runs from its embedded sources alone. self-contained, like a hermit.
pub fn resolve_collect(
    entry: &Path,
    overlay: &HashMap<PathBuf, String>,
) -> Result<(Program, Vec<(PathBuf, String)>), ModuleError> {
    let canonical = canonicalize_in(entry, overlay)?;
    let mut r = Resolver {
        loaded: HashMap::new(),
        in_progress: Vec::new(),
        merged: Vec::new(),
        natives: Vec::new(),
        entry: canonical.clone(),
        overlay: overlay.clone(),
        sources: Vec::new(),
    };
    r.load_module(&canonical)?;
    let mut stmts = native_prelude(&r.natives);
    stmts.extend(r.merged);
    Ok((Program { stmts, natives: r.natives }, r.sources))
}

/// `canonicalize`, except that a path in the overlay gets taken lexically
/// (no disk access needed)
fn canonicalize_in(path: &Path, overlay: &HashMap<PathBuf, String>) -> Result<PathBuf, ModuleError> {
    let lexical = lexical_normalize(path);
    if overlay.contains_key(&lexical) {
        return Ok(lexical);
    }
    canonicalize(path)
}

/// removes `.` and resolves `..` without touching the disk
fn lexical_normalize(path: &Path) -> PathBuf {
    use std::path::Component;
    let mut out = PathBuf::new();
    for c in path.components() {
        match c {
            Component::CurDir => {}
            Component::ParentDir => {
                if !out.pop() {
                    out.push(c.as_os_str());
                }
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

/// `std::fs::canonicalize`, minus the `\\?\` verbatim prefix Windows adds
/// (`\\?\C:\dir\a.spg` -> `C:\dir\a.spg`). without this every path in a
/// diagnostic would carry the prefix and never match the current dir. windows, sigh.
pub fn canon(path: &Path) -> std::io::Result<PathBuf> {
    let p = std::fs::canonicalize(path)?;
    Ok(strip_verbatim(p))
}

fn strip_verbatim(p: PathBuf) -> PathBuf {
    let s = p.to_string_lossy();
    if let Some(rest) = s.strip_prefix("\\\\?\\") {
        // \\?\UNC\server\share stays as-is: it has no drive-letter form
        if !rest.starts_with("UNC\\") {
            return PathBuf::from(rest.to_string());
        }
    }
    p
}

fn canonicalize(path: &Path) -> Result<PathBuf, ModuleError> {
    canon(path).map_err(|e| ModuleError::Io { path: path.to_path_buf(), message: e.to_string() })
}

fn read_and_parse(path: &Path, overlay: &HashMap<PathBuf, String>, is_entry: bool) -> Result<(Program, String), ModuleError> {
    let source = match overlay.get(path) {
        Some(text) => text.clone(),
        None => std::fs::read_to_string(path)
            .map_err(|e| ModuleError::Io { path: path.to_path_buf(), message: e.to_string() })?,
    };
    let (tokens, spec) = Lexer::new(&source).tokenize_with_spec().map_err(|error| ModuleError::Lex {
        path: path.to_path_buf(),
        message: error.to_string(),
        error,
    })?;
    if !is_entry {
        if let Some((name, line, col)) = spec.directives.first() {
            return Err(ModuleError::DirectiveInImport { path: path.to_path_buf(), name: name.clone(), line: *line, col: *col });
        }
    }
    let program = Parser::new(tokens)
        .parse_program()
        .map_err(|e| ModuleError::Parse { path: path.to_path_buf(), message: e.to_string(), error: e })?;
    Ok((program, source))
}

impl Resolver {
    /// loads `path` (already canonical), resolving its imports first.
    /// idempotent: a second call for the same module is a no-op, which
    /// is what makes diamond imports load a shared module exactly once.
    fn load_module(&mut self, path: &Path) -> Result<(), ModuleError> {
        if self.loaded.contains_key(path) {
            return Ok(());
        }

        if let Some(start) = self.in_progress.iter().position(|p| p == path) {
            let mut chain: Vec<PathBuf> = self.in_progress[start..].to_vec();
            chain.push(path.to_path_buf());
            return Err(ModuleError::Cycle { chain });
        }
        self.in_progress.push(path.to_path_buf());

        let is_entry = path == self.entry;
        let (mut program, source) = read_and_parse(path, &self.overlay, is_entry)?;
        self.sources.push((path.to_path_buf(), source));
        let is_entry = path == self.entry;
        let dir = path.parent().map(Path::to_path_buf).unwrap_or_default();

        // every Pos in this module gets stamped with the file it came
        // from BEFORE renaming/merging. so any error from here on
        // (collision, later a type error) can say which file it's in,
        // even for the entry file (single-file programs benefit too:
        // an error there now names the file too, not just line/col
        annotate::annotate_file(&mut program.stmts, &error::friendly(path));

        // 1) this module's own top-level symbols and their mangled names
        let mut symbols: HashMap<String, Symbol> = HashMap::new();
        for stmt in &program.stmts {
            if let Some((name, exported, pos)) = declared_symbol(stmt) {
                let mangled = if is_entry { name.clone() } else { mangle(path, &name) };
                if let Some(prev) = symbols.get(&name) {
                    return Err(ModuleError::Collision {
                        file: path.to_path_buf(),
                        name,
                        kind: CollisionKind::LocalVsLocal,
                        second: pos.clone(),
                        first: prev.pos.clone(),
                    });
                }
                symbols.insert(name, Symbol { mangled, exported, pos: pos.clone() });
            }
        }

        // 2) resolve every import / re-export: load the dependency, then
        // either bring its exported names into THIS module's scope
        // (import) or list them as importable from this module
        // (re-export). same validation for both. only the destination
        // differs, which is the whole difference between the two. tidy.
        let mut name_map: NameMap = symbols.iter().map(|(k, v)| (k.clone(), v.mangled.clone())).collect();
        let mut imported_here: HashMap<String, spg_parser::ast::Pos> = HashMap::new();
        // same names as `imported_here`, plus the path each came from
        let mut imported_from: HashMap<String, (spg_parser::ast::Pos, String)> = HashMap::new();
        let mut reexports: HashMap<String, Symbol> = HashMap::new();

        for stmt in &program.stmts {
            let (names, from, pos, is_reexport) = match stmt {
                Stmt::Import { names, from, pos } => (names, from, pos, false),
                Stmt::ReExport { names, from, pos } => (names, from, pos, true),
                _ => continue, // `install` is handled after this loop
            };

            let resolved = self.resolve_names(path, &dir, names, from, pos)?;

            for (name, sym) in resolved {
                if is_reexport {
                    // importable from here, but not a name inside this
                    // module. it may not clash with what this module
                    // itself exports, nor with another re-export.
                    if let Some(local) = symbols.get(&name).filter(|l| l.exported) {
                        return Err(ModuleError::Collision {
                            file: path.to_path_buf(),
                            name,
                            kind: CollisionKind::ReExportVsLocal,
                            second: pos.clone(),
                            first: local.pos.clone(),
                        });
                    }
                    if let Some(prev) = reexports.get(&name) {
                        return Err(ModuleError::Collision {
                            file: path.to_path_buf(),
                            name,
                            kind: CollisionKind::ReExportVsReExport,
                            second: pos.clone(),
                            first: prev.pos.clone(),
                        });
                    }
                    reexports.insert(name, Symbol { mangled: sym.mangled, exported: true, pos: pos.clone() });
                } else {
                    // an import may not collide with the module's own
                    // declarations, or with another import of the same name
                    if let Some(local) = symbols.get(&name) {
                        return Err(ModuleError::Collision {
                            file: path.to_path_buf(),
                            name,
                            kind: CollisionKind::ImportVsLocal,
                            second: pos.clone(),
                            first: local.pos.clone(),
                        });
                    }
                    if let Some(prev) = imported_here.get(&name) {
                        return Err(ModuleError::Collision {
                            file: path.to_path_buf(),
                            name,
                            kind: CollisionKind::ImportVsImport,
                            second: pos.clone(),
                            first: prev.clone(),
                        });
                    }
                    imported_here.insert(name.clone(), pos.clone());
                    imported_from.insert(name.clone(), (pos.clone(), from.clone()));
                    name_map.insert(name, sym.mangled);
                }
            }
        }

        // 2b) `install`: every export of the file, at the LOWEST priority.
        // a name this file declares, or imports by name, is never replaced
        // (so a new export in the library can't break its installers). two
        // installs giving one name different meanings is only an error if
        // the name is USED (found while renaming, below). unused = fine.
        let mut installed: HashMap<String, (String, spg_parser::ast::Pos, String)> = HashMap::new();
        let mut ambiguous: HashMap<String, ((spg_parser::ast::Pos, String), (spg_parser::ast::Pos, String))> = HashMap::new();
        for stmt in &program.stmts {
            let Stmt::Install { from, pos, std } = stmt else { continue };
            // native libraries live in the engine, not in stdlib/. `install <std: "ps">` turns
            // them on (and brings the struct their calls return into the program)
            if *std && NATIVE_LIBS.contains(&from.as_str()) {
                if !self.natives.iter().any(|n| n == from) {
                    self.natives.push(from.clone());
                }
                continue;
            }
            // `install <std: "name">` points at the standard library folder.
            // `install "path"` is a path relative to this file, as before
            let target = if *std {
                let file = standard_module(from).map_err(|message| ModuleError::Io {
                    path: path.to_path_buf(),
                    message,
                })?;
                file.to_string_lossy().into_owned()
            } else {
                from.clone()
            };
            for (name, sym) in self.resolve_all(path, &dir, &target, pos)? {
                if symbols.contains_key(&name) || imported_here.contains_key(&name) {
                    continue;
                }
                match installed.get(&name) {
                    // the same declaration reached twice (e.g. through a re-export). déjà vu.
                    Some((mangled, ..)) if *mangled == sym.mangled => {}
                    Some((_, first_pos, first_from)) => {
                        ambiguous
                            .entry(name)
                            .or_insert(((first_pos.clone(), first_from.clone()), (pos.clone(), from.clone())));
                    }
                    None => {
                        installed.insert(name, (sym.mangled, pos.clone(), from.clone()));
                    }
                }
            }
        }
        for (name, (mangled, ..)) in &installed {
            if !ambiguous.contains_key(name) {
                name_map.insert(name.clone(), mangled.clone());
            }
        }

        // 3) rewrite every declaration and use to the mangled names, drop
        // the (now eaten) import statements, and append to the output
        let mut renamer = Renamer::new(&name_map).with_ambiguous(ambiguous.keys().cloned().collect());
        renamer.rename_program(&mut program.stmts);
        if let Some(name) = renamer.used_ambiguous().into_iter().next() {
            let (first, second) = ambiguous.remove(&name).expect("only ambiguous names are recorded");
            return Err(ModuleError::AmbiguousInstall { file: path.to_path_buf(), name, first, second });
        }
        program.stmts.retain(|s| !matches!(s, Stmt::Import { .. } | Stmt::ReExport { .. } | Stmt::Install { .. }));
        self.merged.extend(program.stmts);

        self.in_progress.pop();
        self.loaded.insert(path.to_path_buf(), ModuleInfo { symbols, reexports, imported: imported_from });
        Ok(())
    }
}

impl Resolver {
    /// loads `from` (relative to the file that mentions it) and looks up
    /// every requested name in what that module exposes to importers.
    /// its exported declarations plus whatever it re-exports. returns the
    /// `(name, symbol)` pairs, or the first thing wrong with the request.
    /// shared by `import` and `export ... from`. both ask the same
    /// question of the dependency. nosy little resolver.
    fn resolve_names(
        &mut self,
        importer: &Path,
        dir: &Path,
        names: &[String],
        from: &str,
        pos: &spg_parser::ast::Pos,
    ) -> Result<Vec<(String, Symbol)>, ModuleError> {
        let dep_path = self.load_dep(importer, dir, from, pos)?;
        let dep = self.loaded.get(&dep_path).expect("load_module just inserted it");
        self.pick_names(dep, importer, names, from, pos)
    }

    /// everything `from` exposes to importers (its exports plus what it
    /// re-exports), sorted by name so results never depend on hash order
    fn resolve_all(
        &mut self,
        importer: &Path,
        dir: &Path,
        from: &str,
        pos: &spg_parser::ast::Pos,
    ) -> Result<Vec<(String, Symbol)>, ModuleError> {
        let dep_path = self.load_dep(importer, dir, from, pos)?;
        let dep = self.loaded.get(&dep_path).expect("load_module just inserted it");
        let mut out: Vec<(String, Symbol)> = dep.reexports.iter().map(|(n, s)| (n.clone(), s.clone())).collect();
        out.extend(dep.symbols.iter().filter(|(_, s)| s.exported).map(|(n, s)| (n.clone(), s.clone())));
        out.sort_by(|a, b| a.0.cmp(&b.0));
        Ok(out)
    }

    /// checks the path, finds the file relative to the IMPORTING file, and loads it
    fn load_dep(
        &mut self,
        importer: &Path,
        dir: &Path,
        from: &str,
        pos: &spg_parser::ast::Pos,
    ) -> Result<PathBuf, ModuleError> {
        if !from.ends_with(".spg") {
            return Err(ModuleError::MissingExtension {
                importer: importer.to_path_buf(),
                path: from.to_string(),
                pos: pos.clone(),
            });
        }

        // the path is relative to the IMPORTING file, not the cwd.
        // a missing file is reported with the path as the user wrote it
        // (not the half-joined `/dir/./x.spg` the join produces). honest paths.
        let dep_path = canonicalize_in(&dir.join(from), &self.overlay).map_err(|e| ModuleError::Io {
            path: PathBuf::from(from),
            message: format!(
                "{} (imported from '{}' at line {}, column {})",
                e,
                error::friendly(importer),
                pos.line,
                pos.col
            ),
        })?;
        self.load_module(&dep_path)?;
        Ok(dep_path)
    }

    fn pick_names(
        &self,
        dep: &ModuleInfo,
        importer: &Path,
        names: &[String],
        from: &str,
        pos: &spg_parser::ast::Pos,
    ) -> Result<Vec<(String, Symbol)>, ModuleError> {
        let mut out = Vec::with_capacity(names.len());
        for name in names {
            // a re-exported name is exactly as importable as a declared
            // one. declared-but-not-exported gets its own error below
            if let Some(sym) = dep.reexports.get(name) {
                out.push((name.clone(), sym.clone()));
                continue;
            }
            let Some(sym) = dep.symbols.get(name) else {
                // it's in that module's scope only because it imported it
                // say so, since "no declaration" would send the user
                // looking at a file where the name plainly appears. ugh, yes.
                if let Some((at, origin)) = dep.imported.get(name) {
                    return Err(ModuleError::ImportedNotReExported {
                        importer: importer.to_path_buf(),
                        module: from.to_string(),
                        name: name.clone(),
                        pos: pos.clone(),
                        imported_at: at.clone(),
                        origin: origin.clone(),
                    });
                }
                return Err(ModuleError::UnknownExport {
                    importer: importer.to_path_buf(),
                    module: from.to_string(),
                    name: name.clone(),
                    pos: pos.clone(),
                });
            };
            if !sym.exported {
                return Err(ModuleError::NotExported {
                    importer: importer.to_path_buf(),
                    module: from.to_string(),
                    name: name.clone(),
                    pos: pos.clone(),
                });
            }
            out.push((name.clone(), sym.clone()));
        }
        Ok(out)
    }
}

/// `(name, exported, pos)` for a statement that declares a module-level
/// symbol. `None` for everything else
fn declared_symbol(stmt: &Stmt) -> Option<(String, bool, &spg_parser::ast::Pos)> {
    match stmt {
        Stmt::FunctionDecl { name, exported, pos, .. } => Some((name.clone(), *exported, pos)),
        Stmt::StructDecl { name, exported, pos, .. } => Some((name.clone(), *exported, pos)),
        Stmt::EnumDecl { name, exported, pos, .. } => Some((name.clone(), *exported, pos)),
        Stmt::Let { name, exported, pos, .. } => Some((name.clone(), *exported, pos)),
        Stmt::Const { name, exported, pos, .. } => Some((name.clone(), *exported, pos)),
        _ => None,
    }
}

/// `<file stem>::<name>`. readable in error messages. the stem alone
/// isn't unique (`a/util.spg` vs `b/util.spg`), so the path's own hash
/// is appended as a tiebreaker. a hash for the drama.
fn mangle(path: &Path, name: &str) -> String {
    let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("module");
    format!("{}#{:x}{}{}", stem, short_hash(path), spg_parser::ast::MANGLE_SEP, name)
}

fn short_hash(path: &Path) -> u32 {
    // FNV-1a over the path bytes. stable across runs (unlike the
    // std HashMap hasher), so mangled names don't change run to run
    let mut h: u32 = 0x811c9dc5;
    for b in path.to_string_lossy().as_bytes() {
        h ^= *b as u32;
        h = h.wrapping_mul(0x01000193);
    }
    h & 0xffff
}

#[cfg(test)]
mod verbatim_tests {
    use super::strip_verbatim;
    use std::path::PathBuf;

    #[test]
    fn strips_drive_prefix() {
        let p = strip_verbatim(PathBuf::from(r"\\?\C:\spaghetti-prot\a.spg"));
        assert_eq!(p, PathBuf::from(r"C:\spaghetti-prot\a.spg"));
    }

    #[test]
    fn keeps_unc_and_plain() {
        let unc = PathBuf::from(r"\\?\UNC\srv\share\a.spg");
        assert_eq!(strip_verbatim(unc.clone()), unc);
        let plain = PathBuf::from("/home/x/a.spg");
        assert_eq!(strip_verbatim(plain.clone()), plain);
    }
}
