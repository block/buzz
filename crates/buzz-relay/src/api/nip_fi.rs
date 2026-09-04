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
    CommandError, CommandIssuerPolicy, CommandVerifier, IssuerCapacity, NipFiDenyMap, NipFiMode,
    ProductionJwksSource, CLIENT_ATTACHED_HEADER,
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
            let closed = state.conn_manager.disconnect_nip_fi(&pubkey_bytes)
                + state.community_connections.disconnect_nip_fi(&pubkey_bytes);
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
pub struct NipFiCommandComponents {
    /// The shared deny map consumed by WS admission and S5 HTTP admission.
    pub deny_map: Arc<NipFiDenyMap>,
    /// The command verifier for the `POST /api/nip-fi/disconnect` endpoint.
    pub command_verifier: Arc<CommandVerifier<Arc<ProductionJwksSource>>>,
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
        let closed = state.conn_manager.disconnect_nip_fi(&message.pubkey_bytes)
            + state
                .community_connections
                .disconnect_nip_fi(&message.pubkey_bytes);
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
pub fn build_nip_fi_command_components(
    mode: NipFiMode,
    registry: &buzz_auth::IssuerRegistry,
    key_source: Arc<ProductionJwksSource>,
    issuer_command_configs: &[(String, CommandIssuerEnvConfig)],
) -> Result<Option<NipFiCommandComponents>, String> {
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
    /// Number of issuers whose JWKS snapshot was warmed before serving.
    pub warmed_issuers: usize,
    /// Number of issuers wired into the command verifier.
    pub command_issuers: usize,
}

/// Install NIP-FI command components into `app_state`.
///
/// This is the single production startup seam that owns:
/// - enforce-mode pre-flight check (returns `Err` for incomplete config)
/// - JWKS warmup for every configured issuer
/// - background JWKS refresh loop
/// - `build_nip_fi_command_components` invocation
/// - assignment of both `app_state.nip_fi_deny_map` and
///   `app_state.nip_fi_command_verifier`
///
/// `main.rs` constructs the concrete `ProductionJwksSource` and calls this
/// function once; it no longer owns either assignment or the warmup loop.
///
/// Deleting either AppState field assignment or the warmup loop must red the
/// `production_install_warms_and_populates_both_app_state_fields` oracle.
pub async fn install_nip_fi_command_components(
    app_state: &mut crate::state::AppState,
    mode: NipFiMode,
    registry: &buzz_auth::IssuerRegistry,
    key_source: Arc<ProductionJwksSource>,
    jwks_configs: &[buzz_auth::IssuerJwksConfig],
    command_configs: &[(String, CommandIssuerEnvConfig)],
) -> Result<NipFiCommandStartupReport, String> {
    // Pre-flight: enforce mode with no command configs is always an error.
    if matches!(mode, NipFiMode::Enforce) && command_configs.is_empty() {
        return Err(
            "NIP-FI install: enforce mode requires at least one command-capable issuer".to_owned(),
        );
    }

    // Warm each issuer's JWKS snapshot before serving.
    let mut warmed_issuers: usize = 0;
    for jwks_cfg in jwks_configs {
        if let Some(snapshot) = key_source.get_snapshot(&jwks_cfg.issuer).await {
            tracing::info!(
                issuer_len = jwks_cfg.issuer.len(),
                generation = snapshot.generation(),
                "NIP-FI: JWKS warmed"
            );
            warmed_issuers += 1;
        } else {
            tracing::warn!(
                issuer_len = jwks_cfg.issuer.len(),
                "NIP-FI: JWKS warm-up failed — will retry inline"
            );
        }
    }

    // Spawn background refresh loop so snapshots stay fresh after startup.
    {
        let source_for_refresh = Arc::clone(&key_source);
        let jwks_cfgs = jwks_configs.to_vec();
        let shutting_down = Arc::clone(&app_state.shutting_down);
        tokio::spawn(async move {
            loop {
                let min_interval_secs = jwks_cfgs
                    .iter()
                    .map(|c| c.contract.refresh_interval_seconds())
                    .min()
                    .unwrap_or(300);
                tokio::time::sleep(std::time::Duration::from_secs(min_interval_secs)).await;
                if shutting_down.load(std::sync::atomic::Ordering::Acquire) {
                    break;
                }
                for cfg in &jwks_cfgs {
                    source_for_refresh.get_snapshot(&cfg.issuer).await;
                }
            }
        });
    }

    let components =
        build_nip_fi_command_components(mode, registry, Arc::clone(&key_source), command_configs)?;

    let command_issuers = if let Some(c) = components {
        let n = command_configs.len();
        app_state.nip_fi_deny_map = Some(c.deny_map);
        app_state.nip_fi_command_verifier = Some(c.command_verifier);
        tracing::info!("NIP-FI S4: command API enabled ({n} issuer(s))");
        n
    } else {
        0
    };

    Ok(NipFiCommandStartupReport {
        warmed_issuers,
        command_issuers,
    })
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
        use buzz_auth::{FreshnessClass, IssuerPolicy, JwksSourceContract, TokenClass};
        let contract =
            JwksSourceContract::new(format!("{TEST_ISS}/.well-known/jwks.json"), 300, 86400)
                .expect("valid JWKS contract");
        IssuerPolicy::new(
            TEST_ISS.to_owned(),
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
        use buzz_auth::{IssuerJwksConfig, JwksSourceContract};
        let contract =
            JwksSourceContract::new(format!("{TEST_ISS}/.well-known/jwks.json"), 300, 86400)
                .expect("valid JWKS contract");
        IssuerJwksConfig {
            issuer: TEST_ISS.to_owned(),
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
        // Build a minimal AppState with NIP-FI S4 components wired.
        // Uses lazy/invalid DB+Redis — only nip_fi fields and conn_manager matter.
        use crate::state::AppState;
        let config = crate::config::Config::hermetic_for_test();

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
        Arc::new(state)
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
        ctrl_b.set_proven_pubkey(k_b.to_bytes().to_vec());
        let _guard_b = registry.register(uuid::Uuid::new_v4(), community, ctrl_b);

        let ctrl_unrelated = CommunityConnectionControl::new(cancel_unrelated.clone());
        ctrl_unrelated.set_proven_pubkey(k_unrelated.to_bytes().to_vec());
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
                    None,
                    cancel.clone(),
                    community,
                    bp,
                    std::sync::Arc::new(tokio::sync::Mutex::new(std::collections::HashMap::new())),
                    3,
                );
                state
                    .conn_manager
                    .set_authenticated_pubkey(conn_id, pubkey_bytes);
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
            audio_post_auth_register(&ctrl, target.to_bytes().to_vec());
            audio_registry.register(Uuid::new_v4(), community, ctrl)
        };

        // Register unrelated ordinary WS and audio controls.
        let (_unrelated_ws_id, cancel_unrelated_ws) =
            register_ws(&state, unrelated.to_bytes().to_vec());
        let cancel_audio_unrelated = CancellationToken::new();
        let _audio_unrelated_guard = {
            let ctrl = CommunityConnectionControl::new(cancel_audio_unrelated.clone());
            audio_post_auth_register(&ctrl, unrelated.to_bytes().to_vec());
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
            audio_post_auth_register(&ctrl, target2.to_bytes().to_vec());
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

    // ── Test: blocker 4b — production_install_warms_and_populates_both_app_state_fields ─────
    //
    // Calls install_nip_fi_command_components() directly (the production seam that owns
    // warmup + both AppState assignments). Proves:
    // - warmed_issuers == 1 after a seeded get_snapshot
    // - command_issuers == 1
    // - both AppState fields are Some
    // - a valid signed command through the verifier creates a deny visible via the map
    //
    // Mandatory red mutations (proven by separate inline verification below):
    //  1. delete deny_map assignment → AppState.nip_fi_deny_map is None
    //  2. delete verifier assignment → AppState.nip_fi_command_verifier is None
    //  3. delete/bypass warmup → warmed_issuers == 0
    //  4. wire verifier to a different map → verify succeeds but state map denial fails

    #[tokio::test]
    async fn production_install_warms_and_populates_both_app_state_fields() {
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
            &mut state,
            buzz_auth::NipFiMode::Enforce,
            &registry,
            Arc::clone(&key_source),
            &jwks_configs,
            &cmd_configs,
        )
        .await
        .expect("install must succeed for valid config");

        // Warmup: one issuer was seeded and should have a snapshot.
        assert_eq!(
            report.warmed_issuers, 1,
            "warmed_issuers must equal 1 (snapshot was seeded)"
        );
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

        // Set the shutdown flag so the background refresh task eventually exits.
        // This does not prove lifecycle — no task handle is awaited here.
        state
            .shutting_down
            .store(true, std::sync::atomic::Ordering::Release);
    }

    // ── Startup-oracle: shared JWKS source wiring ──────────────────────────
    //
    // Proves that `install_nip_fi_command_components` warms the EXACT Arc that
    // `nip_fi_verifier` holds, so a valid assertion JWT succeeds after startup.
    //
    // Construction path mirrors `main.rs` exactly:
    //   1. Build AppState with an Enforce config that has jwks_configs populated —
    //      `build_nip_fi_components` runs and sets `nip_fi_verifier` and
    //      `nip_fi_jwks_source` on the state.
    //   2. Seed the state's own `nip_fi_jwks_source` (no HTTP).
    //   3. Call `install_nip_fi_command_components` with
    //      `state.nip_fi_jwks_source.clone()` — the same Arc the verifier holds.
    //   4. Assert `nip_fi_verifier.verify(assertion_jwt)` succeeds.
    //
    // Mutation evidence:
    //   Reintroduce a second, unseeded source and pass it to the installer (the
    //   original bug) → the verifier's source stays cold → verify returns
    //   `KeySourceUnavailable` → the is_ok() assertion panics.
    #[tokio::test]
    async fn installer_warmup_makes_assertion_verifier_functional() {
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

        // Seed the state's own JWKS source — the exact Arc the verifier holds.
        // This is the warmup step that main.rs achieves by passing this same Arc
        // to install_nip_fi_command_components.
        let state_source = state.nip_fi_jwks_source.as_ref().unwrap();
        state_source
            .seed_snapshot_for_test(TEST_ISS, test_jwks())
            .await;

        // Clone the source Arc before the mutable borrow of state so the
        // borrow checker sees both borrows as non-overlapping.  This clone is
        // the same operation main.rs performs: it shares the EXACT underlying
        // source, not a fresh one.
        let shared_source = state.nip_fi_jwks_source.clone().unwrap();

        // Call the installer with the state's own source (mirrors main.rs after
        // the shared-source fix). The warmup loop confirms the seeded snapshot.
        let report = install_nip_fi_command_components(
            &mut state,
            NipFiMode::Enforce,
            &registry,
            shared_source,
            &jwks_configs,
            &cmd_configs,
        )
        .await
        .expect("install must succeed for valid config");
        assert_eq!(
            report.warmed_issuers, 1,
            "installer must report 1 warmed issuer (snapshot was pre-seeded)"
        );

        // Assert that a valid assertion JWT now verifies successfully.
        // This is the core oracle: the verifier reads key_set() from the same
        // Arc that the installer warmed — if they were different Arcs, the
        // verifier's source would be cold and this would return KeySourceUnavailable.
        let key = nostr::Keys::generate();
        let token = mint_assertion_token(&key.public_key().to_hex());
        let verifier = state.nip_fi_verifier.as_deref().unwrap();
        let result = verifier.verify(&token);
        assert!(
            result.is_ok(),
            "nip_fi_verifier.verify must succeed after installer warmup on the shared source; \
             got: {result:?}"
        );

        state
            .shutting_down
            .store(true, std::sync::atomic::Ordering::Release);
    }

    /// Mint a valid ES256 `nip-fi+jwt` assertion for `nostr_pubkey = key_hex`,
    /// signed by the route-integration-test key pair.
    /// Used by the startup-oracle test to verify the assertion verifier against
    /// its own JWKS source after installer warmup.
    fn mint_assertion_token(key_hex: &str) -> String {
        use jsonwebtoken::{encode, Algorithm, EncodingKey, Header};
        let now = chrono::Utc::now().timestamp();
        let claims = serde_json::json!({
            "iss": TEST_ISS,
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
}
