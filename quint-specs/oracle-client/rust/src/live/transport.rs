//! HTTP to the oracle daemon over one process-wide pooled agent.

use std::fmt;
use std::sync::OnceLock;

const PROTOCOL_HEADER: &str = "Quint-Oracle-Protocol";

static AGENT: OnceLock<ureq::Agent> = OnceLock::new();

fn agent() -> &'static ureq::Agent {
    AGENT.get_or_init(|| {
        ureq::Agent::config_builder()
            // Non-2xx is diagnosed by us, not raised: the body names what went
            // wrong (a 426 the expected protocol version, a 422 the missing
            // component scope).
            .http_status_as_error(false)
            .build()
            .into()
    })
}

/// A failed send, split by whether the daemon answered.
pub(crate) enum SendError {
    /// The daemon answered with a non-2xx: it is alive and refusing this
    /// request. Always an instrumentation bug, never transient.
    Rejected(String),
    /// No answer — connection refused or reset. The daemon may be down,
    /// starting, or restarting. Nothing times out here, so this is never a
    /// daemon that was merely slow.
    Unreachable(String),
}

impl fmt::Display for SendError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SendError::Rejected(message) | SendError::Unreachable(message) => f.write_str(message),
        }
    }
}

/// The two run outcomes the daemon accepts, spelled as it deserializes them:
/// `{"status": "ok" | "failed"}`, lowercase and case-sensitive.
pub(crate) enum RunStatus {
    Ok,
    Failed,
}

impl RunStatus {
    fn to_json(&self) -> serde_json::Value {
        let status = match self {
            RunStatus::Ok => "ok",
            RunStatus::Failed => "failed",
        };
        serde_json::json!({ "status": status })
    }
}

/// POST one event body to `/test/{name}`.
pub(crate) fn post_event(test: &str, body: &serde_json::Value) -> Result<(), SendError> {
    send(Method::Post, test, body)
}

/// PATCH the run's outcome.
pub(crate) fn patch_status(test: &str, status: RunStatus) -> Result<(), SendError> {
    send(Method::Patch, test, &status.to_json())
}

enum Method {
    Post,
    Patch,
}

impl Method {
    fn name(&self) -> &'static str {
        match self {
            Method::Post => "POST",
            Method::Patch => "PATCH",
        }
    }
}

/// `Err` carries the whole formatted diagnosis, so call sites need no
/// knowledge of the protocol to report it. An unconfigured daemon is `Ok`:
/// the crate is inert, not broken.
fn send(method: Method, test: &str, body: &serde_json::Value) -> Result<(), SendError> {
    let Some(config) = super::config() else {
        return Ok(()); // env unset: no-op
    };
    let url = format!("{}/test/{}", config.base_url, encode_path(test));
    let request = match method {
        Method::Post => agent().post(&url),
        Method::Patch => agent().patch(&url),
    };
    let result = request
        .header(PROTOCOL_HEADER, crate::PROTOCOL_VERSION.to_string())
        .header("Content-Type", "application/json")
        .send(body.to_string());
    match result {
        Ok(mut response) => {
            // Drain so the connection returns to the pool; the body doubles
            // as the diagnosis (a 426 names the expected protocol version).
            let text = response.body_mut().read_to_string().unwrap_or_default();
            if response.status().is_success() {
                Ok(())
            } else {
                Err(SendError::Rejected(format!(
                    "{} {} -> {}: {}",
                    method.name(),
                    url,
                    response.status(),
                    text.trim()
                )))
            }
        }
        Err(error) => Err(SendError::Unreachable(format!(
            "{} {} failed: {error}",
            method.name(),
            url
        ))),
    }
}

/// Percent-encode one URL path segment (RFC 3986 unreserved bytes stay raw;
/// everything else — `/`, `%`, spaces, non-ASCII bytes — is `%XX`-escaped).
/// Test names are `::`-separated Rust paths, so escaping is the common case.
///
/// The daemon percent-decodes the segment back into bytes and reads those as
/// UTF-8 (`decode_percent` in `crates/oracle/src/server.rs`), so a name
/// round-trips intact — non-ASCII included, since a multi-byte character is
/// simply several escapes.
pub(crate) fn encode_path(segment: &str) -> String {
    let mut out = String::with_capacity(segment.len());
    for byte in segment.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(byte as char)
            }
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn path_segments_percent_encode_everything_but_unreserved_bytes() {
        assert_eq!(encode_path("simple_test.1-x~"), "simple_test.1-x~");
        assert_eq!(encode_path("tests::my_test"), "tests%3A%3Amy_test");
        assert_eq!(encode_path("a/b c%d"), "a%2Fb%20c%25d");
        assert_eq!(encode_path("café"), "caf%C3%A9");
        assert_eq!(encode_path(""), "");
    }

    #[test]
    fn run_status_spells_the_values_the_daemon_deserializes() {
        assert_eq!(
            RunStatus::Ok.to_json(),
            serde_json::json!({ "status": "ok" })
        );
        assert_eq!(
            RunStatus::Failed.to_json(),
            serde_json::json!({ "status": "failed" })
        );
    }
}
