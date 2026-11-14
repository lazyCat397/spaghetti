//! `net`: TCP sockets (`load #net`). a client connects, a server listens and accepts. a
//! connection or a listener is a number: keep it in a variable. the calls wait for the network:
//! `readLine` waits for a line, `accept` for a client (`setTimeout` bounds the reads and writes).
//! a closed connection makes `readLine` fail; `read` gives the empty text instead.

use std::collections::HashMap;
use std::io::{self, BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::{CallCtx, Kind, LibError, LibValue, NativeFn, NativeModule};

pub static MODULE: NativeModule = NativeModule {
    kind: Kind::Load,
    name: "net",
    doc: "TCP sockets: connect, listen, accept, send and read lines (load #net)",
    structs: &[],
    objects: &[],
    functions: &[
        NativeFn { name: "connect", sig: "(string, int) -> int", doc: "connect to a host and port. gives back the connection's number", call: connect },
        NativeFn { name: "listen", sig: "(string, int) -> int", doc: "listen on a host and port: \"127.0.0.1\" for this machine only, \"0.0.0.0\" for the network. gives back the listener's number", call: listen },
        NativeFn { name: "accept", sig: "(int) -> int", doc: "wait for a client on a listener. gives back the new connection's number", call: accept },
        NativeFn { name: "send", sig: "(int, string) -> void", doc: "send text over a connection", call: send },
        NativeFn { name: "readLine", sig: "(int) -> string", doc: "read one line, without its line break. fails when the other side has closed", call: read_line },
        NativeFn { name: "read", sig: "(int, int) -> string", doc: "read up to n bytes as text. the empty text means the other side has closed", call: read },
        NativeFn { name: "setTimeout", sig: "(int, int) -> void", doc: "how many milliseconds reads and writes may wait before they fail", call: set_timeout },
        NativeFn { name: "peer", sig: "(int) -> string", doc: "the address of the other side of a connection", call: peer },
        NativeFn { name: "close", sig: "(int) -> void", doc: "close a connection or a listener", call: close },
    ],
};

enum Handle {
    Stream(BufReader<TcpStream>),
    Listener(TcpListener),
}

/// the open handles by number. each one has its own lock, so a read that waits on one
/// connection never holds up the others
#[derive(Default)]
struct Table {
    next: i64,
    items: HashMap<i64, Arc<Mutex<Handle>>>,
}

static TABLE: Mutex<Option<Table>> = Mutex::new(None);

fn put(handle: Handle) -> i64 {
    let mut guard = TABLE.lock().unwrap_or_else(|e| e.into_inner());
    let table = guard.get_or_insert_with(Table::default);
    table.next += 1;
    let id = table.next;
    table.items.insert(id, Arc::new(Mutex::new(handle)));
    id
}

/// the handle behind a number. the table is held only for this lookup, never while waiting
fn handle(function: &'static str, id: i64) -> Result<Arc<Mutex<Handle>>, LibError> {
    let guard = TABLE.lock().unwrap_or_else(|e| e.into_inner());
    guard
        .as_ref()
        .and_then(|t| t.items.get(&id).cloned())
        .ok_or_else(|| LibError::new(function, format!("no open connection or listener number {}", id)))
}

/// runs `work` on a connection. a listener number is refused
fn with_stream<T>(function: &'static str, id: i64, work: impl FnOnce(&mut BufReader<TcpStream>) -> io::Result<T>) -> Result<T, LibError> {
    let shared = handle(function, id)?;
    let mut guard = shared.lock().unwrap_or_else(|e| e.into_inner());
    match &mut *guard {
        Handle::Stream(stream) => work(stream).map_err(|e| explain(function, e)),
        Handle::Listener(_) => Err(LibError::new(function, format!("number {} is a listener, not a connection", id))),
    }
}

/// an io error as a sentence a program can show
fn explain(function: &'static str, e: io::Error) -> LibError {
    let reason = match e.kind() {
        io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock => "timed out".to_string(),
        io::ErrorKind::UnexpectedEof => "the other side closed the connection".to_string(),
        io::ErrorKind::ConnectionRefused => "connection refused".to_string(),
        io::ErrorKind::ConnectionReset => "connection reset by the other side".to_string(),
        io::ErrorKind::InvalidData => "received bytes that are not UTF-8 text".to_string(),
        _ => e.to_string(),
    };
    LibError::new(function, reason)
}

fn text(a: &[LibValue], i: usize) -> &str {
    match a.get(i) {
        Some(LibValue::Str(t)) => t.as_str(),
        _ => "",
    }
}

fn int(a: &[LibValue], i: usize) -> i64 {
    match a.get(i) {
        Some(LibValue::Int(n)) => *n,
        _ => 0,
    }
}

fn port_arg(function: &'static str, a: &[LibValue], i: usize) -> Result<u16, LibError> {
    let n = int(a, i);
    u16::try_from(n).map_err(|_| LibError::new(function, format!("{} is not a port (0 to 65535)", n)))
}

fn connect(_: &CallCtx, a: Vec<LibValue>) -> Result<LibValue, LibError> {
    let port = port_arg("net.connect", &a, 1)?;
    let stream = TcpStream::connect((text(&a, 0), port)).map_err(|e| explain("net.connect", e))?;
    Ok(LibValue::Int(put(Handle::Stream(BufReader::new(stream)))))
}

fn listen(_: &CallCtx, a: Vec<LibValue>) -> Result<LibValue, LibError> {
    let port = port_arg("net.listen", &a, 1)?;
    let listener = TcpListener::bind((text(&a, 0), port)).map_err(|e| explain("net.listen", e))?;
    Ok(LibValue::Int(put(Handle::Listener(listener))))
}

fn accept(_: &CallCtx, a: Vec<LibValue>) -> Result<LibValue, LibError> {
    let id = int(&a, 0);
    let shared = handle("net.accept", id)?;
    let guard = shared.lock().unwrap_or_else(|e| e.into_inner());
    let stream = match &*guard {
        Handle::Listener(listener) => listener.accept().map_err(|e| explain("net.accept", e))?.0,
        Handle::Stream(_) => return Err(LibError::new("net.accept", format!("number {} is a connection, not a listener", id))),
    };
    Ok(LibValue::Int(put(Handle::Stream(BufReader::new(stream)))))
}

fn send(_: &CallCtx, a: Vec<LibValue>) -> Result<LibValue, LibError> {
    let data = text(&a, 1).to_string();
    with_stream("net.send", int(&a, 0), |r| {
        r.get_mut().write_all(data.as_bytes())?;
        r.get_mut().flush()
    })?;
    Ok(LibValue::Void)
}

fn read_line(_: &CallCtx, a: Vec<LibValue>) -> Result<LibValue, LibError> {
    let line = with_stream("net.readLine", int(&a, 0), |r| {
        let mut line = String::new();
        if r.read_line(&mut line)? == 0 {
            return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "closed"));
        }
        Ok(line.trim_end_matches(['\r', '\n']).to_string())
    })?;
    Ok(LibValue::Str(line))
}

