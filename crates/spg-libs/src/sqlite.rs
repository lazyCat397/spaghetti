//! `sqlite`: a database in one file (`load #sqlite`). a database is named by its path, and
//! `":memory:"` makes one that lives only for the run. the connection opens on first use and
//! stays open for the rest of the run. values come back as text: read a number with `@into`.
//! put values into SQL through `?` and the array, never by joining text into the SQL, so a name
//! like O'Neil can not break a query.

use std::collections::HashMap;
use std::sync::Mutex;

use rusqlite::types::ValueRef;
use rusqlite::Connection;

use crate::{CallCtx, Kind, LibError, LibValue, NativeFn, NativeModule};

pub static MODULE: NativeModule = NativeModule {
    kind: Kind::Load,
    name: "sqlite",
    doc: "a database in one file: exec and query (load #sqlite)",
    structs: &[],
    objects: &[],
    functions: &[
        NativeFn { name: "exec", sig: "(string, string) -> void", doc: "run SQL. several statements are fine. the file is created if it is not there", call: exec },
        NativeFn { name: "exec", sig: "(string, string, array<string>) -> int", doc: "run one statement with ? filled in order from the array. gives back how many rows it changed", call: exec_params },
        NativeFn { name: "query", sig: "(string, string) -> array<map<string, string>>", doc: "the rows of a SELECT, one map per row keyed by column name. every value is text", call: query },
        NativeFn { name: "query", sig: "(string, string, array<string>) -> array<map<string, string>>", doc: "the same, with ? filled in order from the array", call: query_params },
    ],
};

/// the open connections, by database path. `None` until the first database is opened
static CONNECTIONS: Mutex<Option<HashMap<String, Connection>>> = Mutex::new(None);

fn text(a: &[LibValue], i: usize) -> &str {
    match a.get(i) {
        Some(LibValue::Str(t)) => t.as_str(),
        _ => "",
    }
}

fn values(a: &[LibValue], i: usize) -> Vec<String> {
    match a.get(i) {
        Some(LibValue::Array(items)) => items
            .iter()
            .map(|v| match v {
                LibValue::Str(s) => s.clone(),
                _ => String::new(),
            })
            .collect(),
        _ => Vec::new(),
    }
}

/// runs `work` on the connection to `path`, opening that database first if it is new
fn with_db<T>(function: &'static str, path: &str, work: impl FnOnce(&Connection) -> rusqlite::Result<T>) -> Result<T, LibError> {
    let mut guard = CONNECTIONS.lock().unwrap_or_else(|e| e.into_inner());
    let open = guard.get_or_insert_with(HashMap::new);
    if !open.contains_key(path) {
        let conn = Connection::open(path).map_err(|e| LibError::io(function, path, e.to_string()))?;
        open.insert(path.to_string(), conn);
    }
    let conn = open
        .get(path)
        .ok_or_else(|| LibError::new(function, "internal: the connection was not kept"))?;
    work(conn).map_err(|e| LibError::new(function, e.to_string()))
}

fn exec(_: &CallCtx, a: Vec<LibValue>) -> Result<LibValue, LibError> {
    let sql = text(&a, 1).to_string();
    with_db("sqlite.exec", text(&a, 0), |c| c.execute_batch(&sql))?;
    Ok(LibValue::Void)
}

fn exec_params(_: &CallCtx, a: Vec<LibValue>) -> Result<LibValue, LibError> {
    let sql = text(&a, 1).to_string();
    let vals = values(&a, 2);
    let changed = with_db("sqlite.exec", text(&a, 0), |c| c.execute(&sql, rusqlite::params_from_iter(vals.iter())))?;
    Ok(LibValue::Int(changed as i64))
}

fn query(_: &CallCtx, a: Vec<LibValue>) -> Result<LibValue, LibError> {
    run_query(&a, Vec::new())
}

fn query_params(_: &CallCtx, a: Vec<LibValue>) -> Result<LibValue, LibError> {
    let vals = values(&a, 2);
    run_query(&a, vals)
}

fn run_query(a: &[LibValue], vals: Vec<String>) -> Result<LibValue, LibError> {
    let sql = text(a, 1).to_string();
    with_db("sqlite.query", text(a, 0), |c| -> rusqlite::Result<LibValue> {
        let mut stmt = c.prepare(&sql)?;
        let columns: Vec<String> = stmt.column_names().iter().map(|n| n.to_string()).collect();
        let mut rows = stmt.query(rusqlite::params_from_iter(vals.iter()))?;
        let mut out = Vec::new();
        while let Some(row) = rows.next()? {
            let mut fields = Vec::with_capacity(columns.len());
            for (i, name) in columns.iter().enumerate() {
                fields.push((LibValue::Str(name.clone()), LibValue::Str(cell_text(row.get_ref(i)?))));
            }
            out.push(LibValue::Map(fields));
        }
        Ok(LibValue::Array(out))
    })
}

/// a cell as text. NULL is the empty text; a blob shows its size, not its bytes
fn cell_text(v: ValueRef<'_>) -> String {
    match v {
        ValueRef::Null => String::new(),
        ValueRef::Integer(n) => n.to_string(),
        ValueRef::Real(x) => x.to_string(),
        ValueRef::Text(t) => String::from_utf8_lossy(t).into_owned(),
        ValueRef::Blob(b) => format!("<blob of {} bytes>", b.len()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx() -> CallCtx<'static> {
        CallCtx { caller_file: None, caller_line: 0 }
    }

    fn s(t: &str) -> LibValue {
        LibValue::Str(t.to_string())
    }

    fn list(items: &[&str]) -> LibValue {
        LibValue::Array(items.iter().map(|i| s(i)).collect())
    }

    /// a database file of its own for each test, so the shared connections never mix them
    fn temp_db(name: &str) -> String {
        let path = std::env::temp_dir().join(format!("spg_sqlite_{}_{}.db", name, std::process::id()));
        let _ = std::fs::remove_file(&path);
        path.to_string_lossy().into_owned()
    }

    #[test]
    fn values_go_in_as_parameters_and_come_back_as_text() {
        let db = temp_db("params");
        exec(&ctx(), vec![s(&db), s("CREATE TABLE gente (nome TEXT, idade INTEGER)")]).unwrap();
        let changed = exec_params(&ctx(), vec![s(&db), s("INSERT INTO gente VALUES (?, ?)"), list(&["O'Neil", "42"])]).unwrap();
        assert_eq!(changed, LibValue::Int(1));
        let LibValue::Array(rows) = query(&ctx(), vec![s(&db), s("SELECT nome, idade FROM gente")]).unwrap() else {
            panic!("not an array");
        };
        assert_eq!(rows.len(), 1);
        let LibValue::Map(fields) = &rows[0] else { panic!("not a map") };
        assert_eq!(fields[0], (s("nome"), s("O'Neil")));
        assert_eq!(fields[1], (s("idade"), s("42")));
    }

    #[test]
    fn a_bad_statement_is_an_error_with_sqlites_message() {
        let db = temp_db("bad");
        let err = query(&ctx(), vec![s(&db), s("SELECT * FROM nada")]).unwrap_err();
        assert_eq!(err.function, "sqlite.query");
        assert!(err.reason.contains("no such table"), "{}", err.reason);
    }
}
