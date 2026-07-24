use thiserror::Error;

#[derive(Debug, Error)]
pub enum CliError {
    /// Invalid argument or flag value — user error
    #[error("{0}")]
    Usage(String),

    /// Relay returned a non-2xx response
    #[error("relay error {status}: {body}")]
    Relay { status: u16, body: String },

    /// Network-level failure (connect, timeout, DNS)
    #[error("network error: {}", fmt_reqwest_error(.0))]
    Network(#[from] reqwest::Error),

    /// Auth missing or rejected (401/403)
    #[error("auth error: {0}")]
    Auth(String),

    /// Nostr key error (NIP-98 signing in `buzz auth`)
    #[error("key error: {0}")]
    Key(String),

    /// Relay accepted the event but reported it as superseded by a newer
    /// head — used by `buzz mem` set/rm to surface NIP-33 LWW conflicts.
    #[error("conflict: {0}")]
    Conflict(String),

    /// Requested resource was absent or tombstoned (e.g. `buzz mem get`
    /// for a slug with no head).
    #[error("{0}")]
    NotFound(String),

    /// A non-idempotent command's outcome is unknown: the request may have
    /// reached the relay, but the response was lost. Never auto-retried and
    /// never labeled retryable — the relay executes these commands before any
    /// dedup, so a blind re-run can duplicate the mutation.
    #[error("delivery unknown: {0}")]
    DeliveryUnknown(String),

    /// Catch-all for unexpected failures
    #[error("{0}")]
    Other(String),
}

/// Walk the full `std::error::Error::source()` chain on a `reqwest::Error`
/// and render it as a colon-separated string, e.g.
/// `error sending request: dns error: failed to lookup address information: ...`
fn fmt_reqwest_error(e: &reqwest::Error) -> String {
    let mut msg = e.to_string();
    let mut source: &dyn std::error::Error = e;
    while let Some(cause) = source.source() {
        let cause_str = cause.to_string();
        if !msg.contains(&cause_str) {
            msg.push_str(": ");
            msg.push_str(&cause_str);
        }
        source = cause;
    }
    msg
}

/// Returns `true` when the error is transient and a retry may succeed.
///
/// Transport-level network errors (connect failure, timeout, mid-request,
/// mid-body transfer, or body decode failure) and relay overload responses
/// (429 / 502 / 503 / 504) are retryable.  `DeliveryUnknown` is never
/// retryable: the operation may already have executed.  All other errors
/// indicate a permanent failure: auth, bad input, builder errors, or logic
/// errors.
pub fn is_retryable_error(e: &CliError) -> bool {
    match e {
        CliError::Network(ref net_err) => {
            net_err.is_connect()
                || net_err.is_timeout()
                || net_err.is_request()
                || net_err.is_body()
                || net_err.is_decode()
        }
        CliError::Relay { status, .. } => matches!(status, 429 | 502 | 503 | 504),
        CliError::DeliveryUnknown(_) => false,
        _ => false,
    }
}

/// Map CliError to process exit code.
/// 0=success (not an error), 1=user/not-found, 2=network/relay, 3=auth,
/// 4=other, 5=write conflict (NIP-33 dominated head).
pub fn exit_code(e: &CliError) -> i32 {
    match e {
        CliError::Usage(_) => 1,
        CliError::Relay { status, .. } => {
            if *status == 401 || *status == 403 {
                3
            } else {
                2
            }
        }
        CliError::Network(_) => 2,
        CliError::Auth(_) => 3,
        CliError::Key(_) => 3,
        CliError::Conflict(_) => 5,
        CliError::NotFound(_) => 1,
        CliError::DeliveryUnknown(_) => 2,
        CliError::Other(_) => 4,
    }
}

/// Serialize error to JSON and write to stderr.
/// Format: {"error": "<category>", "message": "<human-readable detail>", "retryable": <bool>}
pub fn print_error(e: &CliError) {
    let category = match e {
        CliError::Usage(_) => "user_error",
        CliError::Relay { status, .. } => {
            if *status == 401 || *status == 403 {
                "auth_error"
            } else {
                "relay_error"
            }
        }
        CliError::Network(_) => "network_error",
        CliError::Auth(_) => "auth_error",
        CliError::Key(_) => "key_error",
        CliError::Conflict(_) => "conflict",
        CliError::NotFound(_) => "not_found",
        CliError::DeliveryUnknown(_) => "delivery_unknown",
        CliError::Other(_) => "error",
    };
    let obj = serde_json::json!({
        "error": category,
        "message": e.to_string(),
        "retryable": is_retryable_error(e),
    });
    eprintln!("{}", obj);
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---- is_retryable_error ----

    #[test]
    fn network_builder_errors_are_not_retryable() {
        // A bad URL produces a builder-level reqwest::Error (is_builder() == true).
        // Builder errors are not transport failures — not retryable.
        // Transport errors (is_connect/timeout/request) require live I/O to construct;
        // the predicate here mirrors with_retry's condition exactly.
        let e = reqwest::Client::new().get("not-a-url").build().unwrap_err();
        assert!(e.is_builder(), "expected a builder error from bad URL");
        assert!(!is_retryable_error(&CliError::Network(e)));
    }

    #[test]
    fn relay_429_502_503_504_are_retryable() {
        for status in [429u16, 502, 503, 504] {
            assert!(
                is_retryable_error(&CliError::Relay {
                    status,
                    body: String::new()
                }),
                "status {status} should be retryable"
            );
        }
    }

    #[test]
    fn relay_400_401_403_404_422_are_not_retryable() {
        for status in [400u16, 401, 403, 404, 422] {
            assert!(
                !is_retryable_error(&CliError::Relay {
                    status,
                    body: String::new()
                }),
                "status {status} should not be retryable"
            );
        }
    }

    #[test]
    fn other_errors_are_not_retryable() {
        assert!(!is_retryable_error(&CliError::Usage("bad flag".into())));
        assert!(!is_retryable_error(&CliError::Auth("missing key".into())));
        assert!(!is_retryable_error(&CliError::Key("bad key".into())));
        assert!(!is_retryable_error(&CliError::Conflict(
            "superseded".into()
        )));
        assert!(!is_retryable_error(&CliError::NotFound("gone".into())));
        assert!(!is_retryable_error(&CliError::Other("unexpected".into())));
    }

    // ---- print_error "retryable" field ----

    #[test]
    fn json_error_includes_retryable_field_for_network() {
        // Builder errors (bad URL) are not transport-level — retryable: false.
        // This test verifies the JSON shape and that the field is present.
        let e = reqwest::Client::new().get("not-a-url").build().unwrap_err();
        let err = CliError::Network(e);
        let v = serde_json::json!({
            "error": "network_error",
            "message": err.to_string(),
            "retryable": is_retryable_error(&err),
        });
        assert_eq!(v["retryable"].as_bool(), Some(false));
        assert_eq!(v["error"].as_str(), Some("network_error"));
    }

    #[test]
    fn json_error_retryable_false_for_usage() {
        let err = CliError::Usage("bad flag".into());
        let v = serde_json::json!({
            "error": "user_error",
            "message": err.to_string(),
            "retryable": is_retryable_error(&err),
        });
        assert_eq!(v["retryable"].as_bool(), Some(false));
    }

    // ---- Display source-chain ----

    #[test]
    fn network_display_includes_detail_beyond_prefix() {
        let e = reqwest::Client::new().get("not-a-url").build().unwrap_err();
        let display = CliError::Network(e).to_string();
        assert!(
            display.starts_with("network error:"),
            "display should start with 'network error:': {display}"
        );
        assert!(
            display.len() > "network error: ".len(),
            "display should contain error detail: {display}"
        );
    }
}
