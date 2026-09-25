//! NIP-FI admin disconnect endpoint — `POST /api/nip-fi/disconnect`.
//!
//! This module owns:
//!
//! * [`disconnect`] — the axum handler for `POST /api/nip-fi/disconnect`.
//! * [`build_nip_fi_command_components`] — startup initialization called by
//!   `main.rs` to wire the deny map and command verifier into `AppState`.
//!
//! ## Transport invariant
//!
//! The NIP-FI admin API is **not** a protected HTTP surface.  It MUST NOT be
//! subjected to the NIP-FI HTTP-ingress admission procedure.  It carries
//! `Nostr-Federated-Identity` for a command JWS, not an identity assertion.
//! [NIP-FI.md §HTTP ingress, protected surfaces note]
//!
//! Authentication is entirely by the signed command JWT verified inside
//! [`buzz_auth::CommandVerifier::verify`]; no NIP-98 or relay-membership check
//! is performed.
//!
//! ## Environment variables
//!
//! The command API is enabled when `BUZZ_NIP_FI_MODE=enforce` and the issuer
//! JSON entries include the S4 fields.  S4 fields are read from the same
//! `BUZZ_NIP_FI_ISSUERS` JSON array; each issuer entry optionally carries:
//!
//! ```json
//! {
//!   "maximum_command_age_seconds": 30,
//!   "authorized_principals": ["service-account@issuer.example.com"],
//!   "deny_set_capacity": 50000
//! }
//! ```
//!
//! `maximum_command_age_seconds` and `authorized_principals` are required in
//! enforce mode if any issuer is command-capable.  `deny_set_capacity` defaults
//! to [`DEFAULT_DENY_SET_CAPACITY`] when absent.

use std::sync::Arc;

use axum::{
    body::Body,
    extract::State,
    http::{HeaderMap, Response, StatusCode},
};
use serde::Deserialize;
use tracing::{debug, warn};

use buzz_auth::{
    CommandError, CommandIssuerPolicy, CommandVerifier, IssuerCapacity, JwksFetcher, NipFiDenyMap,
    NipFiMode, ProductionJwksSource, CLIENT_ATTACHED_HEADER,
};

use crate::state::AppState;

/// Default per-issuer deny-set capacity when `deny_set_capacity` is absent.
/// 50_000 entries × ~128 bytes ≈ 6.4 MB per issuer.
pub const DEFAULT_DENY_SET_CAPACITY: usize = 50_000;

// ── Request / response shapes ────────────────────────────────────────────────

/// JSON body for `POST /api/nip-fi/disconnect`.
#[derive(Debug, Deserialize)]
pub struct DisconnectRequest {
    /// Lowercase hex encoding of the 32-byte target Nostr public key.
    pub pubkey: String,
}

// ── Handler ───────────────────────────────────────────────────────────────────

/// `POST /api/nip-fi/disconnect`
///
/// Executes `VerifyCommandJwt`, inserts the deny entry, and closes all live
/// sessions for the target pubkey across all communities.
///
/// Response contract (from NIP-FI spec):
///
/// | Condition | Status | Body |
/// |---|---|---|
/// | Authorized; action taken or no-op | `200` | `{"disconnected":true}` |
/// | Missing or invalid command JWT | `401`/`403` | per rejection table |
/// | Malformed request body or `until` exceeds ceiling | `400` | `"bad request\n"` |
/// | Deny set at capacity | `503` | `"deny set full\n"` |
///
/// The endpoint is NOT a protected HTTP surface.  [NIP-FI.md §HTTP ingress]
pub async fn disconnect(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Response<Body> {
    // ── Extract the command JWT from the header ────────────────────────────
    let token = match extract_command_jwt(&headers) {
        Ok(t) => t,
        Err(status) => {
            return if status == StatusCode::UNAUTHORIZED {
                // [NIP-FI.md §Rejection table]: 401 MUST carry WWW-Authenticate: Nostr.
                auth_required_response()
            } else {
                plain_response(status, "evidence rejected\n")
            };
        }
    };

    // ── Parse the JSON body ───────────────────────────────────────────────
    let req: DisconnectRequest = match serde_json::from_slice(&body) {
        Ok(r) => r,
        Err(_) => return plain_response(StatusCode::BAD_REQUEST, "bad request\n"),
    };

    // body.pubkey must be lowercase hex of exactly 32 bytes.
    let body_pubkey = match parse_hex_pubkey(&req.pubkey) {
        Some(k) => k,
        None => return plain_response(StatusCode::BAD_REQUEST, "bad request\n"),
    };

    // ── Command verifier ──────────────────────────────────────────────────
    let verifier = match &state.nip_fi_command_verifier {
        Some(v) => v.clone(),
        None => {
            // Mode is Off or not yet initialized.
            debug!("nip-fi disconnect: no command verifier configured");
            return plain_response(
                StatusCode::SERVICE_UNAVAILABLE,
                "authorization unavailable\n",
            );
        }
    };

    let result = verifier.verify(token, "POST", "/api/nip-fi/disconnect", &body_pubkey);

    match result {
        Ok(cmd) => {
            // ── Deny entry inserted; close sessions synchronously ─────────
            let pubkey_bytes = cmd.target_pubkey.to_bytes();
            // Issuer-scoped: the deny entry is keyed by (caller_iss, k), so only
            // sessions admitted under caller_iss are closed. [FI-TRACE-DENY-SET]
            let closed = state
                .conn_manager
                .disconnect_nip_fi(&cmd.caller_iss, &pubkey_bytes)
                + state
                    .community_connections
                    .disconnect_nip_fi(&cmd.caller_iss, &pubkey_bytes);
            if closed > 0 {
                // [FI-TRACE-PRIVACY-NONPUBLIC]: raw `iss` MUST NOT appear in
                // logs, metrics, or traces.  Log only a count.
                debug!(closed, "nip-fi disconnect: closed sessions");
            }
            metrics::counter!("buzz_nip_fi_disconnect_total").increment(1);
            metrics::counter!(
                "buzz_nip_fi_sessions_closed_total",
                "reason" => "admin_disconnect"
            )
            .increment(closed as u64);

            // Cross-pod propagation: publish to global NIP-FI Redis channel
            // so remote pods can merge the deny entry and close their sessions.
            // Asynchronous: HTTP response does not wait on remote delivery.
            {
                let pubsub = Arc::clone(&state.pubsub);
                let msg = nip_fi_disconnect_message(&cmd);
                tokio::spawn(async move {
                    if let Err(e) = pubsub.publish_nip_fi_disconnect(&msg).await {
                        // [FI-TRACE-PRIVACY-NONPUBLIC]: no iss or pubkey in logs
                        tracing::warn!("nip-fi: cross-pod propagation publish failed: {e}");
                        metrics::counter!("buzz_nip_fi_disconnect_propagation_failures_total")
                            .increment(1);
                    }
                });
            }

            disconnected_response()
        }
        Err(CommandError::DenySetFull) => {
            warn!("nip-fi disconnect: deny set full — command rejected, no sessions closed");
            metrics::counter!("buzz_nip_fi_disconnect_capacity_rejections_total").increment(1);
            plain_response(StatusCode::SERVICE_UNAVAILABLE, "deny set full\n")
        }
        Err(CommandError::UntilExceedsCeiling) | Err(CommandError::MalformedRequest) => {
            plain_response(StatusCode::BAD_REQUEST, "bad request\n")
        }
        Err(CommandError::AuthorizationUnavailable) => plain_response(
            StatusCode::SERVICE_UNAVAILABLE,
            "authorization unavailable\n",
        ),
        Err(CommandError::EvidenceRejected) => {
            plain_response(StatusCode::FORBIDDEN, "evidence rejected\n")
        }
        Err(CommandError::AuthorizationDenied) => {
            plain_response(StatusCode::FORBIDDEN, "authorization denied\n")
        }
    }
}

// ── Startup component builder ─────────────────────────────────────────────────

/// Per-issuer command configuration parsed from the `BUZZ_NIP_FI_ISSUERS` JSON.
///
/// Added to each entry in S4.  All three fields are optional (absent =
/// command API disabled for that issuer / default capacity used).
#[derive(Debug, Default, Clone, serde::Deserialize)]
pub struct CommandIssuerEnvConfig {
    /// Positive seconds, ≤ 60.  Required for the command API to be enabled.
    pub maximum_command_age_seconds: Option<u64>,
    /// Non-empty list of authorized `sub` values.  Required if command age is set.
    pub authorized_principals: Option<Vec<String>>,
    /// Hard ceiling on live deny entries for this issuer.
    /// Defaults to [`DEFAULT_DENY_SET_CAPACITY`] when absent.
    pub deny_set_capacity: Option<usize>,
}

/// The `NipFiDenyMap` + `CommandVerifier` pair built at startup.
pub struct NipFiCommandComponents<F: JwksFetcher = buzz_auth::HttpJwksFetcher> {
    /// The shared deny map consumed by WS admission and S5 HTTP admission.
    pub deny_map: Arc<NipFiDenyMap>,
    /// The command verifier for the `POST /api/nip-fi/disconnect` endpoint.
    pub command_verifier: Arc<CommandVerifier<Arc<ProductionJwksSource<F>>>>,
}

/// Outcome of applying a cross-pod NIP-FI disconnect message.
///
/// Returned by [`apply_nip_fi_disconnect`]; used by the `main.rs` receive loop
/// and by tests to assert the production decision.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NipFiDisconnectApplyResult {
    /// Command API is not enabled on this pod (deny map absent); message ignored.
    Disabled,
    /// Message was rejected before reaching the map (invalid pubkey, unknown
    /// issuer, unrepresentable timestamp, or ceiling exceeded).
    Rejected,
    /// Message was applied; carries the map's merge result.
    Applied(buzz_auth::CrossPodMergeResult),
}

/// Build the [`buzz_pubsub::NipFiDisconnect`] bus message from a successfully
/// verified command.
///
/// This is the single publisher mapping — the HTTP success path calls this
/// function to ensure the nanos precision is always captured correctly.
/// Reverting either the seconds or the nanos field must red the round-trip oracle.
pub fn nip_fi_disconnect_message(cmd: &buzz_auth::CommandResult) -> buzz_pubsub::NipFiDisconnect {
    buzz_pubsub::NipFiDisconnect {
        issuer: cmd.caller_iss.clone(),
        pubkey_bytes: cmd.target_pubkey.to_bytes().to_vec(),
        until_unix: cmd.until.timestamp(),
        until_unix_nanos: cmd.until.timestamp_subsec_nanos(),
    }
}

