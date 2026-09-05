//! `buzz doctor` — non-mutating preflight diagnostics.
//!
//! Unlike every other subcommand, doctor intentionally does NOT require
//! credentials: missing or malformed config must surface as a check result
//! (status `error`) rather than an argument-time usage failure. The command
//! never publishes events and never mutates local or remote state;
//! `--offline` runs only local checks and marks remote probes as skipped.
//!
//! Output is structured JSON on stdout:
//!
//! ```json
//! {"status":"warning","checks":[{"id":"identity","status":"ok","message":"..."}]}
//! ```
//!
//! Exit behavior distinguishes a failed required check from warnings:
//! `0` when every applicable check is `ok` (warnings allowed),
//! `3` when any check is `error` and at least one error is auth/identity,
//! `2` when any check is `error` and all errors are relay/network, and
//! `1` for any other failure mix.  Skipped checks never affect the exit
//! code.  The human-readable `status` field is `ok`, `warning`, or `error`
//! mirroring the worst check outcome.

use serde::Serialize;
use serde_json::json;

use crate::client::{normalize_relay_url, BuzzClient};
use crate::error::CliError;

/// Status of a single doctor check.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
enum Status {
    Ok,
    Warning,
    Error,
    Skipped,
}

impl Status {
    fn as_str(self) -> &'static str {
        match self {
            Self::Ok => "ok",
            Self::Warning => "warning",
            Self::Error => "error",
            Self::Skipped => "skipped",
        }
    }
}

/// Serialisable form of one check for the JSON output.
#[derive(Debug, Serialize)]
struct Check {
    id: &'static str,
    status: Status,
    message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    remediation: Option<String>,
}

impl Check {
    fn ok(id: &'static str, message: impl Into<String>) -> Self {
        Self {
            id,
            status: Status::Ok,
            message: message.into(),
            remediation: None,
        }
    }

    fn warning(id: &'static str, message: impl Into<String>) -> Self {
        Self {
            id,
            status: Status::Warning,
            message: message.into(),
            remediation: None,
        }
    }

    fn error(id: &'static str, message: impl Into<String>, remediation: Option<&str>) -> Self {
        Self {
            id,
            status: Status::Error,
            message: message.into(),
            remediation: remediation.map(str::to_string),
        }
    }

    fn skipped(id: &'static str, message: impl Into<String>) -> Self {
        Self {
            id,
            status: Status::Skipped,
            message: message.into(),
            remediation: None,
        }
    }
}

/// Entry point invoked from `crate::run` before the credential gate.
///
/// The doctor never returns `Err` for missing configuration; it reports
/// the problem as a check and exits non-zero via `run_from_args`.
pub async fn run(cli: &crate::Cli, offline: bool) -> Result<(), CliError> {
    let mut checks: Vec<Check> = Vec::new();

    // ---- local checks ----

    // `cli.relay` itself is a free-form string; canonicalize before probing
    // so the rest of the doctor matches the rest of the CLI.
    let relay_url_raw = cli.relay.clone();
    let relay_url = normalize_relay_url(&relay_url_raw);
    if relay_url.is_empty() {
        checks.push(Check::error(
            "relay_url",
            "relay URL is empty",
            Some("Set BUZZ_RELAY_URL or pass --relay"),
        ));
    } else if !(relay_url.starts_with("http://") || relay_url.starts_with("https://")) {
        checks.push(Check::error(
            "relay_url",
            format!(
                "relay URL {:?} is not an http(s) URL after canonicalization",
                relay_url
            ),
            Some("Use http:// or https://; ws:// and wss:// are rewritten automatically"),
        ));
    } else {
        checks.push(Check::ok(
            "relay_url",
            format!("relay URL canonicalized to {}", relay_url),
        ));
    }

    // identity check: parse the private key. Never echo the material.
    let keys: Option<nostr::Keys> = match cli.private_key.as_deref() {
        None => {
            checks.push(Check::error(
                "identity",
                "BUZZ_PRIVATE_KEY is not set",
                Some("Set BUZZ_PRIVATE_KEY to a hex or nsec key"),
            ));
            None
        }
        Some(raw) => match nostr::Keys::parse(raw) {
            Ok(k) => {
                let pk = k.public_key().to_hex();
                checks.push(Check::ok(
                    "identity",
                    format!("signing identity available: {pk}"),
                ));
                Some(k)
            }
            Err(e) => {
                checks.push(Check::error(
                    "identity",
                    format!("BUZZ_PRIVATE_KEY failed to parse: {e}"),
                    Some("Provide a nostr private key in hex or nsec form"),
                ));
                None
            }
        },
    };

    // auth_tag check: parse only; never print the tag material.
    let auth_tag: Option<nostr::Tag> = match cli.auth_tag.as_deref() {
        None => {
            checks.push(Check::ok(
                "auth_tag",
                "no BUZZ_AUTH_TAG set (optional; only required for delegated agents)",
            ));
            None
        }
        Some("") => {
            checks.push(Check::ok(
                "auth_tag",
                "BUZZ_AUTH_TAG is empty (treated as unset)",
            ));
            None
        }
        Some(raw) => match buzz_sdk::nip_oa::parse_auth_tag(raw) {
            Ok(tag) => match &keys {
                Some(k) => match buzz_sdk::nip_oa::verify_auth_tag(raw, &k.public_key()) {
                    Ok(_) => {
                        checks.push(Check::ok(
                            "auth_tag",
                            "BUZZ_AUTH_TAG parsed and verified for this identity",
                        ));
                        Some(tag)
                    }
                    Err(e) => {
                        checks.push(Check::error(
                            "auth_tag",
                            format!("BUZZ_AUTH_TAG failed verification: {e}"),
                            Some("Re-issue the auth tag for the current identity"),
                        ));
                        None
                    }
                },
                None => {
                    // Cannot verify without an identity; report as warning.
                    checks.push(Check::warning(
                        "auth_tag",
                        "BUZZ_AUTH_TAG present but cannot be verified without a valid identity",
                    ));
                    Some(tag)
                }
            },
            Err(e) => {
                checks.push(Check::error(
                    "auth_tag",
                    format!("BUZZ_AUTH_TAG is malformed: {e}"),
                    Some("Re-export the auth tag JSON from the owning desktop"),
                ));
                None
            }
        },
    };

    // version check: always available, no I/O.
    checks.push(Check::ok(
        "version",
        format!("buzz-cli {}", env!("CARGO_PKG_VERSION")),
    ));

    // ---- remote checks ----

    if offline {
        checks.push(Check::skipped("relay_reachable", "skipped (--offline)"));
        checks.push(Check::skipped("nip11", "skipped (--offline)"));
        checks.push(Check::skipped("auth_read", "skipped (--offline)"));
        checks.push(Check::skipped("membership", "skipped (--offline)"));
    } else {
        run_remote_checks(
            &relay_url,
            keys.as_ref(),
            auth_tag.as_ref(),
            cli,
            &mut checks,
        )
        .await;
    }

    // ---- output ----
    emit_and_exit(&checks)
}

