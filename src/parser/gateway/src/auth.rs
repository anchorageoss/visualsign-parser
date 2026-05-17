//! Optional shared-bearer-token HTTP gate in front of every route except
//! `/health`.
//!
//! Disabled by default — when neither `GATEWAY_AUTH_BEARER_TOKEN` nor
//! `GATEWAY_AUTH_BEARER_FILE` is set the gateway behaves exactly as before.
//! When one of them is set, callers must send `Authorization: Bearer <token>`
//! or receive `401 Unauthorized` with `WWW-Authenticate: Bearer
//! realm="x402-gateway"`. The two env vars are mutually exclusive (both set
//! is a boot-time error).
//!
//! The token is a shared secret. This is a weak gate — its job is to keep
//! random crawlers off the endpoint while AI-agent callers (which can set
//! arbitrary headers but can't easily mint per-caller identity tokens) can
//! reach the x402 settlement layer below. Per-caller identity belongs in a
//! separate auth-proxy workstream.
//!
//! `/health` is excluded so operators can probe liveness without sharing the
//! token, and so Cloud Run-style HTTP health probes (if added later) don't
//! need to carry it either.

use axum::{
    body::Body,
    extract::{Request, State},
    http::{
        HeaderValue, StatusCode,
        header::{AUTHORIZATION, WWW_AUTHENTICATE},
    },
    middleware::Next,
    response::Response,
};
use std::sync::Arc;
use subtle::ConstantTimeEq;

#[derive(Debug, thiserror::Error)]
pub enum AuthError {
    #[error("both GATEWAY_AUTH_BEARER_TOKEN and GATEWAY_AUTH_BEARER_FILE are set; choose one")]
    BothSet,
    #[error("failed to read GATEWAY_AUTH_BEARER_FILE={path}: {message}")]
    ReadFile { path: String, message: String },
    #[error("bearer token from {origin} is empty after trim")]
    Empty { origin: &'static str },
}

/// In-memory bearer token. The bytes are stored after trimming surrounding
/// whitespace from whichever source provided them.
#[derive(Clone)]
pub struct BearerToken {
    bytes: Arc<Vec<u8>>,
}

impl BearerToken {
    /// Real-process entrypoint. Reads `GATEWAY_AUTH_BEARER_TOKEN` (inline)
    /// or `GATEWAY_AUTH_BEARER_FILE` (path to a file containing the token).
    /// Returns `Ok(None)` when neither is set.
    pub fn from_env() -> Result<Option<Self>, AuthError> {
        let inline = std::env::var("GATEWAY_AUTH_BEARER_TOKEN").ok();
        let file_path = std::env::var("GATEWAY_AUTH_BEARER_FILE").ok();
        match (inline, file_path) {
            (None, None) => Ok(None),
            (Some(_), Some(_)) => Err(AuthError::BothSet),
            (Some(raw), None) => Self::from_sources(Some(&raw), None),
            (None, Some(path)) => {
                let raw = std::fs::read_to_string(&path).map_err(|e| AuthError::ReadFile {
                    path: path.clone(),
                    message: e.to_string(),
                })?;
                Self::from_sources(None, Some(&raw))
            }
        }
    }

    /// Testable core. Takes the resolved contents (not paths). At most one
    /// of `inline` / `file_contents` may be `Some`.
    pub fn from_sources(
        inline: Option<&str>,
        file_contents: Option<&str>,
    ) -> Result<Option<Self>, AuthError> {
        match (inline, file_contents) {
            (None, None) => Ok(None),
            (Some(_), Some(_)) => Err(AuthError::BothSet),
            (Some(raw), None) => Self::trim_and_store(raw, "GATEWAY_AUTH_BEARER_TOKEN"),
            (None, Some(raw)) => Self::trim_and_store(raw, "GATEWAY_AUTH_BEARER_FILE"),
        }
    }

    fn trim_and_store(raw: &str, origin: &'static str) -> Result<Option<Self>, AuthError> {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            return Err(AuthError::Empty { origin });
        }
        Ok(Some(Self {
            bytes: Arc::new(trimmed.as_bytes().to_vec()),
        }))
    }

    /// Constant-time match when lengths align. Length mismatch bails early —
    /// the token length is fixed at deploy time, so leaking it costs nothing.
    pub fn matches(&self, candidate: &[u8]) -> bool {
        if candidate.len() != self.bytes.len() {
            return false;
        }
        bool::from(candidate.ct_eq(&self.bytes))
    }

    /// Length of the stored token in bytes. Surfaced for an informative
    /// startup log line; the token contents are never logged.
    pub fn byte_len(&self) -> usize {
        self.bytes.len()
    }
}

/// Decision returned by [`evaluate_request`]. Pulled out of the middleware
/// fn so the routing logic is testable without spinning up an axum Router.
#[derive(Debug, PartialEq, Eq)]
pub enum AuthOutcome {
    Allow,
    Deny,
}