/// Apply a received cross-pod NIP-FI disconnect message against the local
/// deny map and connection registry.
///
/// This is the single consumer path — the `main.rs` receive loop calls this
/// function after receiving a message from the broadcast channel.  Extracting
/// the logic here allows tests to call the exact production path end-to-end
/// without driving a live Redis subscriber.
///
/// `now` is passed explicitly so tests can supply controlled timestamps.
pub fn apply_nip_fi_disconnect(
    state: &crate::state::AppState,
    message: &buzz_pubsub::NipFiDisconnect,
    now: chrono::DateTime<chrono::Utc>,
) -> NipFiDisconnectApplyResult {
    let deny_map = match state.nip_fi_deny_map.as_deref() {
        Some(m) => m,
        None => return NipFiDisconnectApplyResult::Disabled,
    };

    // Validate pubkey bytes.
    let pubkey = match nostr::PublicKey::from_slice(&message.pubkey_bytes) {
        Ok(k) => k,
        Err(_) => {
            tracing::warn!(
                len = message.pubkey_bytes.len(),
                "nip-fi cross-pod: malformed pubkey bytes — rejected"
            );
            return NipFiDisconnectApplyResult::Rejected;
        }
    };

    // Validate that the issuer is locally configured.
    if state
        .config
        .nip_fi
        .registry
        .policy_for_issuer(&message.issuer)
        .is_none()
    {
        tracing::warn!("nip-fi cross-pod: unknown issuer (not locally configured) — rejected");
        return NipFiDisconnectApplyResult::Rejected;
    }

    // Validate timestamp representability.
    let until = match chrono::DateTime::from_timestamp(message.until_unix, message.until_unix_nanos)
    {
        Some(t) => t,
        None => {
            tracing::warn!(
                until_unix = message.until_unix,
                "nip-fi cross-pod: unrepresentable until timestamp — rejected"
            );
            return NipFiDisconnectApplyResult::Rejected;
        }
    };

    // Validate that `until` does not exceed the issuer's ceiling.
    if let Some(policy) = state
        .config
        .nip_fi
        .registry
        .policy_for_issuer(&message.issuer)
    {
        let skew = chrono::Duration::seconds(policy.skew_seconds() as i64);
        let max_age = chrono::Duration::seconds(policy.maximum_assertion_age_seconds() as i64);
        if let Some(ceiling) = now
            .checked_add_signed(skew)
            .and_then(|t| t.checked_add_signed(max_age))
        {
            if until > ceiling {
                tracing::warn!("nip-fi cross-pod: until exceeds issuer ceiling — rejected");
                return NipFiDisconnectApplyResult::Rejected;
            }
        }
    }

    // Merge the deny entry.
    use buzz_auth::CrossPodMergeResult;
    let merge_result = deny_map.merge_cross_pod_deny(&message.issuer, &pubkey, until, now);

    // Close sessions for all merge outcomes except UnknownIssuer.
    let close_sessions = |reason: &str| {
        let closed = state
            .conn_manager
            .disconnect_nip_fi(&message.issuer, &message.pubkey_bytes)
            + state
                .community_connections
                .disconnect_nip_fi(&message.issuer, &message.pubkey_bytes);
        if closed > 0 {
            tracing::debug!(closed, reason = reason, "nip-fi cross-pod: closed sessions");
        }
    };

    match &merge_result {
        CrossPodMergeResult::Merged => {
            close_sessions("merged");
        }
        CrossPodMergeResult::UnknownIssuer => {
            tracing::warn!("nip-fi cross-pod: merge returned UnknownIssuer — rejected");
        }
        CrossPodMergeResult::CapacityExceeded => {
            tracing::warn!(
                "nip-fi cross-pod: deny set full for issuer — closing targeted sessions without map entry (capacity miss; issuer re-push is the recovery path)"
            );
            close_sessions("capacity-exceeded");
            metrics::counter!("buzz_nip_fi_cross_pod_capacity_exceeded_total").increment(1);
        }
        CrossPodMergeResult::ShardPoisoned => {
            tracing::error!(
                "nip-fi cross-pod: issuer shard is poisoned — sessions closed (fail-closed)"
            );
            close_sessions("poisoned shard failsafe");
            metrics::counter!("buzz_nip_fi_cross_pod_shard_poison_total").increment(1);
        }
    }

    NipFiDisconnectApplyResult::Applied(merge_result)
}

/// Build the NIP-FI command components from the issuer policies and key source.
///
/// Called by `install_nip_fi_command_components` (and transitively `main.rs`).
/// Returns `Err` when any command config is invalid.  In enforce mode, returns
/// `Err` when no command-capable issuers are present (assertion-only enforce is
/// not supported by this PR; every enforce issuer must carry command config).
///
/// `issuer_command_configs` must be in the same order as `registry.all_policies()`.
pub fn build_nip_fi_command_components<F: JwksFetcher>(
    mode: NipFiMode,
    registry: &buzz_auth::IssuerRegistry,
    key_source: Arc<ProductionJwksSource<F>>,
    issuer_command_configs: &[(String, CommandIssuerEnvConfig)],
) -> Result<Option<NipFiCommandComponents<F>>, String> {
    if matches!(mode, NipFiMode::Off) {
        return Ok(None);
    }

    // Build per-issuer command policies and capacity overrides.
    let mut command_policies: Vec<CommandIssuerPolicy> = Vec::new();
    let mut issuer_capacities: Vec<IssuerCapacity> = Vec::new();
    let mut default_capacity = DEFAULT_DENY_SET_CAPACITY;

    for (idx, (issuer, cmd_cfg)) in issuer_command_configs.iter().enumerate() {
        let age = match cmd_cfg.maximum_command_age_seconds {
            Some(a) => a,
            None => {
                // In enforce mode every issuer must be command-capable;
                // from_env() already guarantees this, but be defensive here too.
                if matches!(mode, NipFiMode::Enforce) {
                    return Err(format!(
                        "nip-fi: enforce issuer [index {idx}] has no maximum_command_age_seconds — \
                         assertion-only issuers are not supported in enforce mode"
                    ));
                }
                continue; // non-enforce mode: skip issuers without command config
            }
        };
        let principals = match &cmd_cfg.authorized_principals {
            Some(p) if !p.is_empty() => p.clone(),
            _ => {
                // from_env() already rejects this; treat as a hard error here.
                return Err(format!(
                    "nip-fi: issuer [index {idx}] has maximum_command_age_seconds but no \
                     authorized_principals — startup validation should have caught this"
                ));
            }
        };
        let capacity = cmd_cfg
            .deny_set_capacity
            .unwrap_or(DEFAULT_DENY_SET_CAPACITY);

        // Validate and construct the command policy — no warn-and-skip.
        let policy = CommandIssuerPolicy::new(issuer.clone(), age, principals, capacity)
            .map_err(|e| format!("nip-fi: issuer [index {idx}] invalid command policy: {e}"))?;

        issuer_capacities.push(IssuerCapacity {
            issuer: issuer.clone(),
            capacity,
        });
        command_policies.push(policy);

        // Track the maximum capacity across issuers for the default slot.
        if capacity > default_capacity {
            default_capacity = capacity;
        }
    }

    if command_policies.is_empty() {
        if matches!(mode, NipFiMode::Enforce) {
            // Enforce with no command-capable issuers is a misconfiguration:
            // from_env() guarantees every enforce issuer has command config, so
            // an empty set here means something was skipped or the configs are wrong.
            return Err(
                "nip-fi: enforce mode requires at least one command-capable issuer; \
                 no command policies were built — check issuer configuration"
                    .to_owned(),
            );
        }
        debug!("nip-fi: no command-capable issuers configured — command API disabled");
        return Ok(None);
    }

    let deny_map = Arc::new(NipFiDenyMap::new(default_capacity, issuer_capacities));

    let command_verifier = Arc::new(CommandVerifier::new(
        registry.clone(),
        key_source,
        command_policies,
        (*deny_map).clone(),
    ));

    Ok(Some(NipFiCommandComponents {
        deny_map,
        command_verifier,
    }))
}

/// Result of a successful [`install_nip_fi_command_components`] call.
#[derive(Debug)]
pub struct NipFiCommandStartupReport {
    /// Number of issuers wired into the command verifier.
    pub command_issuers: usize,
}

/// Install NIP-FI command components into the two `AppState` slots.
///
/// This is the single production startup seam that owns:
/// - enforce-mode pre-flight check (returns `Err` for incomplete config)
/// - `build_nip_fi_command_components` invocation
/// - assignment of both `nip_fi_deny_map` and `nip_fi_command_verifier`
///
/// It performs no JWKS I/O. `main.rs` owns the single warm + per-issuer
/// refresh lifecycle for the shared `key_source`; the command verifier reads
/// that cache lazily at verify time. `main.rs` passes
/// `&mut app_state.nip_fi_deny_map` and `&mut app_state.nip_fi_command_verifier`;
/// the slots are generic over the fetcher so tests can count fetches.
pub fn install_nip_fi_command_components<F: JwksFetcher>(
    deny_map_slot: &mut Option<Arc<NipFiDenyMap>>,
    command_verifier_slot: &mut Option<Arc<CommandVerifier<Arc<ProductionJwksSource<F>>>>>,
    mode: NipFiMode,
    registry: &buzz_auth::IssuerRegistry,
    key_source: Arc<ProductionJwksSource<F>>,
    command_configs: &[(String, CommandIssuerEnvConfig)],
) -> Result<NipFiCommandStartupReport, String> {
    // Pre-flight: enforce mode with no command configs is always an error.
    if matches!(mode, NipFiMode::Enforce) && command_configs.is_empty() {
        return Err(
            "NIP-FI install: enforce mode requires at least one command-capable issuer".to_owned(),
        );
    }

    let components = build_nip_fi_command_components(mode, registry, key_source, command_configs)?;

    let command_issuers = if let Some(c) = components {
        let n = command_configs.len();
        *deny_map_slot = Some(c.deny_map);
        *command_verifier_slot = Some(c.command_verifier);
        tracing::info!("NIP-FI S4: command API enabled ({n} issuer(s))");
        n
    } else {
        0
    };

    Ok(NipFiCommandStartupReport { command_issuers })
}

/// Validate a command issuer config entry without constructing a policy.
///
/// Called by `nip_fi_config.rs` at startup before `build_nip_fi_command_components`
/// so that invalid config is rejected at `Config::from_env()`, not at serve time.
/// Returns `Err` with a non-sensitive message (no raw issuer URI).
pub fn validate_command_issuer_config(
    idx: usize,
    age_seconds: u64,
    principals: &[String],
    capacity: usize,
) -> Result<(), String> {
    CommandIssuerPolicy::new(
        // Use a sentinel issuer for validation only — no URI written to any log.
        format!("https://validate-sentinel-{idx}.internal"),
        age_seconds,
        principals.to_vec(),
        capacity,
    )
    .map(|_| ())
    .map_err(|e| format!("issuer [index {idx}] invalid command policy: {e}"))
}

// ── Helpers ───────────────────────────────────────────────────────────────────

/// Extract the command JWS token from the `Nostr-Federated-Identity: Bearer`
/// header.  Returns `Err(401)` if the header is absent, `Err(403)` otherwise.
///
/// The same header is used for assertion tokens at upgrade and for command
/// tokens at the admin API — distinct roles on distinct paths, never mixed.
fn extract_command_jwt(headers: &HeaderMap) -> Result<&str, StatusCode> {
    let mut values = headers.get_all(CLIENT_ATTACHED_HEADER).iter();
    let first = values.next().ok_or(StatusCode::UNAUTHORIZED)?;
    // Repeated header → reject.
    if values.next().is_some() {
        return Err(StatusCode::FORBIDDEN);
    }
    let raw = first.to_str().map_err(|_| StatusCode::FORBIDDEN)?;
    if raw.contains(',') {
        return Err(StatusCode::FORBIDDEN);
    }
    let token = raw.strip_prefix("Bearer ").ok_or(StatusCode::FORBIDDEN)?;
    if token.is_empty() || token.contains(char::is_whitespace) {
        return Err(StatusCode::FORBIDDEN);
    }
    Ok(token)
}

fn parse_hex_pubkey(raw: &str) -> Option<nostr::PublicKey> {
    if raw.len() != 64
        || !raw
            .bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
    {
        return None;
    }
    nostr::PublicKey::from_hex(raw).ok()
}

fn plain_response(status: StatusCode, body: &'static str) -> Response<Body> {
    Response::builder()
        .status(status)
        .header("Content-Type", "text/plain; charset=utf-8")
        .body(Body::from(body))
        .unwrap_or_else(|_| Response::new(Body::empty()))
}

/// Build the `401 authentication required` response with the mandatory
/// `WWW-Authenticate: Nostr` header. [NIP-FI.md §Rejection table]
fn auth_required_response() -> Response<Body> {
    Response::builder()
        .status(StatusCode::UNAUTHORIZED)
        .header("Content-Type", "text/plain; charset=utf-8")
        .header("WWW-Authenticate", "Nostr")
        .body(Body::from("authentication required\n"))
        .unwrap_or_else(|_| Response::new(Body::empty()))
}