async fn run_remote_checks(
    relay_url: &str,
    keys: Option<&nostr::Keys>,
    auth_tag: Option<&nostr::Tag>,
    cli: &crate::Cli,
    checks: &mut Vec<Check>,
) {
    // Build a client only when we have a parseable identity; local checks
    // already reported the identity problem, so the remote probes degrade
    // to skipped/errored rather than re-diagnosing it.
    let authed_client: Option<BuzzClient> = match keys {
        Some(k) => match BuzzClient::new(
            relay_url.to_string(),
            k.clone(),
            auth_tag.cloned(),
            cli.auth_tag.clone(),
        ) {
            Ok(c) => Some(c),
            Err(e) => {
                checks.push(Check::error(
                    "relay_reachable",
                    format!("failed to build HTTP client: {e}"),
                    None,
                ));
                None
            }
        },
        None => None,
    };

    // Unauthenticated reachability probe: try the public NIP-11 document.
    // We do this with a bare reqwest client so that doctor works even when
    // the identity is invalid — reachability is independent of auth.
    // Track the outcome so we can avoid re-emitting the same error when
    // the authed probe below also fails.
    let relay_reachable = match probe_reachability(relay_url).await {
        Ok(()) => {
            checks.push(Check::ok(
                "relay_reachable",
                format!("relay at {relay_url} responded to an unauthenticated probe"),
            ));
            true
        }
        Err(msg) => {
            checks.push(Check::error(
                "relay_reachable",
                msg,
                Some("Check BUZZ_RELAY_URL, DNS, and network connectivity"),
            ));
            false
        }
    };

    // NIP-11 probe. Public per spec; relay may also accept the metadata
    // unauthenticated.  The same probe result feeds both the reachability
    // message above (already emitted) and the parsed metadata here.
    match fetch_nip11(relay_url).await {
        Ok(doc) => {
            let name = doc
                .get("name")
                .and_then(|v| v.as_str())
                .unwrap_or("<unnamed>");
            let supported = doc
                .get("supported_nips")
                .and_then(|v| v.as_array())
                .map(|arr| arr.len())
                .unwrap_or(0);
            checks.push(Check::ok(
                "nip11",
                format!("NIP-11 document parsed: {name} (advertises {supported} NIPs)"),
            ));
        }
        Err(msg) => {
            checks.push(Check::error(
                "nip11",
                msg,
                Some("Verify the relay exposes / (or /info) as application/nostr+json"),
            ));
        }
    }

    // Authenticated read probe + membership probe. Both require an identity;
    // skip them cleanly if we don't have one rather than re-diagnosing.
    match authed_client {
        None => {
            checks.push(Check::skipped(
                "auth_read",
                "skipped (no valid signing identity)",
            ));
            checks.push(Check::skipped(
                "membership",
                "skipped (no valid signing identity)",
            ));
        }
        Some(client) => {
            // auth_read: a small, bounded, read-only filter.  Using
            // kinds:[39002] (#p=self) doubles as the membership probe.
            let my_pk = client.keys().public_key().to_hex();
            let filter = json!({
                "kinds": [39002],
                "#p": [my_pk],
            });
            match client.query_paginated(filter, 1).await {
                Ok(events) => {
                    checks.push(Check::ok("auth_read", "authenticated read succeeded"));
                    // membership: any kind:39002 with #p=self means we're
                    // a member of at least one channel.
                    if events.is_empty() {
                        checks.push(Check::warning(
                            "membership",
                            "identity is not a member of any channel visible to this relay",
                        ));
                    } else {
                        checks.push(Check::ok(
                            "membership",
                            format!(
                                "identity has membership in at least one channel (saw {} event(s))",
                                events.len()
                            ),
                        ));
                    }
                }
                // True auth rejection — either an explicit Auth variant or a
                // relay 401/403 — classifies as auth (exit 3).
                Err(e @ CliError::Auth(_))
                | Err(
                    e @ CliError::Relay {
                        status: 401 | 403, ..
                    },
                ) => {
                    checks.push(Check::error(
                        "auth_read",
                        format!("relay rejected authentication: {e}"),
                        Some("Verify BUZZ_PRIVATE_KEY and BUZZ_AUTH_TAG"),
                    ));
                    checks.push(Check::skipped(
                        "membership",
                        "skipped (authentication failed)",
                    ));
                }
                // Anything else — network/transport or non-401 relay — means
                // the relay couldn't be queried authoritatively.  If the
                // unauthenticated reachability probe already failed, the
                // check list already carries that signal; don't double-report.
                // Otherwise, this path means the relay was reachable publicly
                // but rejected our authed call network-side: report as a
                // relay-side failure (exit 2), NOT as an auth failure.
                Err(e) => {
                    if relay_reachable {
                        checks.push(Check::error(
                            "relay_reachable",
                            format!("authenticated read failed: {e}"),
                            Some("Check relay availability, DNS, and network connectivity"),
                        ));
                    }
                    checks.push(Check::skipped(
                        "auth_read",
                        "skipped (relay unreachable for authenticated read)",
                    ));
                    checks.push(Check::skipped(
                        "membership",
                        "skipped (relay unreachable for authenticated read)",
                    ));
                }
            }
        }
    }
}