/// Pure auth-routing decision: should this request reach the handler?
/// The middleware fn below is a thin shell around this.
pub fn evaluate_request(
    path: &str,
    authorization_header: Option<&str>,
    expected: &BearerToken,
) -> AuthOutcome {
    if path == "/health" {
        return AuthOutcome::Allow;
    }
    let provided = match authorization_header.and_then(|v| v.strip_prefix("Bearer ")) {
        Some(t) => t,
        None => return AuthOutcome::Deny,
    };
    if expected.matches(provided.as_bytes()) {
        AuthOutcome::Allow
    } else {
        AuthOutcome::Deny
    }
}

/// Axum middleware. Attach via:
///
/// ```ignore
/// app = app.layer(axum::middleware::from_fn_with_state(
///     bearer_token,
///     parser_gateway::auth::require_bearer_token,
/// ));
/// ```
pub async fn require_bearer_token(
    State(token): State<BearerToken>,
    req: Request,
    next: Next,
) -> Response {
    let path = req.uri().path();
    let header_value = req
        .headers()
        .get(AUTHORIZATION)
        .and_then(|h| h.to_str().ok());
    match evaluate_request(path, header_value, &token) {
        AuthOutcome::Allow => next.run(req).await,
        AuthOutcome::Deny => unauthorized(),
    }
}

fn unauthorized() -> Response {
    let mut resp = Response::new(Body::empty());
    *resp.status_mut() = StatusCode::UNAUTHORIZED;
    resp.headers_mut().insert(
        WWW_AUTHENTICATE,
        HeaderValue::from_static(r#"Bearer realm="x402-gateway""#),
    );
    resp
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    fn token(s: &str) -> BearerToken {
        BearerToken::from_sources(Some(s), None).unwrap().unwrap()
    }

    #[test]
    fn from_sources_absent_returns_none() {
        let res = BearerToken::from_sources(None, None).unwrap();
        assert!(res.is_none());
    }

    #[test]
    fn from_sources_inline_round_trip() {
        let t = token("panic-at-the-gateway");
        assert!(t.matches(b"panic-at-the-gateway"));
    }

    #[test]
    fn from_sources_file_round_trip() {
        let t = BearerToken::from_sources(None, Some("panic-at-the-gateway\n"))
            .unwrap()
            .unwrap();
        assert!(t.matches(b"panic-at-the-gateway"));
    }

    #[test]
    fn from_sources_both_set_errors() {
        let res = BearerToken::from_sources(Some("a"), Some("b"));
        assert!(matches!(res, Err(AuthError::BothSet)));
    }

    #[test]
    fn from_sources_empty_after_trim_errors() {
        let res = BearerToken::from_sources(Some("   \t\n"), None);
        assert!(matches!(res, Err(AuthError::Empty { .. })));
    }

    #[test]
    fn from_sources_trims_surrounding_whitespace() {
        let t = BearerToken::from_sources(Some("  hello\n"), None)
            .unwrap()
            .unwrap();
        assert!(t.matches(b"hello"));
        assert!(!t.matches(b"  hello\n"));
        assert!(!t.matches(b"  hello"));
    }

    #[test]
    fn matches_length_mismatch_returns_false() {
        let t = token("abc");
        assert!(!t.matches(b"ab"));
        assert!(!t.matches(b"abcd"));
        assert!(t.matches(b"abc"));
    }

    #[test]
    fn evaluate_request_health_bypasses_auth() {
        let t = token("any");
        assert_eq!(evaluate_request("/health", None, &t), AuthOutcome::Allow);
        assert_eq!(
            evaluate_request("/health", Some("Bearer wrong"), &t),
            AuthOutcome::Allow,
        );
    }

    #[test]
    fn evaluate_request_no_header_denies() {
        let t = token("panic-at-the-gateway");
        assert_eq!(
            evaluate_request("/visualsign/api/v2/parse", None, &t),
            AuthOutcome::Deny,
        );
    }

    #[test]
    fn evaluate_request_wrong_scheme_denies() {
        let t = token("panic-at-the-gateway");
        assert_eq!(
            evaluate_request(
                "/visualsign/api/v2/parse",
                Some("Basic YWdlbnQ6cGFuaWMtYXQtdGhlLWdhdGV3YXk="),
                &t,
            ),
            AuthOutcome::Deny,
        );
    }

    #[test]
    fn evaluate_request_wrong_token_denies() {
        let t = token("panic-at-the-gateway");
        assert_eq!(
            evaluate_request(
                "/visualsign/api/v2/parse",
                Some("Bearer wrong-password"),
                &t,
            ),
            AuthOutcome::Deny,
        );
    }

    #[test]
    fn evaluate_request_correct_token_allows() {
        let t = token("panic-at-the-gateway");
        assert_eq!(
            evaluate_request(
                "/visualsign/api/v2/parse",
                Some("Bearer panic-at-the-gateway"),
                &t,
            ),
            AuthOutcome::Allow,
        );
    }

    #[test]
    fn unauthorized_response_carries_www_authenticate_bearer_realm() {
        let r = unauthorized();
        assert_eq!(r.status(), StatusCode::UNAUTHORIZED);
        let www = r
            .headers()
            .get(WWW_AUTHENTICATE)
            .expect("WWW-Authenticate header is required")
            .to_str()
            .unwrap();
        assert!(www.starts_with("Bearer "), "got: {www}");
        assert!(www.contains(r#"realm="x402-gateway""#));
    }
}