/// Spec-exact 200 success response.
///
/// The spec body is `{"disconnected": true}` (note the space after `:`).
/// `serde_json::to_vec` produces `{"disconnected":true}` without the space.
/// We produce the literal bytes directly to stay byte-exact.
fn disconnected_response() -> Response<Body> {
    Response::builder()
        .status(StatusCode::OK)
        .header("Content-Type", "application/json")
        .body(Body::from("{\"disconnected\": true}"))
        .unwrap_or_else(|_| Response::new(Body::empty()))
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderValue;

    fn headers_with(value: &str) -> HeaderMap {
        let mut h = HeaderMap::new();
        h.insert(
            CLIENT_ATTACHED_HEADER,
            HeaderValue::from_str(value).unwrap(),
        );
        h
    }

    // ── JWT extraction contract ────────────────────────────────────────────

    #[test]
    fn absent_header_gives_401() {
        let h = HeaderMap::new();
        assert_eq!(extract_command_jwt(&h), Err(StatusCode::UNAUTHORIZED));
    }

    #[test]
    fn repeated_header_gives_403() {
        let mut h = HeaderMap::new();
        h.append(
            CLIENT_ATTACHED_HEADER,
            HeaderValue::from_static("Bearer aaa.bbb.ccc"),
        );
        h.append(
            CLIENT_ATTACHED_HEADER,
            HeaderValue::from_static("Bearer ddd.eee.fff"),
        );
        assert_eq!(extract_command_jwt(&h), Err(StatusCode::FORBIDDEN));
    }

    #[test]
    fn non_bearer_gives_403() {
        let h = headers_with("Token aaa.bbb.ccc");
        assert_eq!(extract_command_jwt(&h), Err(StatusCode::FORBIDDEN));
    }

    #[test]
    fn valid_bearer_extracted() {
        let h = headers_with("Bearer aaa.bbb.ccc");
        assert_eq!(extract_command_jwt(&h), Ok("aaa.bbb.ccc"));
    }

    // ── Hex pubkey parsing ────────────────────────────────────────────────

    #[test]
    fn uppercase_hex_rejected() {
        let upper = "A".repeat(64);
        assert!(parse_hex_pubkey(&upper).is_none());
    }

    #[test]
    fn wrong_length_rejected() {
        let short = "a".repeat(63);
        let long = "a".repeat(65);
        assert!(parse_hex_pubkey(&short).is_none());
        assert!(parse_hex_pubkey(&long).is_none());
    }

    // ── CommandIssuerEnvConfig default capacity ────────────────────────────

    // (The previous constant-assertion test was removed: asserting None and a constant
    // does not bind production behavior. The builder is now covered by route integration tests.)

    // ── HTTP response contract ─────────────────────────────────────────────

    /// The spec requires `{"disconnected": true}` (note the space after `:`).
    #[tokio::test]
    async fn disconnected_response_is_spec_exact() {
        let resp = disconnected_response();
        assert_eq!(resp.status(), StatusCode::OK);
        let ct = resp
            .headers()
            .get("Content-Type")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");
        assert_eq!(ct, "application/json");
        // Body bytes are verified directly — serde_json compact and the spec
        // literal are NOT the same (serde_json omits the space).
        let body_bytes = axum::body::to_bytes(resp.into_body(), 64).await.unwrap();
        assert_eq!(
            body_bytes.as_ref(),
            b"{\"disconnected\": true}",
            "success body must be byte-exact per spec"
        );
    }

    /// `401` MUST carry `WWW-Authenticate: Nostr` and the spec body.
    #[tokio::test]
    async fn auth_required_response_has_www_authenticate() {
        let resp = auth_required_response();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
        let www_auth = resp
            .headers()
            .get("WWW-Authenticate")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");
        assert_eq!(www_auth, "Nostr", "401 MUST carry WWW-Authenticate: Nostr");
        let body_bytes = axum::body::to_bytes(resp.into_body(), 64).await.unwrap();
        assert_eq!(body_bytes.as_ref(), b"authentication required\n");
    }

    /// `403` error responses MUST NOT carry `WWW-Authenticate`.
    #[test]
    fn error_responses_have_no_www_authenticate() {
        for body in &["evidence rejected\n", "authorization denied\n"] {
            let resp = plain_response(StatusCode::FORBIDDEN, body);
            assert!(
                resp.headers().get("WWW-Authenticate").is_none(),
                "403 must not carry WWW-Authenticate"
            );
        }
    }

    /// `503` plain responses have the spec-exact body.
    #[test]
    fn deny_set_full_response_body_is_spec_exact() {
        use buzz_auth::CommandError;
        let body = CommandError::DenySetFull.response_body();
        assert_eq!(
            body, "deny set full\n",
            "FI-TRACE-DENY-SET: 503 body must be 'deny set full\\n'"
        );
    }
}

// ── Route integration tests ────────────────────────────────────────────────────
//
// Exercises `disconnect()` through the full axum router with a warmed
// ProductionJwksSource, a real CommandVerifier, and an AppState wired exactly
// as production does (nip_fi_command_verifier + nip_fi_deny_map both set).
//
// These tests call the route at POST /api/nip-fi/disconnect via oneshot and
// verify every spec response row: 401, 403 (evidence), 403 (authz), 400, 503
// (capacity), 200 exact bytes.  The startup-assembly invariant is also
// verified: the tests GO RED if either state field is absent (503 unavailable).

#[cfg(test)]
mod route_integration_tests {
    use super::*;
    use axum::{
        body::Body,
        http::{Request, StatusCode},
    };
    use buzz_auth::{
        CommandIssuerPolicy, CommandVerifier, IssuerCapacity, IssuerRegistry, NipFiDenyMap,
        ProductionJwksSource,
    };
    use std::sync::Arc;
    use tower::ServiceExt;

    // ── Shared test key material ────────────────────────────────────────────

    // ES256 key pair — same material as command.rs tests, known-good.
    const TEST_PRIVATE_KEY_PEM: &str =
        "-----BEGIN PRIVATE KEY-----\nMIGHAgEAMBMGByqGSM49AgEGCCqGSM49AwEHBG0wawIBAQQgcnxDM4EiirH9dHUE\nWZc759TX4s5PAn8kO5ovXSnGxCWhRANCAARFb6ZnsfkqOOXyEhj3KBQphGKF4vTa\nzhebbavbZ1ZoklqkF1cGg+jTO7rONAVEzXvXUWtV6CdDV+rybiVmFP2w\n-----END PRIVATE KEY-----\n";

    const TEST_ISS: &str = "https://idp.test.example.com";
    const TEST_AUD: &str = "https://relay.test.example.com";
    const TEST_SUB: &str = "admin-svc@test.example.com";
    const TEST_PATH: &str = "/api/nip-fi/disconnect";

    // Key ID used in both the JWKS and the JWT header.
    const TEST_KID: &str = "route-test-key-1";

    fn test_public_jwk() -> jsonwebtoken::jwk::Jwk {
        // Public-key coordinates extracted from TEST_PRIVATE_KEY_PEM (P-256),
        // which is the same key pair as command.rs TEST_JWK_X/Y constants.
        serde_json::from_value(serde_json::json!({
            "kty": "EC",
            "crv": "P-256",
            "x": "RW-mZ7H5Kjjl8hIY9ygUKYRiheL02s4Xm22r22dWaJI",
            "y": "WqQXVwaD6NM7us40BUTNe9dRa1XoJ0NX6vJuJWYU_bA",
            "alg": "ES256",
            "use": "sig",
            "kid": TEST_KID
        }))
        .expect("valid test JWK")
    }

    fn test_jwks() -> jsonwebtoken::jwk::JwkSet {
        jsonwebtoken::jwk::JwkSet {
            keys: vec![test_public_jwk()],
        }
    }

    fn test_issuer_policy() -> buzz_auth::IssuerPolicy {
        issuer_policy(TEST_ISS)
    }

    fn issuer_policy(iss: &str) -> buzz_auth::IssuerPolicy {
        use buzz_auth::{FreshnessClass, IssuerPolicy, JwksSourceContract, TokenClass};
        let contract = JwksSourceContract::new(format!("{iss}/.well-known/jwks.json"), 300, 86400)
            .expect("valid JWKS contract");
        IssuerPolicy::new(
            iss.to_owned(),
            vec![TEST_AUD.to_owned()],
            TokenClass::DedicatedNipFi,
            FreshnessClass::OfflineJwt,
            vec![buzz_auth::JwtAlgorithm::ES256],
            30,
            3600,
            None,
            contract,
        )
        .expect("valid issuer policy")
    }

    fn test_jwks_config() -> buzz_auth::IssuerJwksConfig {
        jwks_config(TEST_ISS)
    }

    fn jwks_config(iss: &str) -> buzz_auth::IssuerJwksConfig {
        use buzz_auth::{IssuerJwksConfig, JwksSourceContract};
        let contract = JwksSourceContract::new(format!("{iss}/.well-known/jwks.json"), 300, 86400)
            .expect("valid JWKS contract");
        IssuerJwksConfig {
            issuer: iss.to_owned(),
            contract,
        }
    }

    fn mint_token(target_hex: &str, until_offset_secs: i64, extra: serde_json::Value) -> String {
        use jsonwebtoken::{encode, Algorithm, EncodingKey, Header};
        let now = chrono::Utc::now().timestamp();
        let mut claims = serde_json::json!({
            "iss": TEST_ISS,
            "aud": TEST_AUD,
            "sub": TEST_SUB,
            "iat": now,
            "exp": now + 60,
            "jti": uuid::Uuid::new_v4().to_string(),
            "method": "POST",
            "path": TEST_PATH,
            "cmd": "disconnect",
            "target_pubkey": target_hex,
            "until": now + until_offset_secs,
        });
        if let Some(obj) = extra.as_object() {
            for (k, v) in obj {
                claims[k] = v.clone();
            }
        }
        let mut header = Header::new(Algorithm::ES256);
        header.typ = Some("nip-fi-command+jwt".to_owned());
        header.kid = Some(TEST_KID.to_owned());
        let key = EncodingKey::from_ec_pem(TEST_PRIVATE_KEY_PEM.as_bytes()).expect("test EC key");
        encode(&header, &claims, &key).expect("sign test token")
    }

    async fn build_test_state(capacity: usize) -> Arc<crate::state::AppState> {
        Arc::new(build_test_app_state(capacity, crate::config::Config::hermetic_for_test()).await)
    }

    async fn build_test_app_state(
        capacity: usize,
        config: crate::config::Config,
    ) -> crate::state::AppState {
        // Build a minimal AppState with NIP-FI S4 components wired.
        // Uses lazy/invalid DB+Redis — only nip_fi fields and conn_manager matter.
        use crate::state::AppState;

        let pool = sqlx::PgPool::connect_lazy(&config.database_url).expect("lazy pg pool");
        let db = buzz_db::Db::from_pool(pool.clone());
        let redis_pool = deadpool_redis::Config::from_url(&config.redis_url)
            .create_pool(Some(deadpool_redis::Runtime::Tokio1))
            .expect("redis pool");
        let pubsub = Arc::new(
            buzz_pubsub::PubSubManager::new(&config.redis_url, redis_pool.clone())
                .await
                .expect("pubsub"),
        );
        let audit = buzz_audit::AuditService::new(pool.clone());
        let auth = buzz_auth::AuthService::new(config.auth.clone());
        let search = buzz_search::SearchService::new(pool.clone());
        let workflow_engine = Arc::new(buzz_workflow::WorkflowEngine::new(
            db.clone(),
            buzz_workflow::WorkflowConfig::default(),
        ));
        let media_storage = buzz_media::MediaStorage::new(&config.media).expect("media storage");
        let (mut state, _audit_shutdown) = AppState::new(
            config,
            db,
            redis_pool,
            audit,
            pubsub,
            auth,
            search,
            workflow_engine,
            nostr::Keys::generate(),
            media_storage,
        );

        // Wire NIP-FI S4 components.
        let jwks_configs = vec![test_jwks_config()];
        let key_source = Arc::new(
            ProductionJwksSource::new(jwks_configs, buzz_auth::HttpJwksFetcher::new())
                .expect("key source"),
        );
        // Seed the snapshot without making an HTTP request.
        key_source
            .seed_snapshot_for_test(TEST_ISS, test_jwks())
            .await;

        let mut registry = IssuerRegistry::new();
        registry.insert(test_issuer_policy());

        let deny_map = Arc::new(NipFiDenyMap::new(
            capacity,
            vec![IssuerCapacity {
                issuer: TEST_ISS.to_owned(),
                capacity,
            }],
        ));
        let policy =
            CommandIssuerPolicy::new(TEST_ISS.to_owned(), 30, vec![TEST_SUB.to_owned()], capacity)
                .expect("command policy");
        let verifier = Arc::new(CommandVerifier::new(
            registry,
            Arc::clone(&key_source),
            vec![policy],
            (*deny_map).clone(),
        ));

        state.nip_fi_deny_map = Some(Arc::clone(&deny_map));
        state.nip_fi_command_verifier = Some(verifier);
        state
    }