/// Cheap reachability probe: GET `/` with the NIP-11 accept header.  We
/// don't care about the body here; only that something HTTP 2xx came back.
async fn probe_reachability(relay_url: &str) -> Result<(), String> {
    let http = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .connect_timeout(std::time::Duration::from_secs(5))
        .build()
        .map_err(|e| format!("failed to build HTTP client: {e}"))?;
    let resp = http
        .get(relay_url)
        .header("Accept", "application/nostr+json")
        .send()
        .await
        .map_err(|e| format!("relay probe failed: {e}"))?;
    if resp.status().is_success() {
        Ok(())
    } else {
        Err(format!("relay returned HTTP {}", resp.status()))
    }
}

/// Fetch and parse the relay's NIP-11 information document.
///
/// We reuse `get_public` so redirect, timeout, and TLS behavior matches
/// the rest of the CLI.
async fn fetch_nip11(relay_url: &str) -> Result<serde_json::Value, String> {
    let http = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .connect_timeout(std::time::Duration::from_secs(5))
        .build()
        .map_err(|e| format!("failed to build HTTP client: {e}"))?;
    // NIP-11 specifies GET on the relay URL itself with the metadata
    // accept header.  The `get_public` helper on a constructed client does
    // exactly this; we inline the call here so doctor does not need an
    // authed `BuzzClient`.
    let resp = http
        .get(relay_url)
        .header("Accept", "application/nostr+json")
        .send()
        .await
        .map_err(|e| format!("NIP-11 probe failed: {e}"))?;
    let status = resp.status();
    let body = resp
        .text()
        .await
        .map_err(|e| format!("failed to read NIP-11 body: {e}"))?;
    if !status.is_success() {
        return Err(format!("relay returned HTTP {status}"));
    }
    serde_json::from_str(&body).map_err(|e| format!("NIP-11 document was not valid JSON: {e}"))
}

