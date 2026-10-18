//! `@http`: the blocking client behind `@http.get` and `@http.post`. ureq 3 with rustls
//! (TLS through the OS trust store, so corporate CAs work like in a browser).
//! a non-2xx answer is NOT an error here: the program gets the status and decides.
//! only failures to get an answer at all (DNS, refused, timeout, TLS) are errors.

use std::time::Duration;

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
