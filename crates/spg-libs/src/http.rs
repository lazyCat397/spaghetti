//! `http`: the blocking client behind `http.get` and `http.post` (`install #http`).
//! ureq 3 with rustls (TLS through the OS trust store, so corporate CAs work like in a browser).
//! a non-2xx answer is NOT an error here: the program gets the status and decides.
//! only failures to get an answer at all (DNS, refused, timeout, TLS) are errors.

use std::time::Duration;

use crate::{CallCtx, Kind, LibError, LibValue, NativeFn, NativeModule};

const GET_DOC: &str = "GET a url, optional headers map and timeout in seconds (default 30). gives back an HttpResponse (status, headers, body). a 404 is a normal answer; only no answer at all throws";
const POST_DOC: &str = "POST a body to a url, optional headers map and timeout in seconds. gives back an HttpResponse";

pub static MODULE: NativeModule = NativeModule {
    kind: Kind::Install,
    name: "http",
    doc: "web requests (install #http)",
    structs: &["struct HttpResponse {\n    status: int,\n    headers: map<string, string>,\n    body: string\n}\n"],
    objects: &[],
    functions: &[
        NativeFn { name: "get", sig: "(string) -> HttpResponse", doc: GET_DOC, call: get },
        NativeFn { name: "get", sig: "(string, int) -> HttpResponse", doc: GET_DOC, call: get },
        NativeFn { name: "get", sig: "(string, map<string, string>) -> HttpResponse", doc: GET_DOC, call: get },
        NativeFn { name: "get", sig: "(string, map<string, string>, int) -> HttpResponse", doc: GET_DOC, call: get },
        NativeFn { name: "post", sig: "(string, string) -> HttpResponse", doc: POST_DOC, call: post },
        NativeFn { name: "post", sig: "(string, string, int) -> HttpResponse", doc: POST_DOC, call: post },
        NativeFn { name: "post", sig: "(string, string, map<string, string>) -> HttpResponse", doc: POST_DOC, call: post },
        NativeFn { name: "post", sig: "(string, string, map<string, string>, int) -> HttpResponse", doc: POST_DOC, call: post },
    ],
};

fn get(_: &CallCtx, args: Vec<LibValue>) -> Result<LibValue, LibError> {
    call(args, false)
}

fn post(_: &CallCtx, args: Vec<LibValue>) -> Result<LibValue, LibError> {
    call(args, true)
}

/// the optional arguments come in any order the typechecker allowed: an int is the timeout
/// (seconds), a map is the headers, and for POST the string after the url is the body.
/// builds the `HttpResponse` (status, headers, body), in that order. only a failure to get an
/// answer is an error; a 404 or 500 is a normal response.
fn call(args: Vec<LibValue>, post: bool) -> Result<LibValue, LibError> {
    let function = if post { "http.post" } else { "http.get" };
    let mut rest = args.into_iter();
    let url = match rest.next() {
        Some(LibValue::Str(s)) => s,
        _ => return Err(LibError::new(function, "expected a url (string)")),
    };
    let mut body: Option<String> = None;
    let mut headers: Vec<(String, String)> = Vec::new();
    let mut timeout: i64 = 30;
    for extra in rest {
        match extra {
            LibValue::Str(s) if post && body.is_none() => body = Some(s),
            LibValue::Int(n) => timeout = n,
            LibValue::Map(entries) => {
                for (k, v) in entries {
                    match (k, v) {
                        (LibValue::Str(k), LibValue::Str(v)) => headers.push((k, v)),
                        _ => return Err(LibError::new(function, "the headers must be strings")),
                    }
                }
            }
            _ => return Err(LibError::new(function, "expected a timeout, headers or a body")),
        }
    }
    if timeout <= 0 {
        return Err(LibError::new(function, "the timeout must be at least 1 second"));
    }
    if post && body.is_none() {
        body = Some(String::new());
    }
    let answer = request(&url, body.as_deref(), &headers, timeout as u64).map_err(|reason| LibError::new(function, reason))?;
    Ok(LibValue::record(
        "HttpResponse",
        vec![
            ("status", LibValue::Int(i64::from(answer.status))),
            ("headers", LibValue::Map(answer.headers.into_iter().map(|(k, v)| (LibValue::Str(k), LibValue::Str(v))).collect())),
            ("body", LibValue::Str(answer.body)),
        ],
    ))
}

/// what a request to `@http` sends back to the program
pub struct Response {
    pub status: u16,
    /// header names are lowercase. a repeated header keeps its last value.
    pub headers: Vec<(String, String)>,
    pub body: String,
}

/// the User-Agent sent when the program doesn't set one (some APIs refuse requests without it)
fn default_user_agent() -> String {
    format!("spg/{}", env!("CARGO_PKG_VERSION"))
}

/// one request. `body` is Some for POST, None for GET. `headers` are the program's own,
/// and win over the defaults. `timeout_secs` covers the whole request.
pub fn request(
    url: &str,
    body: Option<&str>,
    headers: &[(String, String)],
    timeout_secs: u64,
) -> Result<Response, String> {
    let config = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(timeout_secs)))
        .http_status_as_error(false)
        .tls_config(ureq::tls::TlsConfig::builder().root_certs(ureq::tls::RootCerts::PlatformVerifier).build())
        .build();
    let agent: ureq::Agent = config.into();

    // the program's headers, plus the default User-Agent when it didn't send one
    let mut all: Vec<(String, String)> = headers.to_vec();
    if !all.iter().any(|(k, _)| k.eq_ignore_ascii_case("user-agent")) {
        all.push(("user-agent".to_string(), default_user_agent()));
    }

    let mut answer = match body {
        Some(data) => {
            let mut req = agent.post(url);
            for (k, v) in &all {
                req = req.header(k.as_str(), v.as_str());
            }
            req.send(data).map_err(|e| e.to_string())?
        }
        None => {
            let mut req = agent.get(url);
            for (k, v) in &all {
                req = req.header(k.as_str(), v.as_str());
            }
            req.call().map_err(|e| e.to_string())?
        }
    };

    let status = answer.status().as_u16();
    let out_headers = answer
        .headers()
        .iter()
        .map(|(name, value)| (name.as_str().to_string(), String::from_utf8_lossy(value.as_bytes()).into_owned()))
        .collect();
    let text = answer.body_mut().read_to_string().map_err(|e| e.to_string())?;
    Ok(Response { status, headers: out_headers, body: text })
}