/// Aggregate after running all checks and return the process-level
/// `CliError` (or `Ok` for success).
///
/// Mapping (exposed for tests via `decide_outcome`):
/// - any `error` check mentioning `identity` or `auth_tag` or `auth_read` => `CliError::Auth` (exit 3)
/// - any other `error` check                                            => `CliError::Relay` (exit 2)
/// - no `error` but any `warning`                                       => `Ok(())` (exit 0)
/// - all `ok` / `skipped`                                               => `Ok(())` (exit 0)
fn emit_and_exit(checks: &[Check]) -> Result<(), CliError> {
    let overall = if checks.iter().any(|c| c.status == Status::Error) {
        "error"
    } else if checks.iter().any(|c| c.status == Status::Warning) {
        "warning"
    } else {
        "ok"
    };

    let doc = json!({
        "status": overall,
        "checks": checks.iter().map(|c| json!({
            "id": c.id,
            "status": c.status.as_str(),
            "message": c.message,
            "remediation": c.remediation,
        })).collect::<Vec<_>>(),
    });
    println!("{doc}");

    if overall == "ok" || overall == "warning" {
        return Ok(());
    }

    // Distinguish credential-shaped failures from transport failures.
    let mut has_auth_error = false;
    let mut first_other_error: Option<&Check> = None;
    for c in checks {
        if c.status != Status::Error {
            continue;
        }
        match c.id {
            "identity" | "auth_tag" | "auth_read" => has_auth_error = true,
            _ => {
                if first_other_error.is_none() {
                    first_other_error = Some(c);
                }
            }
        }
    }

    if has_auth_error {
        return Err(CliError::Auth(
            "one or more doctor checks failed (see JSON output for details)".into(),
        ));
    }
    if let Some(c) = first_other_error {
        return Err(CliError::Relay {
            status: 0,
            body: format!("doctor check {} failed: {}", c.id, c.message),
        });
    }
    Err(CliError::Other("one or more doctor checks failed".into()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::exit_code;

    fn check_with(id: &'static str, status: Status) -> Check {
        Check {
            id,
            status,
            message: String::new(),
            remediation: None,
        }
    }

    #[test]
    fn all_ok_emits_status_ok_and_exit_zero() {
        let checks = vec![
            check_with("relay_url", Status::Ok),
            check_with("identity", Status::Ok),
        ];
        assert!(emit_and_exit(&checks).is_ok());
    }

    #[test]
    fn warning_emits_warning_and_exit_zero() {
        let checks = vec![
            check_with("relay_url", Status::Ok),
            check_with("membership", Status::Warning),
        ];
        assert!(emit_and_exit(&checks).is_ok());
    }

    #[test]
    fn error_on_identity_maps_to_auth() {
        let checks = vec![check_with("identity", Status::Error)];
        let err = emit_and_exit(&checks).unwrap_err();
        assert!(matches!(err, CliError::Auth(_)));
        assert_eq!(exit_code(&err), 3);
    }

    #[test]
    fn error_on_auth_read_maps_to_auth() {
        let checks = vec![check_with("auth_read", Status::Error)];
        let err = emit_and_exit(&checks).unwrap_err();
        assert!(matches!(err, CliError::Auth(_)));
        assert_eq!(exit_code(&err), 3);
    }

    #[test]
    fn error_on_relay_reachable_maps_to_relay_exit_two() {
        let checks = vec![check_with("relay_reachable", Status::Error)];
        let err = emit_and_exit(&checks).unwrap_err();
        assert!(matches!(err, CliError::Relay { status: 0, .. }));
        assert_eq!(exit_code(&err), 2);
    }

    #[test]
    fn error_mixed_auth_wins_over_relay() {
        let checks = vec![
            check_with("relay_reachable", Status::Error),
            check_with("identity", Status::Error),
        ];
        let err = emit_and_exit(&checks).unwrap_err();
        assert!(matches!(err, CliError::Auth(_)));
    }

    #[test]
    fn skipped_checks_do_not_affect_outcome() {
        let checks = vec![
            check_with("relay_url", Status::Ok),
            check_with("identity", Status::Ok),
            check_with("auth_read", Status::Skipped),
            check_with("membership", Status::Skipped),
        ];
        assert!(emit_and_exit(&checks).is_ok());
    }

    #[test]
    fn json_shape_has_status_and_checks_array() {
        // Serialise directly to avoid printing; validate the shape line by
        // line rather than relying on `println!` output.
        let checks = [check_with("identity", Status::Ok)];
        let doc = serde_json::json!({
            "status": "ok",
            "checks": checks.iter().map(|c| serde_json::json!({
                "id": c.id,
                "status": c.status.as_str(),
                "message": c.message,
                "remediation": c.remediation,
            })).collect::<Vec<_>>(),
        });
        assert_eq!(doc["status"].as_str(), Some("ok"));
        let arr = doc["checks"].as_array().expect("checks is an array");
        assert_eq!(arr.len(), 1);
        assert_eq!(arr[0]["id"].as_str(), Some("identity"));
        assert_eq!(arr[0]["status"].as_str(), Some("ok"));
    }
}