fn read(_: &CallCtx, a: Vec<LibValue>) -> Result<LibValue, LibError> {
    let max = int(&a, 1);
    if !(1..=1_048_576).contains(&max) {
        return Err(LibError::new("net.read", "the size must be from 1 to 1048576 bytes"));
    }
    let bytes = with_stream("net.read", int(&a, 0), |r| {
        let mut buf = vec![0u8; max as usize];
        let n = r.read(&mut buf)?;
        buf.truncate(n);
        Ok(buf)
    })?;
    Ok(LibValue::Str(String::from_utf8_lossy(&bytes).into_owned()))
}

fn set_timeout(_: &CallCtx, a: Vec<LibValue>) -> Result<LibValue, LibError> {
    let ms = int(&a, 1);
    if ms <= 0 {
        return Err(LibError::new("net.setTimeout", "the timeout must be more than 0 milliseconds"));
    }
    let wait = Some(Duration::from_millis(ms as u64));
    with_stream("net.setTimeout", int(&a, 0), |r| {
        r.get_ref().set_read_timeout(wait)?;
        r.get_ref().set_write_timeout(wait)
    })?;
    Ok(LibValue::Void)
}

fn peer(_: &CallCtx, a: Vec<LibValue>) -> Result<LibValue, LibError> {
    let addr = with_stream("net.peer", int(&a, 0), |r| r.get_ref().peer_addr())?;
    Ok(LibValue::Str(addr.to_string()))
}

fn close(_: &CallCtx, a: Vec<LibValue>) -> Result<LibValue, LibError> {
    let id = int(&a, 0);
    let mut guard = TABLE.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(table) = guard.as_mut() {
        table.items.remove(&id);
    }
    Ok(LibValue::Void)
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

    /// a port nobody is using right now (bind to 0, read the port, let go)
    fn free_port() -> i64 {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        l.local_addr().unwrap().port() as i64
    }

    #[test]
    fn a_line_goes_to_a_server_thread_and_comes_back_upper_case() {
        let port = free_port();
        let LibValue::Int(listener) = listen(&ctx(), vec![s("127.0.0.1"), LibValue::Int(port)]).unwrap() else {
            panic!("not a number");
        };
        let server = std::thread::spawn(move || {
            let LibValue::Int(conn) = accept(&CallCtx { caller_file: None, caller_line: 0 }, vec![LibValue::Int(listener)]).unwrap() else {
                panic!("not a number");
            };
            let LibValue::Str(line) = read_line(&CallCtx { caller_file: None, caller_line: 0 }, vec![LibValue::Int(conn)]).unwrap() else {
                panic!("not text");
            };
            send(&CallCtx { caller_file: None, caller_line: 0 }, vec![LibValue::Int(conn), s(&format!("{}\n", line.to_uppercase()))]).unwrap();
        });
        let LibValue::Int(conn) = connect(&ctx(), vec![s("127.0.0.1"), LibValue::Int(port)]).unwrap() else {
            panic!("not a number");
        };
        send(&ctx(), vec![LibValue::Int(conn), s("ola\n")]).unwrap();
        assert_eq!(read_line(&ctx(), vec![LibValue::Int(conn)]).unwrap(), s("OLA"));
        server.join().unwrap();
    }

    #[test]
    fn a_port_out_of_range_is_refused_before_connecting() {
        let err = connect(&ctx(), vec![s("127.0.0.1"), LibValue::Int(70000)]).unwrap_err();
        assert_eq!(err.function, "net.connect");
        assert!(err.reason.contains("not a port"), "{}", err.reason);
    }
}