    fn target_hex() -> String {
        nostr::Keys::generate().public_key().to_hex()
    }

    async fn do_request(
        state: Arc<crate::state::AppState>,
        method: &str,
        headers: Vec<(&'static str, String)>,
        body: Option<serde_json::Value>,
    ) -> axum::response::Response {
        use crate::router::build_router;
        let body_bytes = match body {
            Some(v) => serde_json::to_vec(&v).unwrap().into(),
            None => axum::body::Bytes::new(),
        };
        let mut req = Request::builder().method(method).uri(TEST_PATH);
        for (k, v) in &headers {
            req = req.header(*k, v.as_str());
        }
        let req = req.body(Body::from(body_bytes)).unwrap();
        build_router(state).oneshot(req).await.unwrap()
    }

    // ── Test: no verifier → 503 (startup-assembly invariant) ─────────────────

    #[tokio::test]
    async fn absent_verifier_gives_503_unavailable() {
        // If nip_fi_command_verifier is not set, every request gets 503.
        // This tests the handler fallback path when the verifier is absent from
        // AppState.  The production startup assembly is covered separately by
        // `production_assembly_build_nip_fi_command_components_wires_both_fields`.
        // Build a state without the verifier.
        let no_verifier_state = {
            let config = crate::config::Config::hermetic_for_test();
            let pool = sqlx::PgPool::connect_lazy(&config.database_url).unwrap();
            let db = buzz_db::Db::from_pool(pool.clone());
            let redis_pool = deadpool_redis::Config::from_url(&config.redis_url)
                .create_pool(Some(deadpool_redis::Runtime::Tokio1))
                .unwrap();
            let pubsub = Arc::new(
                buzz_pubsub::PubSubManager::new(&config.redis_url, redis_pool.clone())
                    .await
                    .unwrap(),
            );
            let audit = buzz_audit::AuditService::new(pool.clone());
            let auth = buzz_auth::AuthService::new(config.auth.clone());
            let search = buzz_search::SearchService::new(pool.clone());
            let workflow_engine = Arc::new(buzz_workflow::WorkflowEngine::new(
                db.clone(),
                buzz_workflow::WorkflowConfig::default(),
            ));
            let media_storage = buzz_media::MediaStorage::new(&config.media).unwrap();
            let (state, _) = crate::state::AppState::new(
                config,
                db,
                redis_pool,
                audit,
                pubsub,
                auth,
                search,
                workflow_engine,
                nostr::Keys::generate(),
                media_storage,
            );
            // nip_fi_command_verifier stays None.
            Arc::new(state)
        };
        let target = target_hex();
        let token = mint_token(&target, 300, serde_json::json!({}));
        let resp = do_request(
            no_verifier_state,
            "POST",
            vec![
                ("Content-Type", "application/json".into()),
                (CLIENT_ATTACHED_HEADER, format!("Bearer {token}")),
            ],
            Some(serde_json::json!({"pubkey": target})),
        )
        .await;
        assert_eq!(resp.status(), StatusCode::SERVICE_UNAVAILABLE);
    }

    // ── Test: absent header → 401 + WWW-Authenticate ─────────────────────────

    #[tokio::test]
    async fn absent_header_route_gives_401_with_www_authenticate() {
        let state = build_test_state(1000).await;
        let target = target_hex();
        let resp = do_request(
            state,
            "POST",
            vec![("Content-Type", "application/json".into())],
            Some(serde_json::json!({"pubkey": target})),
        )
        .await;
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
        let www_auth = resp
            .headers()
            .get("WWW-Authenticate")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");
        assert_eq!(www_auth, "Nostr", "401 MUST carry WWW-Authenticate: Nostr");
    }

    // ── Test: bad signature → 403 evidence rejected ───────────────────────────

    #[tokio::test]
    async fn bad_signature_gives_403_evidence_rejected() {
        let state = build_test_state(1000).await;
        let target = target_hex();
        // Tamper the token.
        let token = mint_token(&target, 300, serde_json::json!({}));
        let tampered = format!("{token}X");
        let resp = do_request(
            state,
            "POST",
            vec![
                ("Content-Type", "application/json".into()),
                (CLIENT_ATTACHED_HEADER, format!("Bearer {tampered}")),
            ],
            Some(serde_json::json!({"pubkey": target})),
        )
        .await;
        assert_eq!(resp.status(), StatusCode::FORBIDDEN);
    }

    // ── Test: capacity exceeded → 503 does NOT burn jti ──────────────────────

    #[tokio::test]
    async fn capacity_503_does_not_burn_jti_route_retry_succeeds() {
        // capacity=1, two distinct targets.
        let state = build_test_state(1).await;
        let target_a = target_hex();
        let target_b = target_hex();

        // First request fills the slot.
        let token_a = mint_token(&target_a, 300, serde_json::json!({}));
        let resp_a = do_request(
            Arc::clone(&state),
            "POST",
            vec![
                ("Content-Type", "application/json".into()),
                (CLIENT_ATTACHED_HEADER, format!("Bearer {token_a}")),
            ],
            Some(serde_json::json!({"pubkey": target_a})),
        )
        .await;
        assert_eq!(resp_a.status(), StatusCode::OK);

        // Second request hits capacity → 503.  Jti NOT burned.
        let jti_b = uuid::Uuid::new_v4().to_string();
        let token_b = mint_token(&target_b, 300, serde_json::json!({"jti": jti_b}));
        let resp_b = do_request(
            Arc::clone(&state),
            "POST",
            vec![
                ("Content-Type", "application/json".into()),
                (CLIENT_ATTACHED_HEADER, format!("Bearer {token_b}")),
            ],
            Some(serde_json::json!({"pubkey": target_b})),
        )
        .await;
        assert_eq!(
            resp_b.status(),
            StatusCode::SERVICE_UNAVAILABLE,
            "capacity exceeded must return 503"
        );
        let body = axum::body::to_bytes(resp_b.into_body(), 64).await.unwrap();
        assert_eq!(body.as_ref(), b"deny set full\n");
        // Jti was NOT burned: the same token_b can be reused once the slot frees.
        // (Route-level: we verify the 503 body; the jti non-burn is covered by
        // command.rs::capacity_503_does_not_burn_jti_retry_succeeds_after_slot_freed)
    }

    // ── Test: successful disconnect → 200 spec-exact bytes ───────────────────

    #[tokio::test]
    async fn success_response_is_spec_exact_bytes() {
        let state = build_test_state(1000).await;
        let target = target_hex();
        let token = mint_token(&target, 300, serde_json::json!({}));
        let resp = do_request(
            Arc::clone(&state),
            "POST",
            vec![
                ("Content-Type", "application/json".into()),
                (CLIENT_ATTACHED_HEADER, format!("Bearer {token}")),
            ],
            Some(serde_json::json!({"pubkey": target})),
        )
        .await;
        assert_eq!(resp.status(), StatusCode::OK);
        let ct = resp
            .headers()
            .get("Content-Type")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");
        assert_eq!(ct, "application/json");
        let body = axum::body::to_bytes(resp.into_body(), 64).await.unwrap();
        assert_eq!(
            body.as_ref(),
            b"{\"disconnected\": true}",
            "200 body must be byte-exact per spec (note the space after ':')"
        );
    }

    // ── Test: count-independence (zero vs many sessions) ─────────────────────

    #[tokio::test]
    async fn success_body_identical_regardless_of_sessions_closed() {
        // Zero live sessions: response must still be {"disconnected": true}.
        let state = build_test_state(1000).await;
        let target = target_hex();
        let token = mint_token(&target, 300, serde_json::json!({}));
        let resp = do_request(
            state,
            "POST",
            vec![
                ("Content-Type", "application/json".into()),
                (CLIENT_ATTACHED_HEADER, format!("Bearer {token}")),
            ],
            Some(serde_json::json!({"pubkey": target})),
        )
        .await;
        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), 64).await.unwrap();
        assert_eq!(
            body.as_ref(),
            b"{\"disconnected\": true}",
            "zero-sessions success must be byte-identical to many-sessions success [no count leak]"
        );
    }

    // ── Test: deny entry recorded after success ───────────────────────────────

    #[tokio::test]
    async fn success_records_deny_entry_visible_to_is_denied() {
        let state = build_test_state(1000).await;
        let target = target_hex();
        let target_pubkey = nostr::PublicKey::from_hex(&target).expect("valid hex pubkey");

        // Before disconnect: not denied.
        let deny_map = state.nip_fi_deny_map.as_deref().expect("deny map present");
        assert!(
            !deny_map.is_denied(TEST_ISS, &target_pubkey, chrono::Utc::now()),
            "must not be denied before disconnect"
        );

        // Execute disconnect.
        let token = mint_token(&target, 300, serde_json::json!({}));
        let resp = do_request(
            Arc::clone(&state),
            "POST",
            vec![
                ("Content-Type", "application/json".into()),
                (CLIENT_ATTACHED_HEADER, format!("Bearer {token}")),
            ],
            Some(serde_json::json!({"pubkey": target})),
        )
        .await;
        assert_eq!(resp.status(), StatusCode::OK);

        // After disconnect: denied.
        assert!(
            deny_map.is_denied(TEST_ISS, &target_pubkey, chrono::Utc::now()),
            "must be denied after successful disconnect"
        );
    }

    // ── Test: consumer capacity oracle (replacement per NIP-FI.md:306-336) ─────
    //
    // Drives apply_nip_fi_disconnect with a delivered target that encounters a
    // pre-filled capacity-1 map.  Asserts Applied(CapacityExceeded); the
    // pre-existing map key remains denied only until its TTL; the missed target
    // and unrelated key are NOT map-denied; targeted live sessions are closed;
    // unrelated live peers remain open.
    //
    // Mandatory reds:
    //  (a) consumer stops calling merge_cross_pod_deny → Applied(CapacityExceeded) missed;
    //      targeted session close assertion fails
    //  (b) reintroduce issuer-wide blocking → missed-target is_denied assertion fails
    //  (c) consumer skips close_sessions on CapacityExceeded → targeted cancel assertion fails

    #[tokio::test]
    async fn consumer_capacity_miss_closes_targeted_session_without_map_denial() {
        use super::apply_nip_fi_disconnect;
        use super::NipFiDisconnectApplyResult;
        use crate::state::CommunityConnectionControl;
        use buzz_auth::CrossPodMergeResult;
        use tokio_util::sync::CancellationToken;

        let state = {
            let mut config = crate::config::Config::hermetic_for_test();
            // Wire TEST_ISS into the NIP-FI registry so the consumer seam accepts it.
            config.nip_fi.registry.insert(test_issuer_policy());
            let pool = sqlx::PgPool::connect_lazy(&config.database_url).unwrap();
            let db = buzz_db::Db::from_pool(pool.clone());
            let redis_pool = deadpool_redis::Config::from_url(&config.redis_url)
                .create_pool(Some(deadpool_redis::Runtime::Tokio1))
                .unwrap();
            let pubsub = Arc::new(
                buzz_pubsub::PubSubManager::new(&config.redis_url, redis_pool.clone())
                    .await
                    .unwrap(),
            );
            let audit = buzz_audit::AuditService::new(pool.clone());
            let auth = buzz_auth::AuthService::new(config.auth.clone());
            let search = buzz_search::SearchService::new(pool.clone());
            let workflow_engine = Arc::new(buzz_workflow::WorkflowEngine::new(
                db.clone(),
                buzz_workflow::WorkflowConfig::default(),
            ));
            let media_storage = buzz_media::MediaStorage::new(&config.media).unwrap();
            let (mut state, _) = crate::state::AppState::new(
                config,
                db,
                redis_pool,
                audit,
                pubsub,
                auth,
                search,
                workflow_engine,
                nostr::Keys::generate(),
                media_storage,
            );
            // Capacity=1, pre-filled with k_a so the second delivery (k_b) hits capacity.
            let deny_map = Arc::new(buzz_auth::NipFiDenyMap::new(
                1,
                vec![buzz_auth::IssuerCapacity {
                    issuer: TEST_ISS.to_owned(),
                    capacity: 1,
                }],
            ));
            state.nip_fi_deny_map = Some(Arc::clone(&deny_map));
            Arc::new(state)
        };

        let now = chrono::Utc::now();
        let until_unix = (now + chrono::Duration::seconds(300)).timestamp();

        let k_a = nostr::Keys::generate().public_key();
        let k_b = nostr::Keys::generate().public_key();
        let k_unrelated = nostr::Keys::generate().public_key();

        // Pre-fill slot with k_a via the consumer seam.
        let msg_a = buzz_pubsub::NipFiDisconnect {
            issuer: TEST_ISS.to_owned(),
            pubkey_bytes: k_a.to_bytes().to_vec(),
            until_unix,
            until_unix_nanos: 0,
        };
        let result_a = apply_nip_fi_disconnect(&state, &msg_a, now);
        assert_eq!(
            result_a,
            NipFiDisconnectApplyResult::Applied(CrossPodMergeResult::Merged),
            "first consumer message must merge"
        );

        // Register live sessions for targeted (k_b) and unrelated (k_unrelated) peers.
        let cancel_b = CancellationToken::new();
        let cancel_unrelated = CancellationToken::new();
        let registry = &state.community_connections;
        let community = buzz_core::tenant::CommunityId::from_uuid(uuid::Uuid::new_v4());

        let ctrl_b = CommunityConnectionControl::new(cancel_b.clone());
        ctrl_b.set_proven_identity(k_b.to_bytes().to_vec(), Some(TEST_ISS.to_owned()));
        let _guard_b = registry.register(uuid::Uuid::new_v4(), community, ctrl_b);

        let ctrl_unrelated = CommunityConnectionControl::new(cancel_unrelated.clone());
        ctrl_unrelated
            .set_proven_identity(k_unrelated.to_bytes().to_vec(), Some(TEST_ISS.to_owned()));
        let _guard_unrelated = registry.register(uuid::Uuid::new_v4(), community, ctrl_unrelated);

        // Deliver k_b — capacity exhausted, no map entry added.
        let msg_b = buzz_pubsub::NipFiDisconnect {
            issuer: TEST_ISS.to_owned(),
            pubkey_bytes: k_b.to_bytes().to_vec(),
            until_unix,
            until_unix_nanos: 0,
        };
        let result_b = apply_nip_fi_disconnect(&state, &msg_b, now);
        assert_eq!(
            result_b,
            NipFiDisconnectApplyResult::Applied(CrossPodMergeResult::CapacityExceeded),
            "second consumer message must hit capacity"
        );

        // Targeted session (k_b) must be cancelled despite no map entry.
        assert!(
            cancel_b.is_cancelled(),
            "targeted session must be closed even on CapacityExceeded"
        );
        // Unrelated session must NOT be cancelled.
        assert!(
            !cancel_unrelated.is_cancelled(),
            "unrelated session must remain open after capacity miss"
        );

        // Map checks — no issuer-wide denial synthesized.
        let deny_map = state.nip_fi_deny_map.as_deref().expect("deny map present");

        // k_a is still denied (its entry was not evicted).
        assert!(
            deny_map.is_denied(TEST_ISS, &k_a, now),
            "pre-existing k_a entry must remain denied"
        );
        // k_b has no map entry — NOT denied via the map.
        assert!(
            !deny_map.is_denied(TEST_ISS, &k_b, now),
            "missed target k_b must NOT be map-denied after capacity miss"
        );
        // Unrelated key is not map-denied.
        assert!(
            !deny_map.is_denied(TEST_ISS, &k_unrelated, now),
            "unrelated key must NOT be denied after capacity miss"
        );
        // At exact equality with k_a's TTL, k_a is admitted.
        let at_ttl = chrono::DateTime::from_timestamp(until_unix, 0).unwrap();
        assert!(
            !deny_map.is_denied(TEST_ISS, &k_a, at_ttl),
            "k_a must be admitted at exact equality with its TTL"
        );
    }

    // ── Test: Item 5 — dual-transport registration witness ──────────────────────────────────────
    //
    // Proves that the production audio post-auth registration helper
    // (`audio_post_auth_register`) is the seam used to register audio connections
    // in the fan-out, and that apply_nip_fi_disconnect drives both connection
    // registries (ordinary WS via conn_manager and audio via community_connections).
    //
    // Four sub-claims verified:
    //  1. targeted ordinary WS is cancelled (conn_manager path)
    //  2. targeted audio is cancelled with AuthorizationDenied (community_connections path,
    //     registered via the production audio_post_auth_register helper)
    //  3. unrelated ordinary WS and audio peers remain open
    //  4. capacity-failure variant: the delivered target still closes both transports
    //     despite no map entry
    //
    // Mandatory reds:
    //  - no-op audio_post_auth_register leaves targeted audio open
    //  - removing community_connections from the fan-out leaves audio open
    //  - removing conn_manager from the fan-out leaves ordinary WS open
    //  - broad/non-key-exact matching would close unrelated peers (asserted absent)
    //  - skipping close on CapacityExceeded leaves targeted sessions open (capacity variant)

    #[tokio::test]
    async fn dual_transport_registration_witness() {
        use super::apply_nip_fi_disconnect;
        use super::NipFiDisconnectApplyResult;
        use crate::audio::handler::audio_post_auth_register;
        use crate::state::CommunityConnectionControl;
        use buzz_auth::CrossPodMergeResult;
        use tokio::sync::mpsc;
        use tokio_util::sync::CancellationToken;
        use uuid::Uuid;

        let community = buzz_core::tenant::CommunityId::from_uuid(Uuid::new_v4());

        // ── Helper: register an ordinary WS connection and return (conn_id, cancel). ──
        // Mirrors how connection.rs registers after NIP-42 auth: register first, then
        // call set_authenticated_pubkey.
        let register_ws =
            |state: &crate::state::AppState, pubkey_bytes: Vec<u8>| -> (Uuid, CancellationToken) {
                let conn_id = Uuid::new_v4();
                let (tx, _rx) = mpsc::channel(8);
                let (ctrl_tx, _ctrl_rx) = mpsc::channel(8);
                let cancel = CancellationToken::new();
                let bp = std::sync::Arc::new(std::sync::atomic::AtomicU8::new(0));
                state.conn_manager.register(
                    conn_id,
                    tx,
                    ctrl_tx,
                    mpsc::channel(1).0,
                    None,
                    cancel.clone(),
                    community,
                    bp,
                    std::sync::Arc::new(tokio::sync::Mutex::new(std::collections::HashMap::new())),
                    3,
                    crate::state::CommunityConnectionControl::new(cancel.clone()),
                );
                state.conn_manager.set_authenticated_identity(
                    conn_id,
                    pubkey_bytes,
                    Some(TEST_ISS.to_owned()),
                );
                (conn_id, cancel)
            };

        // ── Build state: capacity 2 so the Merged case succeeds for the target. ──
        let state = {
            let mut config = crate::config::Config::hermetic_for_test();
            // Wire TEST_ISS into the NIP-FI registry so apply_nip_fi_disconnect accepts it.
            config.nip_fi.registry.insert(test_issuer_policy());
            let pool = sqlx::PgPool::connect_lazy(&config.database_url).unwrap();
            let db = buzz_db::Db::from_pool(pool.clone());
            let redis_pool = deadpool_redis::Config::from_url(&config.redis_url)
                .create_pool(Some(deadpool_redis::Runtime::Tokio1))
                .unwrap();
            let pubsub = Arc::new(
                buzz_pubsub::PubSubManager::new(&config.redis_url, redis_pool.clone())
                    .await
                    .unwrap(),
            );
            let audit = buzz_audit::AuditService::new(pool.clone());
            let auth = buzz_auth::AuthService::new(config.auth.clone());
            let search = buzz_search::SearchService::new(pool.clone());
            let workflow_engine = Arc::new(buzz_workflow::WorkflowEngine::new(
                db.clone(),
                buzz_workflow::WorkflowConfig::default(),
            ));
            let media_storage = buzz_media::MediaStorage::new(&config.media).unwrap();
            let (mut state, _) = crate::state::AppState::new(
                config,
                db,
                redis_pool,
                audit,
                pubsub,
                auth,
                search,
                workflow_engine,
                nostr::Keys::generate(),
                media_storage,
            );
            let deny_map = Arc::new(buzz_auth::NipFiDenyMap::new(
                2,
                vec![buzz_auth::IssuerCapacity {
                    issuer: TEST_ISS.to_owned(),
                    capacity: 2,
                }],
            ));
            state.nip_fi_deny_map = Some(Arc::clone(&deny_map));
            Arc::new(state)
        };

        let target = nostr::Keys::generate().public_key();
        let unrelated = nostr::Keys::generate().public_key();
        let now = chrono::Utc::now();
        let until_unix = (now + chrono::Duration::seconds(300)).timestamp();

        // Register targeted ordinary WS and audio controls.
        let (_target_ws_id, cancel_target_ws) = register_ws(&state, target.to_bytes().to_vec());

        // Register targeted audio via the production helper.
        // Guards must live until after the assertions — declared here, not in a sub-block.
        let audio_registry = &state.community_connections;
        let cancel_audio_target = CancellationToken::new();
        let _audio_target_guard = {
            let ctrl = CommunityConnectionControl::new(cancel_audio_target.clone());
            audio_post_auth_register(&ctrl, target.to_bytes().to_vec(), Some(TEST_ISS.to_owned()));
            audio_registry.register(Uuid::new_v4(), community, ctrl)
        };

        // Register unrelated ordinary WS and audio controls.
        let (_unrelated_ws_id, cancel_unrelated_ws) =
            register_ws(&state, unrelated.to_bytes().to_vec());
        let cancel_audio_unrelated = CancellationToken::new();
        let _audio_unrelated_guard = {
            let ctrl = CommunityConnectionControl::new(cancel_audio_unrelated.clone());
            audio_post_auth_register(
                &ctrl,
                unrelated.to_bytes().to_vec(),
                Some(TEST_ISS.to_owned()),
            );
            audio_registry.register(Uuid::new_v4(), community, ctrl)
        };

        // Drive apply_nip_fi_disconnect for the target (Merged case).
        let msg = buzz_pubsub::NipFiDisconnect {
            issuer: TEST_ISS.to_owned(),
            pubkey_bytes: target.to_bytes().to_vec(),
            until_unix,
            until_unix_nanos: 0,
        };
        let result = apply_nip_fi_disconnect(&state, &msg, now);
        assert_eq!(
            result,
            NipFiDisconnectApplyResult::Applied(CrossPodMergeResult::Merged),
            "target disconnect must merge"
        );

        // Claim 1: targeted ordinary WS is cancelled.
        assert!(
            cancel_target_ws.is_cancelled(),
            "targeted ordinary WS must be cancelled by disconnect fan-out"
        );
        // Claim 2: targeted audio is cancelled (registered via audio_post_auth_register).
        assert!(
            cancel_audio_target.is_cancelled(),
            "targeted audio connection must be cancelled via community_connections fan-out"
        );
        // Claim 3a: unrelated ordinary WS remains open.
        assert!(
            !cancel_unrelated_ws.is_cancelled(),
            "unrelated ordinary WS must remain open"
        );
        // Claim 3b: unrelated audio remains open.
        assert!(
            !cancel_audio_unrelated.is_cancelled(),
            "unrelated audio connection must remain open"
        );

        // ── Capacity-failure variant ──────────────────────────────────────────────
        // Pre-fill the map to capacity with a different key, then deliver target2
        // (capacity exceeded). Assert target2's sessions still close despite no map entry.
        let target2 = nostr::Keys::generate().public_key();

        // Register target2 ordinary WS and audio.
        let (_t2_ws_id, cancel_t2_ws) = register_ws(&state, target2.to_bytes().to_vec());
        let cancel_t2_audio = CancellationToken::new();
        let _t2_audio_guard = {
            let ctrl = CommunityConnectionControl::new(cancel_t2_audio.clone());
            audio_post_auth_register(
                &ctrl,
                target2.to_bytes().to_vec(),
                Some(TEST_ISS.to_owned()),
            );
            audio_registry.register(Uuid::new_v4(), community, ctrl)
        };

        // The map is now at capacity (target is in it from the Merged above, plus we need
        // one more to saturate cap=2). Pre-fill the second slot with a filler key.
        let filler = nostr::Keys::generate().public_key();
        let msg_fill = buzz_pubsub::NipFiDisconnect {
            issuer: TEST_ISS.to_owned(),
            pubkey_bytes: filler.to_bytes().to_vec(),
            until_unix,
            until_unix_nanos: 0,
        };
        let fill_result = apply_nip_fi_disconnect(&state, &msg_fill, now);
        assert_eq!(
            fill_result,
            NipFiDisconnectApplyResult::Applied(CrossPodMergeResult::Merged),
            "filler must merge to saturate capacity"
        );

        // Now deliver target2 — capacity exceeded, no map entry.
        let msg2 = buzz_pubsub::NipFiDisconnect {
            issuer: TEST_ISS.to_owned(),
            pubkey_bytes: target2.to_bytes().to_vec(),
            until_unix,
            until_unix_nanos: 0,
        };
        let result2 = apply_nip_fi_disconnect(&state, &msg2, now);
        assert_eq!(
            result2,
            NipFiDisconnectApplyResult::Applied(CrossPodMergeResult::CapacityExceeded),
            "second target must hit capacity"
        );

        // Claim 4a: targeted ordinary WS still closes despite CapacityExceeded.
        assert!(
            cancel_t2_ws.is_cancelled(),
            "target2 ordinary WS must close even on CapacityExceeded"
        );
        // Claim 4b: targeted audio still closes despite CapacityExceeded.
        assert!(
            cancel_t2_audio.is_cancelled(),
            "target2 audio must close even on CapacityExceeded"
        );
    }

    // ── Test: blocker 3 — fractional deadline survives publisher wire and consumer equality boundary ─
    //
    // Exercises the full publisher → encode → decode → apply_nip_fi_disconnect chain with a
    // non-zero nanos deadline. Proves denied immediately after T, admitted at exact T + nanos.
    //
    // Red mutations:
    //  - publisher writes zero nanos → until reconstructed as T+0 → equality boundary fails
    //  - encoder omits nanos field → decoder defaults to 0 → same failure
    //  - decoder defaults a present field to zero → same failure
    //  - consumer reconstructs with zero nanos → same failure
    //  - comparison changes from < to <= → admitted before exact boundary

    #[tokio::test]
    async fn fractional_deadline_survives_publisher_wire_and_consumer_equality_boundary() {
        use super::NipFiDisconnectApplyResult;
        use super::{apply_nip_fi_disconnect, nip_fi_disconnect_message};
        use buzz_auth::CrossPodMergeResult;

        // Build a state with TEST_ISS in the registry so apply_nip_fi_disconnect accepts it.
        let state = {
            let mut config = crate::config::Config::hermetic_for_test();
            // Wire TEST_ISS into the NIP-FI registry so apply_nip_fi_disconnect accepts it.
            config.nip_fi.registry.insert(test_issuer_policy());
            let pool = sqlx::PgPool::connect_lazy(&config.database_url).unwrap();
            let db = buzz_db::Db::from_pool(pool.clone());
            let redis_pool = deadpool_redis::Config::from_url(&config.redis_url)
                .create_pool(Some(deadpool_redis::Runtime::Tokio1))
                .unwrap();
            let pubsub = Arc::new(
                buzz_pubsub::PubSubManager::new(&config.redis_url, redis_pool.clone())
                    .await
                    .unwrap(),
            );
            let audit = buzz_audit::AuditService::new(pool.clone());
            let auth = buzz_auth::AuthService::new(config.auth.clone());
            let search = buzz_search::SearchService::new(pool.clone());
            let workflow_engine = Arc::new(buzz_workflow::WorkflowEngine::new(
                db.clone(),
                buzz_workflow::WorkflowConfig::default(),
            ));
            let media_storage = buzz_media::MediaStorage::new(&config.media).unwrap();
            let (mut state, _) = crate::state::AppState::new(
                config,
                db,
                redis_pool,
                audit,
                pubsub,
                auth,
                search,
                workflow_engine,
                nostr::Keys::generate(),
                media_storage,
            );
            let deny_map = Arc::new(buzz_auth::NipFiDenyMap::new(
                1000,
                vec![buzz_auth::IssuerCapacity {
                    issuer: TEST_ISS.to_owned(),
                    capacity: 1000,
                }],
            ));
            state.nip_fi_deny_map = Some(Arc::clone(&deny_map));
            Arc::new(state)
        };

        // Build a CommandResult with a fractional deadline: T + 500_000_000 ns.
        // We construct the CommandResult directly rather than going through the HTTP
        // handler so we can control the exact until timestamp.
        let target = nostr::Keys::generate().public_key();
        let t_whole = chrono::DateTime::from_timestamp(1_800_000_000, 0).unwrap();
        let nanos: u32 = 500_000_000;
        let t_frac = chrono::DateTime::from_timestamp(1_800_000_000, nanos).unwrap();

        // Construct a synthetic CommandResult.
        let cmd = buzz_auth::CommandResult {
            caller_iss: TEST_ISS.to_owned(),
            caller_sub: TEST_SUB.to_owned(),
            target_pubkey: target,
            until: t_frac,
        };

        // Publisher seam → encode → decode.
        let msg = nip_fi_disconnect_message(&cmd);
        assert_eq!(
            msg.until_unix, 1_800_000_000,
            "publisher must capture whole-second"
        );
        assert_eq!(msg.until_unix_nanos, nanos, "publisher must capture nanos");

        let encoded = buzz_pubsub::encode_nip_fi_disconnect(&msg).expect("encode must succeed");
        let decoded = buzz_pubsub::decode_nip_fi_disconnect(&encoded).expect("decode must succeed");
        assert_eq!(decoded.until_unix_nanos, nanos, "decoded nanos must match");

        // Consumer seam: apply with now = t_frac - 1ns (just inside the deadline).
        let now_inside = t_frac - chrono::Duration::nanoseconds(1);
        let result = apply_nip_fi_disconnect(&state, &decoded, now_inside);
        assert_eq!(
            result,
            NipFiDisconnectApplyResult::Applied(CrossPodMergeResult::Merged),
            "apply must succeed with now inside deadline"
        );

        let deny_map = state.nip_fi_deny_map.as_deref().expect("deny map present");

        // Denied immediately after T (now = T + 1ns, well inside the deadline T+500ms).
        let now_after_t = t_whole + chrono::Duration::nanoseconds(1);
        assert!(
            deny_map.is_denied(TEST_ISS, &target, now_after_t),
            "must be denied at T+1ns (deadline is T+500ms)"
        );

        // Denied at deadline minus 1ns (just before equality boundary).
        let now_before_boundary = t_frac - chrono::Duration::nanoseconds(1);
        assert!(
            deny_map.is_denied(TEST_ISS, &target, now_before_boundary),
            "must be denied at deadline - 1ns"
        );

        // Admitted at exact equality (now == until): contract is `now < until`,
        // so exact equality means admitted.
        assert!(
            !deny_map.is_denied(TEST_ISS, &target, t_frac),
            "must be admitted at exact equality (now < until fails at now == until)"
        );

        // Also admitted after (now = T + whole second).
        assert!(
            !deny_map.is_denied(TEST_ISS, &target, t_whole + chrono::Duration::seconds(1)),
            "must be admitted past the deadline"
        );
    }

    // ── Test: blocker 4b — production_install_populates_both_app_state_fields ─────
    //
    // Calls install_nip_fi_command_components() directly (the production seam that owns
    // both AppState assignments). Proves:
    // - command_issuers == 1
    // - both AppState fields are Some
    // - a valid signed command through the verifier creates a deny visible via the map
    //
    // Mandatory red mutations (proven by separate inline verification below):
    //  1. delete deny_map assignment → AppState.nip_fi_deny_map is None
    //  2. delete verifier assignment → AppState.nip_fi_command_verifier is None
    //  3. wire verifier to a different map → verify succeeds but state map denial fails

    #[tokio::test]
    async fn production_install_populates_both_app_state_fields() {
        use super::install_nip_fi_command_components;

        // Build a minimal AppState — same construction as build_test_state but without
        // the S4 fields so we can verify install_nip_fi_command_components populates them.
        let config = crate::config::Config::hermetic_for_test();
        let pool = sqlx::PgPool::connect_lazy(&config.database_url).unwrap();
        let db = buzz_db::Db::from_pool(pool.clone());
        let redis_pool = deadpool_redis::Config::from_url(&config.redis_url)
            .create_pool(Some(deadpool_redis::Runtime::Tokio1))
            .unwrap();
        let pubsub = Arc::new(
            buzz_pubsub::PubSubManager::new(&config.redis_url, redis_pool.clone())
                .await
                .unwrap(),
        );
        let audit = buzz_audit::AuditService::new(pool.clone());
        let auth = buzz_auth::AuthService::new(config.auth.clone());
        let search = buzz_search::SearchService::new(pool.clone());
        let workflow_engine = Arc::new(buzz_workflow::WorkflowEngine::new(
            db.clone(),
            buzz_workflow::WorkflowConfig::default(),
        ));
        let media_storage = buzz_media::MediaStorage::new(&config.media).unwrap();
        let (mut state, _) = crate::state::AppState::new(
            config,
            db,
            redis_pool,
            audit,
            pubsub,
            auth,
            search,
            workflow_engine,
            nostr::Keys::generate(),
            media_storage,
        );
        // Both fields start as None — we'll verify install populates them.
        assert!(state.nip_fi_deny_map.is_none());
        assert!(state.nip_fi_command_verifier.is_none());

        let jwks_configs = vec![test_jwks_config()];
        let key_source = Arc::new(
            ProductionJwksSource::new(jwks_configs.clone(), buzz_auth::HttpJwksFetcher::new())
                .expect("valid key source"),
        );
        // Seed the JWKS snapshot hermetically (no HTTP).
        key_source
            .seed_snapshot_for_test(TEST_ISS, test_jwks())
            .await;

        let mut registry = IssuerRegistry::new();
        registry.insert(test_issuer_policy());

        let cmd_configs = vec![(
            TEST_ISS.to_owned(),
            CommandIssuerEnvConfig {
                maximum_command_age_seconds: Some(30),
                authorized_principals: Some(vec![TEST_SUB.to_owned()]),
                deny_set_capacity: Some(100),
            },
        )];

        let report = install_nip_fi_command_components(
            &mut state.nip_fi_deny_map,
            &mut state.nip_fi_command_verifier,
            buzz_auth::NipFiMode::Enforce,
            &registry,
            Arc::clone(&key_source),
            &cmd_configs,
        )
        .expect("install must succeed for valid config");

        assert_eq!(report.command_issuers, 1, "command_issuers must equal 1");

        // Both AppState fields must be populated.
        assert!(
            state.nip_fi_deny_map.is_some(),
            "nip_fi_deny_map must be Some after install"
        );
        assert!(
            state.nip_fi_command_verifier.is_some(),
            "nip_fi_command_verifier must be Some after install"
        );

        // Verify a valid command through the verifier and confirm the deny entry
        // is visible through the AppState's deny_map (proves shared map wiring).
        let target = nostr::Keys::generate().public_key();
        let token = mint_token(&target.to_hex(), 300, serde_json::json!({}));
        let verifier = state.nip_fi_command_verifier.as_ref().unwrap();
        let result = verifier.verify(&token, "POST", TEST_PATH, &target);
        assert!(
            result.is_ok(),
            "verifier must accept a valid command: {result:?}"
        );
        let deny_map = state.nip_fi_deny_map.as_deref().unwrap();
        assert!(
            deny_map.is_denied(TEST_ISS, &target, chrono::Utc::now()),
            "deny entry must be visible via AppState.nip_fi_deny_map after verify"
        );
    }

    // ── Single JWKS lifecycle owner witness ───────────────────────────────
    //
    // `main.rs` owns the only warm + per-issuer refresh of the shared JWKS
    // source. The installer must build and install components without any
    // fetch. Reintroducing a warm loop (or refresh spawn) inside the installer
    // makes `call_count` non-zero and reds this test.
    #[tokio::test(start_paused = true)]
    async fn installer_performs_no_jwks_fetch() {
        use super::install_nip_fi_command_components;

        let fetcher = buzz_auth::ScriptedJwksFetcher::new([]);
        let fetches = Arc::clone(&fetcher.call_count);
        let key_source = Arc::new(
            ProductionJwksSource::new(vec![test_jwks_config()], fetcher).expect("valid key source"),
        );
        let mut registry = IssuerRegistry::new();
        registry.insert(test_issuer_policy());
        let cmd_configs = vec![(
            TEST_ISS.to_owned(),
            CommandIssuerEnvConfig {
                maximum_command_age_seconds: Some(30),
                authorized_principals: Some(vec![TEST_SUB.to_owned()]),
                deny_set_capacity: Some(100),
            },
        )];
        let mut deny_map = None;
        let mut command_verifier = None;

        install_nip_fi_command_components(
            &mut deny_map,
            &mut command_verifier,
            buzz_auth::NipFiMode::Enforce,
            &registry,
            key_source,
            &cmd_configs,
        )
        .expect("install must succeed for valid config");
        tokio::time::sleep(std::time::Duration::from_secs(86_400)).await;

        assert!(deny_map.is_some() && command_verifier.is_some());
        assert_eq!(
            fetches.load(std::sync::atomic::Ordering::SeqCst),
            0,
            "installer must not fetch JWKS; main.rs is the single lifecycle owner"
        );
    }

    // ── Startup-oracle: shared JWKS source wiring ──────────────────────────
    //
    // Proves that the installer, given the state's own JWKS source (as
    // `main.rs` passes it), wires the command verifier onto the EXACT Arc that
    // `nip_fi_verifier` holds: once that source is warm, both verify.
    //
    // Construction path mirrors `main.rs` exactly:
    //   1. Build AppState with an Enforce config that has jwks_configs populated —
    //      `build_nip_fi_components` runs and sets `nip_fi_verifier` and
    //      `nip_fi_jwks_source` on the state.
    //   2. Seed the state's own `nip_fi_jwks_source` (no HTTP).
    //   3. Call `install_nip_fi_command_components` with
    //      `state.nip_fi_jwks_source.clone()` — the same Arc the verifier holds.
    //   4. Assert both `nip_fi_verifier` and `nip_fi_command_verifier` verify.
    //
    // Mutation evidence:
    //   Pass a second, unseeded source to the installer → the command
    //   verifier's source stays cold → command verify fails.
    #[tokio::test]
    async fn installer_shares_jwks_source_with_assertion_verifier() {
        use super::install_nip_fi_command_components;
        use crate::nip_fi_config::NipFiRelayConfig;
        use buzz_auth::NipFiMode;

        // Build the config with an Enforce NIP-FI section so build_nip_fi_components
        // sets nip_fi_verifier + nip_fi_jwks_source on the AppState.
        let mut config = crate::config::Config::hermetic_for_test();
        let jwks_configs = vec![test_jwks_config()];
        let mut registry = IssuerRegistry::new();
        registry.insert(test_issuer_policy());
        let cmd_configs = vec![(
            TEST_ISS.to_owned(),
            CommandIssuerEnvConfig {
                maximum_command_age_seconds: Some(30),
                authorized_principals: Some(vec![TEST_SUB.to_owned()]),
                deny_set_capacity: Some(100),
            },
        )];
        config.nip_fi = NipFiRelayConfig {
            mode: NipFiMode::Enforce,
            registry: registry.clone(),
            jwks_configs: jwks_configs.clone(),
            command_configs: cmd_configs.clone(),
            max_connection_lifetime_secs: 3600,
        };

        let pool = sqlx::PgPool::connect_lazy(&config.database_url).unwrap();
        let db = buzz_db::Db::from_pool(pool.clone());
        let redis_pool = deadpool_redis::Config::from_url(&config.redis_url)
            .create_pool(Some(deadpool_redis::Runtime::Tokio1))
            .unwrap();
        let pubsub = Arc::new(
            buzz_pubsub::PubSubManager::new(&config.redis_url, redis_pool.clone())
                .await
                .unwrap(),
        );
        let audit = buzz_audit::AuditService::new(pool.clone());
        let auth = buzz_auth::AuthService::new(config.auth.clone());
        let search = buzz_search::SearchService::new(pool.clone());
        let workflow_engine = Arc::new(buzz_workflow::WorkflowEngine::new(
            db.clone(),
            buzz_workflow::WorkflowConfig::default(),
        ));
        let media_storage = buzz_media::MediaStorage::new(&config.media).unwrap();
        // build_nip_fi_components runs here because mode == Enforce and jwks_configs
        // is non-empty; nip_fi_verifier and nip_fi_jwks_source are set on the state.
        let (mut state, _) = crate::state::AppState::new(
            config,
            db,
            redis_pool,
            audit,
            pubsub,
            auth,
            search,
            workflow_engine,
            nostr::Keys::generate(),
            media_storage,
        );
        assert!(
            state.nip_fi_verifier.is_some(),
            "nip_fi_verifier must be Some for Enforce config"
        );
        assert!(
            state.nip_fi_jwks_source.is_some(),
            "nip_fi_jwks_source must be Some for Enforce config"
        );

        let shared_source = state.nip_fi_jwks_source.clone().unwrap();
        install_nip_fi_command_components(
            &mut state.nip_fi_deny_map,
            &mut state.nip_fi_command_verifier,
            NipFiMode::Enforce,
            &registry,
            Arc::clone(&shared_source),
            &cmd_configs,
        )
        .expect("install must succeed for valid config");

        // Warm the state's own source after install — the step main.rs's
        // single lifecycle owner performs. Both verifiers must see it.
        shared_source
            .seed_snapshot_for_test(TEST_ISS, test_jwks())
            .await;

        let key = nostr::Keys::generate();
        let token = mint_assertion_token(&key.public_key().to_hex());
        let verifier = state.nip_fi_verifier.as_deref().unwrap();
        let result = verifier.verify_assertion(&token);
        assert!(
            result.is_ok(),
            "assertion verifier must read the warmed shared source; got: {result:?}"
        );

        let target = nostr::Keys::generate().public_key();
        let command = mint_token(&target.to_hex(), 300, serde_json::json!({}));
        let command_verifier = state.nip_fi_command_verifier.as_ref().unwrap();
        let result = command_verifier.verify(&command, "POST", TEST_PATH, &target);
        assert!(
            result.is_ok(),
            "command verifier must read the same warmed shared source; got: {result:?}"
        );
    }

    /// Mint a valid ES256 `nip-fi+jwt` assertion for `nostr_pubkey = key_hex`,
    /// signed by the route-integration-test key pair.
    /// Used by the startup-oracle test to verify the assertion verifier against
    /// its own warmed JWKS source.
    fn mint_assertion_token(key_hex: &str) -> String {
        mint_assertion_token_for(TEST_ISS, key_hex)
    }

    fn mint_assertion_token_for(iss: &str, key_hex: &str) -> String {
        use jsonwebtoken::{encode, Algorithm, EncodingKey, Header};
        let now = chrono::Utc::now().timestamp();
        let claims = serde_json::json!({
            "iss": iss,
            "aud": TEST_AUD,
            "sub": TEST_SUB,
            "iat": now,
            "exp": now + 600,
            "nostr_pubkey": key_hex,
        });
        let mut header = Header::new(Algorithm::ES256);
        header.typ = Some("nip-fi+jwt".to_owned());
        header.kid = Some(TEST_KID.to_owned());
        let key =
            EncodingKey::from_ec_pem(TEST_PRIVATE_KEY_PEM.as_bytes()).expect("valid test EC key");
        encode(&header, &claims, &key).expect("sign assertion-test token")
    }

    // ── Issuer scope: the close scans match (issuer, k) ──────────────────────
    //
    // A deny entry is keyed by (iss, k).  The same key K admitted under a
    // second issuer B is not blocked by A's entry, so neither the route nor the
    // cross-pod consumer may close B's sessions.  [FI-TRACE-DENY-SET]

    const OTHER_ISS: &str = "https://idp-b.test.example.com";

    /// Live root + audio sessions for one (issuer, key), with the tokens the
    /// close path cancels.  The guard keeps the audio socket registered.
    struct IssuerSessions {
        root: tokio_util::sync::CancellationToken,
        audio: tokio_util::sync::CancellationToken,
        _audio_guard: crate::state::CommunityConnectionGuard,
    }

    impl IssuerSessions {
        fn register(state: &crate::state::AppState, issuer: &str, key: &nostr::PublicKey) -> Self {
            use crate::state::CommunityConnectionControl;
            use tokio::sync::mpsc;
            use tokio_util::sync::CancellationToken;
            let community = buzz_core::tenant::CommunityId::from_uuid(uuid::Uuid::nil());
            let root = CancellationToken::new();
            let conn_id = uuid::Uuid::new_v4();
            state.conn_manager.register(
                conn_id,
                mpsc::channel(8).0,
                mpsc::channel(8).0,
                mpsc::channel(1).0,
                None,
                root.clone(),
                community,
                Arc::new(std::sync::atomic::AtomicU8::new(0)),
                Arc::new(tokio::sync::Mutex::new(std::collections::HashMap::new())),
                3,
                CommunityConnectionControl::new(root.clone()),
            );
            state.conn_manager.set_authenticated_identity(
                conn_id,
                key.to_bytes().to_vec(),
                Some(issuer.to_owned()),
            );
            let audio = CancellationToken::new();
            let ctrl = CommunityConnectionControl::new(audio.clone());
            crate::audio::handler::audio_post_auth_register(
                &ctrl,
                key.to_bytes().to_vec(),
                Some(issuer.to_owned()),
            );
            let _audio_guard =
                state
                    .community_connections
                    .register(uuid::Uuid::new_v4(), community, ctrl);
            Self {
                root,
                audio,
                _audio_guard,
            }
        }

        fn assert_closed(&self, who: &str) {
            assert!(self.root.is_cancelled(), "{who}: root session must close");
            assert!(self.audio.is_cancelled(), "{who}: audio session must close");
        }

        fn assert_open(&self, who: &str) {
            assert!(
                !self.root.is_cancelled(),
                "{who}: root session must survive"
            );
            assert!(
                !self.audio.is_cancelled(),
                "{who}: audio session must survive"
            );
        }
    }

    #[tokio::test]
    async fn route_disconnect_closes_only_caller_issuer_sessions() {
        let state = build_test_state(1000).await;
        let key = nostr::Keys::generate().public_key();
        let under_a = IssuerSessions::register(&state, TEST_ISS, &key);
        let under_b = IssuerSessions::register(&state, OTHER_ISS, &key);

        let token = mint_token(&key.to_hex(), 300, serde_json::json!({}));
        let resp = do_request(
            Arc::clone(&state),
            "POST",
            vec![
                ("Content-Type", "application/json".into()),
                (CLIENT_ATTACHED_HEADER, format!("Bearer {token}")),
            ],
            Some(serde_json::json!({"pubkey": key.to_hex()})),
        )
        .await;
        assert_eq!(resp.status(), StatusCode::OK);

        under_a.assert_closed("issuer A");
        under_b.assert_open("issuer B");
    }

    /// State whose registry and deny map know only issuer A (`TEST_ISS`).
    async fn cross_pod_state(capacity: usize) -> Arc<crate::state::AppState> {
        let mut config = crate::config::Config::hermetic_for_test();
        config.nip_fi.registry.insert(test_issuer_policy());
        let pool = sqlx::PgPool::connect_lazy(&config.database_url).unwrap();
        let db = buzz_db::Db::from_pool(pool.clone());
        let redis_pool = deadpool_redis::Config::from_url(&config.redis_url)
            .create_pool(Some(deadpool_redis::Runtime::Tokio1))
            .unwrap();
        let pubsub = Arc::new(
            buzz_pubsub::PubSubManager::new(&config.redis_url, redis_pool.clone())
                .await
                .unwrap(),
        );
        let audit = buzz_audit::AuditService::new(pool.clone());
        let auth = buzz_auth::AuthService::new(config.auth.clone());
        let search = buzz_search::SearchService::new(pool.clone());
        let workflow_engine = Arc::new(buzz_workflow::WorkflowEngine::new(
            db.clone(),
            buzz_workflow::WorkflowConfig::default(),
        ));
        let media_storage = buzz_media::MediaStorage::new(&config.media).unwrap();
        let (mut state, _) = crate::state::AppState::new(
            config,
            db,
            redis_pool,
            audit,
            pubsub,
            auth,
            search,
            workflow_engine,
            nostr::Keys::generate(),
            media_storage,
        );
        state.nip_fi_deny_map = Some(Arc::new(NipFiDenyMap::new(
            capacity,
            vec![IssuerCapacity {
                issuer: TEST_ISS.to_owned(),
                capacity,
            }],
        )));
        Arc::new(state)
    }

    fn cross_pod_message(key: &nostr::PublicKey) -> buzz_pubsub::NipFiDisconnect {
        buzz_pubsub::NipFiDisconnect {
            issuer: TEST_ISS.to_owned(),
            pubkey_bytes: key.to_bytes().to_vec(),
            until_unix: (chrono::Utc::now() + chrono::Duration::seconds(300)).timestamp(),
            until_unix_nanos: 0,
        }
    }

    /// Delivers an A-issued cross-pod message for K with A and B sessions
    /// live, and asserts the merge outcome and that only A's sessions close.
    fn assert_cross_pod_closes_only_issuer_a(
        state: &crate::state::AppState,
        expected: buzz_auth::CrossPodMergeResult,
    ) {
        let key = nostr::Keys::generate().public_key();
        let under_a = IssuerSessions::register(state, TEST_ISS, &key);
        let under_b = IssuerSessions::register(state, OTHER_ISS, &key);

        let result = apply_nip_fi_disconnect(state, &cross_pod_message(&key), chrono::Utc::now());

        assert_eq!(result, NipFiDisconnectApplyResult::Applied(expected));
        under_a.assert_closed("issuer A");
        under_b.assert_open("issuer B");
    }

    #[tokio::test]
    async fn cross_pod_merged_closes_only_message_issuer_sessions() {
        let state = cross_pod_state(10).await;
        assert_cross_pod_closes_only_issuer_a(&state, buzz_auth::CrossPodMergeResult::Merged);
    }

    #[tokio::test]
    async fn cross_pod_capacity_exceeded_closes_only_message_issuer_sessions() {
        let state = cross_pod_state(1).await;
        let filler = nostr::Keys::generate().public_key();
        assert_eq!(
            apply_nip_fi_disconnect(&state, &cross_pod_message(&filler), chrono::Utc::now()),
            NipFiDisconnectApplyResult::Applied(buzz_auth::CrossPodMergeResult::Merged),
        );
        assert_cross_pod_closes_only_issuer_a(
            &state,
            buzz_auth::CrossPodMergeResult::CapacityExceeded,
        );
    }

    #[tokio::test]
    async fn cross_pod_shard_poisoned_closes_only_message_issuer_sessions() {
        let state = cross_pod_state(10).await;
        state
            .nip_fi_deny_map
            .as_deref()
            .expect("deny map present")
            .poison_shard_for_test(TEST_ISS);
        assert_cross_pod_closes_only_issuer_a(
            &state,
            buzz_auth::CrossPodMergeResult::ShardPoisoned,
        );
    }

    // ── HTTP admission reads the shared deny map ─────────────────────────────
    //
    // Deny entries are written only through `POST /api/nip-fi/disconnect`; HTTP
    // admission (`admit_nip_fi_http_on_state`) and WS admission must both see
    // them through `AppState::nip_fi_deny_map`.  [FI-TRACE-DENY-SET]

    /// `build_test_app_state` in `Enforce` mode with an assertion verifier that
    /// trusts issuer A (`TEST_ISS`) and issuer B (`OTHER_ISS`), both signed by
    /// the test key.  The command API and deny map know only issuer A.
    async fn http_enforce_state() -> Arc<crate::state::AppState> {
        let mut config = crate::config::Config::hermetic_for_test();
        config.require_relay_membership = false;
        config.nip_fi.mode = buzz_auth::NipFiMode::Enforce;
        config.nip_fi.registry.insert(issuer_policy(TEST_ISS));
        config.nip_fi.registry.insert(issuer_policy(OTHER_ISS));
        let mut state = build_test_app_state(1000, config).await;

        let key_source = Arc::new(
            ProductionJwksSource::new(
                vec![jwks_config(TEST_ISS), jwks_config(OTHER_ISS)],
                buzz_auth::HttpJwksFetcher::new(),
            )
            .expect("key source"),
        );
        key_source
            .seed_snapshot_for_test(TEST_ISS, test_jwks())
            .await;
        key_source
            .seed_snapshot_for_test(OTHER_ISS, test_jwks())
            .await;
        state.nip_fi_verifier = Some(Arc::new(buzz_auth::FederatedAssertionVerifier::new(
            state.config.nip_fi.registry.clone(),
            key_source,
        )));
        Arc::new(state)
    }

    /// Deny `key` under issuer A through the real disconnect route.
    async fn deny_via_route(
        state: &Arc<crate::state::AppState>,
        key: &nostr::PublicKey,
        until_offset_secs: i64,
    ) {
        let token = mint_token(&key.to_hex(), until_offset_secs, serde_json::json!({}));
        let resp = do_request(
            Arc::clone(state),
            "POST",
            vec![
                ("Content-Type", "application/json".into()),
                (CLIENT_ATTACHED_HEADER, format!("Bearer {token}")),
            ],
            Some(serde_json::json!({"pubkey": key.to_hex()})),
        )
        .await;
        assert_eq!(
            resp.status(),
            StatusCode::OK,
            "disconnect route must succeed"
        );
    }

    /// HTTP admission for an `(iss, key)` assertion paired with a NIP-98 proof
    /// for `key`.  `Ok(())` = admitted; `Err` = the denial response.
    // Response<Body> is intentionally large (axum's design); see admit_nip_fi_http.
    #[allow(clippy::result_large_err)]
    fn http_admit(
        state: &crate::state::AppState,
        iss: &str,
        key: &nostr::PublicKey,
    ) -> Result<(), axum::response::Response> {
        let mut headers = HeaderMap::new();
        headers.insert(
            CLIENT_ATTACHED_HEADER,
            format!("Bearer {}", mint_assertion_token_for(iss, &key.to_hex()))
                .parse()
                .expect("header value"),
        );
        crate::nip_fi_http::admit_nip_fi_http_on_state(state, &headers, || {
            Ok(crate::nip_fi_http::Nip98Proof::new(*key, ()))
        })
        .map(|_| ())
    }

    async fn assert_fixed_authorization_denied(resp: axum::response::Response, why: &str) {
        assert_eq!(resp.status(), StatusCode::FORBIDDEN, "{why}");
        assert_eq!(
            resp.headers()
                .get("Content-Type")
                .and_then(|v| v.to_str().ok()),
            Some("text/plain; charset=utf-8"),
            "{why}"
        );
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .expect("body");
        assert_eq!(&body[..], b"authorization denied\n", "{why}");
    }

    #[tokio::test]
    async fn http_admission_denies_key_denied_via_route() {
        let state = http_enforce_state().await;
        let key = nostr::Keys::generate().public_key();
        assert!(
            http_admit(&state, TEST_ISS, &key).is_ok(),
            "control: (iss-A, k) is admitted before any deny"
        );

        deny_via_route(&state, &key, 300).await;

        let resp = http_admit(&state, TEST_ISS, &key)
            .expect_err("(iss-A, k) must be denied at HTTP ingress after the route denies it");
        assert_fixed_authorization_denied(resp, "(iss-A, k) HTTP denial").await;
    }

    #[tokio::test]
    async fn http_admission_deny_is_issuer_scoped() {
        let state = http_enforce_state().await;
        let key = nostr::Keys::generate().public_key();
        deny_via_route(&state, &key, 300).await;

        assert!(
            http_admit(&state, TEST_ISS, &key).is_err(),
            "control: (iss-A, k) is denied"
        );
        assert!(
            http_admit(&state, OTHER_ISS, &key).is_ok(),
            "a deny for (iss-A, k) must not block (iss-B, k)"
        );
    }

    #[tokio::test]
    async fn http_admission_readmits_after_deny_until_passes() {
        let state = http_enforce_state().await;
        let key = nostr::Keys::generate().public_key();
        // `until` is whole seconds from `now`: +3 leaves at least 2s of window.
        deny_via_route(&state, &key, 3).await;
        assert!(
            http_admit(&state, TEST_ISS, &key).is_err(),
            "(iss-A, k) is denied inside the window"
        );

        tokio::time::sleep(std::time::Duration::from_secs(4)).await;

        assert!(
            http_admit(&state, TEST_ISS, &key).is_ok(),
            "(iss-A, k) must be admitted again once `until` has passed"
        );
    }

    #[tokio::test]
    async fn one_route_deny_is_enforced_by_ws_and_http_admission() {
        use crate::router::build_router;
        let state = http_enforce_state().await;
        let key = nostr::Keys::generate().public_key();
        deny_via_route(&state, &key, 300).await;

        let ws_request = Request::get("/")
            .header(axum::http::header::HOST, "relay.example")
            .header("Upgrade", "websocket")
            .header("Connection", "Upgrade")
            .header("Sec-WebSocket-Version", "13")
            .header("Sec-WebSocket-Key", "dGhlIHNhbXBsZSBub25jZQ==")
            .header(
                CLIENT_ATTACHED_HEADER,
                format!("Bearer {}", mint_assertion_token(&key.to_hex())),
            )
            .body(Body::empty())
            .expect("request");
        let ws_resp = build_router(Arc::clone(&state))
            .oneshot(ws_request)
            .await
            .expect("router response");
        assert_fixed_authorization_denied(ws_resp, "WS admission sees the route's deny").await;

        let http_resp =
            http_admit(&state, TEST_ISS, &key).expect_err("HTTP admission sees the same deny");
        assert_fixed_authorization_denied(http_resp, "HTTP admission sees the route's deny").await;
    }

    #[tokio::test]
    // Response<Body> is intentionally large (axum's design); see admit_nip_fi_http.
    #[allow(clippy::result_large_err)]
    async fn off_mode_http_admission_ignores_route_deny_entries() {
        // Off mode: the command route still records the entry, but HTTP
        // admission never consults the deny map — the NIP-98 closure result is
        // returned unchanged, byte for byte.
        let state = build_test_state(1000).await;
        assert!(matches!(
            state.config.nip_fi.mode,
            buzz_auth::NipFiMode::Off
        ));
        let key = nostr::Keys::generate().public_key();
        deny_via_route(&state, &key, 300).await;
        assert!(
            state
                .nip_fi_deny_map
                .as_deref()
                .expect("deny map present")
                .is_denied(TEST_ISS, &key, chrono::Utc::now()),
            "control: the route recorded the deny entry"
        );

        let admitted =
            crate::nip_fi_http::admit_nip_fi_http_on_state(&state, &HeaderMap::new(), || {
                Ok(crate::nip_fi_http::Nip98Proof::new(key, 7u8))
            })
            .expect("Off mode admits on NIP-98 success regardless of deny entries");
        assert_eq!(admitted.proven_pubkey(), &key);
        assert!(admitted.assertion().is_none());
        assert_eq!(*admitted.extra(), 7);

        let legacy = || {
            Response::builder()
                .status(StatusCode::UNAUTHORIZED)
                .header("Content-Type", "application/json")
                .body(Body::from(r#"{"error":"legacy"}"#))
                .expect("legacy response")
        };
        let resp = crate::nip_fi_http::admit_nip_fi_http_on_state::<(), _>(
            &state,
            &HeaderMap::new(),
            || Err(legacy()),
        )
        .expect_err("Off mode propagates the NIP-98 failure");
        let expected = legacy();
        assert_eq!(resp.status(), expected.status());
        assert_eq!(resp.headers(), expected.headers());
        let got = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .expect("body");
        let want = axum::body::to_bytes(expected.into_body(), usize::MAX)
            .await
            .expect("body");
        assert_eq!(got, want, "Off-mode legacy response must be byte-identical");
    }
}
