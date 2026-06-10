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
    #[error("network error: {0}")]
    Network(#[from] reqwest::Error),

    /// Auth missing or rejected (401/403)
    #[error("auth error: {0}")]
    Auth(String),

    /// Nostr key error (NIP-98 signing in `sprout auth`)
    #[error("key error: {0}")]
    Key(String),

    /// Relay accepted the event but reported it as superseded by a newer
    /// head — used by `sprout mem` set/rm to surface NIP-33 LWW conflicts.
    #[error("conflict: {0}")]
    Conflict(String),

    /// Requested resource was absent or tombstoned (e.g. `sprout mem get`
    /// for a slug with no head).
    #[error("{0}")]
    NotFound(String),

    /// Catch-all for unexpected failures
    #[error("{0}")]
    Other(String),
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
        CliError::Other(_) => 4,
    }
}

/// Serialize error to JSON and write to stderr.
/// Format: {"error": "<category>", "message": "<human-readable detail>"}
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
        CliError::Other(_) => "error",
    };
    let obj = serde_json::json!({
        "error": category,
        "message": e.to_string(),
    });
    eprintln!("{}", obj);
}
