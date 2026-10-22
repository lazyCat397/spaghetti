//! `csv`: comma-separated text (`install #csv`). quotes, commas and line breaks inside a field
//! are handled by the `csv` crate. `parse` takes the first line as the column names and gives
//! back one map per row, so `row["name"]` reads a column. a short row fills its missing columns
//! with "", and extra fields past the last column are dropped.

use crate::{CallCtx, Kind, LibError, LibValue, NativeFn, NativeModule};

pub static MODULE: NativeModule = NativeModule {
    kind: Kind::Install,
    name: "csv",
    doc: "comma-separated text: parse and write (install #csv)",
    structs: &[],
    functions: &[
        NativeFn { name: "parse", sig: "(string) -> array<map<string, string>>", doc: "the rows of a CSV text, one map per row, keyed by the first line's column names", call: parse },
        NativeFn { name: "rows", sig: "(string) -> array<array<string>>", doc: "the rows of a CSV text as plain lists of fields (the first line included)", call: rows },
        NativeFn { name: "stringify", sig: "(array<map<string, string>>) -> string", doc: "CSV text from maps. the columns are the keys of the first map", call: stringify },
        NativeFn { name: "stringifyRows", sig: "(array<array<string>>) -> string", doc: "CSV text from lists of fields, one line per list", call: stringify_rows },
    ],
};

fn text(a: &[LibValue]) -> &str {
    match a.first() {
        Some(LibValue::Str(t)) => t.as_str(),
        _ => "",
    }
}

fn items(a: &[LibValue]) -> &[LibValue] {
    match a.first() {
        Some(LibValue::Array(v)) => v.as_slice(),
        _ => &[],
    }
}

/// a value as text, or "" for anything that is not text (the typechecker keeps this from happening)
fn as_text(v: &LibValue) -> String {
    match v {
        LibValue::Str(s) => s.clone(),
        _ => String::new(),
    }
}

/// the value under a key in a map (`row["name"]`), or "" when the column is missing
fn field(row: &LibValue, key: &str) -> String {
    match row {
        LibValue::Map(pairs) => pairs
            .iter()
            .find(|(k, _)| matches!(k, LibValue::Str(s) if s == key))
            .map(|(_, v)| as_text(v))
            .unwrap_or_default(),
        _ => String::new(),
    }
}

fn parse(_: &CallCtx, a: Vec<LibValue>) -> Result<LibValue, LibError> {
    let mut reader = csv::ReaderBuilder::new()
        .has_headers(true)
        .flexible(true)
        .from_reader(text(&a).as_bytes());
    let headers: Vec<String> = reader
        .headers()
        .map_err(|e| LibError::new("csv.parse", e.to_string()))?
        .iter()
        .map(str::to_string)
        .collect();
    let mut out = Vec::new();
    for record in reader.records() {
        let record = record.map_err(|e| LibError::new("csv.parse", e.to_string()))?;
        let pairs = headers
            .iter()
            .enumerate()
            .map(|(i, h)| (LibValue::Str(h.clone()), LibValue::Str(record.get(i).unwrap_or("").to_string())))
            .collect();
        out.push(LibValue::Map(pairs));
    }
    Ok(LibValue::Array(out))
}

fn rows(_: &CallCtx, a: Vec<LibValue>) -> Result<LibValue, LibError> {
    let mut reader = csv::ReaderBuilder::new()
        .has_headers(false)
        .flexible(true)
        .from_reader(text(&a).as_bytes());
    let mut out = Vec::new();
    for record in reader.records() {
        let record = record.map_err(|e| LibError::new("csv.rows", e.to_string()))?;
        out.push(LibValue::Array(record.iter().map(|f| LibValue::Str(f.to_string())).collect()));
    }
    Ok(LibValue::Array(out))
}

fn stringify(_: &CallCtx, a: Vec<LibValue>) -> Result<LibValue, LibError> {
    let rows = items(&a);
    // the columns: the keys of the first row, in order
    let columns: Vec<String> = match rows.first() {
        Some(LibValue::Map(pairs)) => pairs.iter().map(|(k, _)| as_text(k)).collect(),
        _ => return Ok(LibValue::Str(String::new())),
    };
    let mut writer = csv::Writer::from_writer(Vec::new());
    writer.write_record(&columns).map_err(|e| LibError::new("csv.stringify", e.to_string()))?;
    for row in rows {
        let line: Vec<String> = columns.iter().map(|c| field(row, c)).collect();
        writer.write_record(&line).map_err(|e| LibError::new("csv.stringify", e.to_string()))?;
    }
    finish("csv.stringify", writer)
}

fn stringify_rows(_: &CallCtx, a: Vec<LibValue>) -> Result<LibValue, LibError> {
    let mut writer = csv::Writer::from_writer(Vec::new());
    for row in items(&a) {
        let line: Vec<String> = match row {
            LibValue::Array(fields) => fields.iter().map(as_text).collect(),
            _ => Vec::new(),
        };
        writer.write_record(&line).map_err(|e| LibError::new("csv.stringifyRows", e.to_string()))?;
    }
    finish("csv.stringifyRows", writer)
}

fn finish(function: &'static str, writer: csv::Writer<Vec<u8>>) -> Result<LibValue, LibError> {
    let bytes = writer.into_inner().map_err(|e| LibError::new(function, e.error().to_string()))?;
    Ok(LibValue::Str(String::from_utf8_lossy(&bytes).into_owned()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::LibFn;

    fn call(f: LibFn, v: LibValue) -> Result<LibValue, LibError> {
        f(&CallCtx { caller_file: None, caller_line: 0 }, vec![v])
    }

    fn st(s: &str) -> LibValue {
        LibValue::Str(s.to_string())
    }

    #[test]
    fn parse_keys_rows_by_the_header_and_handles_quotes() {
        let text = "nome,obs\n\"Ana\",\"oi, tudo bem\"\nBia\n";
        let LibValue::Array(rows) = call(parse, st(text)).unwrap() else { panic!("not an array") };
        assert_eq!(rows.len(), 2);
        assert_eq!(field(&rows[0], "obs"), "oi, tudo bem");
        // the short row: its missing column is empty
        assert_eq!(field(&rows[1], "nome"), "Bia");
        assert_eq!(field(&rows[1], "obs"), "");
    }

    #[test]
    fn stringify_writes_the_header_and_quotes_what_it_must() {
        let row = LibValue::Map(vec![
            (st("nome"), st("Ana")),
            (st("obs"), st("a, b")),
        ]);
        let out = call(stringify, LibValue::Array(vec![row])).unwrap();
        assert_eq!(out, st("nome,obs\nAna,\"a, b\"\n"));
        assert_eq!(call(stringify, LibValue::Array(vec![])).unwrap(), st(""));
    }

    #[test]
    fn rows_and_stringify_rows_round_trip() {
        let LibValue::Array(rows) = call(rows, st("1,2\n3,4\n")).unwrap() else { panic!() };
        assert_eq!(rows.len(), 2);
        let back = call(stringify_rows, LibValue::Array(rows)).unwrap();
        assert_eq!(back, st("1,2\n3,4\n"));
    }
}
