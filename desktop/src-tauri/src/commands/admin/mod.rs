//! Desktop in-app admin surface — NIP-98 client for `/api/admin/v1`.
//!
//! Implements five Tauri commands that fetch JSON and binary content from the
//! relay's deployment-admin API using the app keypair as the NIP-98 signing
//! identity. A sixth command, `admin_probe`, discovers which authentication
//! mode the configured admin origin is running and whether the app identity
//! is authorized.
//!
//! # Security model
//!
//! The webview never supplies paths, methods, or full URLs. Every IPC command
//! accepts an `AdminOrigin` (scheme + host + optional port, validated on
//! construction) and typed query parameters; the final URL is built natively
//! from a closed route enum. The URL that is signed is byte-identical to the
//! URL that is fetched.
//!
//! A dedicated no-redirect reqwest client prevents redirect-hop SSRF — a relay
//! 3xx is returned verbatim and treated as an error so the NIP-98 header is
//! never forwarded across origins.
//!
//! Keys are acquired via `AppState::signing_keys()`, which returns `Err` when
//! the identity is in recovery mode (keyring locked or lost), ensuring the app
//! keypair can never sign admin events under an inaccessible identity.
//!
//! Response sizes are bounded by Content-Length preflight and a streaming byte
//! counter, mirroring the `media_download.rs` pattern.

pub mod client;
pub(super) mod helpers;
pub(crate) mod origin;
pub(crate) mod routes;

// ── Response size caps ────────────────────────────────────────────────────

/// Success-JSON cap: reports list returns up to 200 rows, each note field
/// can reach the 256 KiB event-content cap. Sized for the worst case.
const SUCCESS_JSON_CAP: u64 = 52_428_800; // 50 MiB

/// Probe-response cap: `/probe` returns a tiny fixed-shape JSON envelope.
/// 8 KiB is far more than the payload needs while bounding a hostile body.
const PROBE_JSON_CAP: u64 = 8_192; // 8 KiB

/// Error-body cap: relay error responses are brief JSON envelopes.
const ERROR_BODY_CAP: u64 = 65_536; // 64 KiB

/// Attachment preview cap. 10 MiB is generous for images and small documents
/// while protecting against accidental OOM.
const ATTACHMENT_CAP: u64 = 10_485_760; // 10 MiB

// Re-export helpers into this module's namespace.
use helpers::{
    delete_admin_json, fetch_admin_json, finish_attachment_response, patch_admin_json,
    post_admin_json, put_admin_json,
};

// ── Typed probe result ────────────────────────────────────────────────────

/// Result of an `admin_probe` call. Each variant maps to a distinct UI state.
/// Tauri serialises this as `{ "state": "<camelCaseVariant>", ... }`.
#[derive(Debug, serde::Serialize)]
#[serde(tag = "state", rename_all = "camelCase")]
pub enum AdminProbeResult {
    /// NIP-98 mode is active and the current app keypair is on the allowlist.
    /// Includes the principal's `role` and `source` for the staffing tab.
    Nip98Authorized {
        /// The resolved role: `"operator"` or `"moderator"`.
        role: Option<String>,
        /// How the role was resolved: `"config"`, `"owner_fallback"`, or `"db"`.
        source: Option<String>,
    },
    /// NIP-98 mode is active but the app keypair was rejected after a signed
    /// attempt. Likely: pubkey not in `RELAY_OPERATOR_PUBKEYS`, clock skew, or
    /// relay config mismatch.
    Nip98Denied,
    /// Bearer-token mode (`BUZZ_ADMIN_AUTH=token`). The desktop cannot mint a
    /// bearer token; the operator must use the web console.
    TokenMode,
    /// Auth is disabled (`BUZZ_ADMIN_AUTH=disabled`). No credential needed.
    Disabled,
    /// The origin is reachable but the `/api/admin/v1` prefix is absent or
    /// returns a non-admin response.
    NotAdminApi,
    /// Network/TLS error, DNS failure, or Cloudflare Access interception.
    NetworkOrIntercepted,
}

// ── Typed query struct ────────────────────────────────────────────────────

/// Query parameters accepted by `admin_list_reports`.
#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AdminReportsQuery {
    pub community_id: Option<String>,
    pub status: Option<String>,
    pub report_type: Option<String>,
    pub target_kind: Option<String>,
    pub after: Option<String>,
    pub before: Option<String>,
    pub limit: Option<i64>,
}

// ── Probe ─────────────────────────────────────────────────────────────────

/// A boxed signing closure: given a URL, returns a `Nostr <token>` Authorization header.
type SignFn = Box<dyn Fn(&str) -> Result<String, String> + Send + Sync>;

/// Probe an admin origin to determine the authentication mode and whether the
/// current app keypair is authorized.
///
/// Algorithm:
/// 1. Send an unauthenticated GET to `/api/admin/v1/probe`.
/// 2. Detect HTML/interception pages (Cloudflare Access, captive portals)
///    from Content-Type and final URL host → `NetworkOrIntercepted`.
/// 3. 200 + valid `ProbeResponse` with `authMode: "disabled"` → `Disabled`.
/// 4. 401 + `WWW-Authenticate: Nostr` → NIP-98 mode. Retry with a freshly
///    signed kind-27235. 200 + valid `ProbeResponse` → `Nip98Authorized`
///    carrying the relay-resolved `role`/`source`; non-200 → `Nip98Denied`.
/// 5. 401 + `WWW-Authenticate: Bearer` → `TokenMode`.
/// 6. 403/404 or other non-401 → `NotAdminApi`.
/// 7. Network/redirect/TLS error → `NetworkOrIntercepted`.
#[tauri::command]
pub async fn admin_probe(
    origin: String,
    state: tauri::State<'_, crate::app_state::AppState>,
) -> Result<AdminProbeResult, String> {
    use crate::relay::build_nip98_auth_header_for_keys;

    // Resolve signing keys before entering the inner probe. Recovery mode
    // (locked/lost keyring) is surfaced here rather than inside the loop.
    let sign: Option<SignFn> = match state.signing_keys() {
        Ok(keys) => Some(Box::new(move |url: &str| {
            build_nip98_auth_header_for_keys(&keys, &reqwest::Method::GET, url, &[])
                .map_err(|e| format!("nip98 build failed: {e}"))
        })),
        Err(_) => None,
    };

    admin_probe_inner(&origin, sign).await
}

/// Inner probe implementation with injectable signing.
///
/// Accepts an optional signing closure so live-listener tests can drive the
/// full state machine — including the Nostr challenge/response path — without
/// requiring a real `AppState`. `None` simulates recovery mode (no key).
async fn admin_probe_inner(
    origin: &str,
    sign: Option<impl Fn(&str) -> Result<String, String>>,
) -> Result<AdminProbeResult, String> {
    let origin = origin::AdminOrigin::parse(origin)?;
    let url = origin.route_url(&routes::AdminRoute::Probe, &routes::AdminQuery::default());

    let http_client = client::ADMIN_CLIENT
        .get()
        .ok_or_else(|| "admin client not initialised".to_string())?;

    // Step 1: unauthenticated GET.
    let resp = match http_client.get(&url).send().await {
        Ok(r) => r,
        Err(e) => {
            tracing::debug!(error = %e, "admin_probe: network error");
            return Ok(AdminProbeResult::NetworkOrIntercepted);
        }
    };

    if resp.status().is_redirection() {
        return Ok(AdminProbeResult::NetworkOrIntercepted);
    }

    // Step 2: detect HTML/interception before reading body or interpreting status.
    if is_probe_response_intercepted(&resp) {
        return Ok(AdminProbeResult::NetworkOrIntercepted);
    }

    // Step 3: success without auth → disabled mode (only when the body is a
    // coherent disabled-mode probe: status ok, authMode disabled, no
    // principal, no capabilities). A 200 in any other shape or mode is a
    // contract violation (token/nip98 must 401 an unauthenticated caller);
    // classify defensively.
    if resp.status().is_success() {
        let content_type = response_content_type(&resp);
        let bytes = read_bounded(resp, PROBE_JSON_CAP).await?;
        return Ok(match parse_probe(&content_type, &bytes) {
            Some(p) if p.is_coherent_disabled() => AdminProbeResult::Disabled,
            _ => AdminProbeResult::NotAdminApi,
        });
    }

    // Step 4–6: interpret 401.
    if resp.status() == reqwest::StatusCode::UNAUTHORIZED {
        let www_auth = resp
            .headers()
            .get(reqwest::header::WWW_AUTHENTICATE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_ascii_lowercase();

        if www_auth.starts_with("nostr") {
            // NIP-98 mode: try signing.
            let auth_header = match &sign {
                Some(f) => f(&url)?,
                None => return Ok(AdminProbeResult::Nip98Denied),
            };
            let auth_resp = match http_client
                .get(&url)
                .header(reqwest::header::AUTHORIZATION, &auth_header)
                .send()
                .await
            {
                Ok(r) => r,
                Err(_) => return Ok(AdminProbeResult::NetworkOrIntercepted),
            };

            // Redirects on the authenticated retry are also interception.
            if auth_resp.status().is_redirection() {
                return Ok(AdminProbeResult::NetworkOrIntercepted);
            }

            // Validate the Authorization header was accepted by checking for HTML.
            if is_probe_response_intercepted(&auth_resp) {
                return Ok(AdminProbeResult::NetworkOrIntercepted);
            }

            if auth_resp.status().is_success() {
                let content_type = response_content_type(&auth_resp);
                let bytes = read_bounded(auth_resp, PROBE_JSON_CAP).await?;
                return Ok(match parse_probe(&content_type, &bytes) {
                    // Trust the 2xx only when the full NIP-98 invariant holds;
                    // carry the relay-resolved role/source for the staffing tab.
                    Some(p) => match p.authorized_principal() {
                        Some((role, source)) => AdminProbeResult::Nip98Authorized {
                            role: Some(role.as_str().to_string()),
                            source: Some(source.as_str().to_string()),
                        },
                        // Structurally a probe body but not a coherent
                        // authorized NIP-98 response: fail closed.
                        None => AdminProbeResult::NotAdminApi,
                    },
                    // 2xx but not a probe shape: endpoint exists but isn't
                    // the admin API.
                    None => AdminProbeResult::NotAdminApi,
                });
            }
            return Ok(AdminProbeResult::Nip98Denied);
        }

        if www_auth.starts_with("bearer") {
            return Ok(AdminProbeResult::TokenMode);
        }

        // Unknown 401 shape.
        return Ok(AdminProbeResult::NotAdminApi);
    }

    Ok(AdminProbeResult::NotAdminApi)
}

/// Check the response Content-Type and final URL host for signs of
/// captive-portal or Cloudflare Access interception.
///
/// Uses the same classification logic as `relay.rs::classify_intercepted_response`.
fn is_probe_response_intercepted(resp: &reqwest::Response) -> bool {
    let host = resp.url().host_str().unwrap_or("").to_lowercase();
    let ct = resp
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_lowercase();

    // Cloudflare Access redirects to its own domain.
    if host == "cloudflareaccess.com" || host.ends_with(".cloudflareaccess.com") {
        return true;
    }
    // Any HTML body from a non-relay host is a proxy/captive portal page.
    if ct.contains("text/html") {
        return true;
    }
    false
}

/// Read a bounded response body (no auth check, just bytes).
async fn read_bounded(resp: reqwest::Response, cap: u64) -> Result<Vec<u8>, String> {
    use futures_util::StreamExt;

    if let Some(cl) = resp.content_length() {
        if cl > cap {
            return Err(format!("probe response too large ({cl} bytes)"));
        }
    }
    let mut bytes = Vec::new();
    let mut stream = resp.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| format!("probe stream error: {e}"))?;
        if bytes.len() as u64 + chunk.len() as u64 > cap {
            return Err(format!("probe response too large (cap {cap} bytes)"));
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}

/// The relay's `/probe` response contract (`ProbeResponse` in the relay's
/// `api/admin/mod.rs`, serialised `rename_all = "camelCase"`).
///
/// All six fields are required and typed; deserialisation rejects a body that
/// omits any field or carries a wrong-typed one, so an unrelated JSON endpoint
/// cannot be mistaken for the admin API. Unknown fields are tolerated for
/// forward compatibility. Structural validity alone does NOT authorize: a
/// deserialised `ProbeWire` still has to pass [`ProbeWire::authorized_principal`]
/// or [`ProbeWire::is_coherent_disabled`] before its state is trusted.
#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct ProbeWire {
    status: String,
    auth_mode: String,
    role: Option<String>,
    source: Option<String>,
    can_act: bool,
    can_staff: bool,
}

/// The relay's resolved principal role. Closed vocabulary — an unknown string
/// fails to parse and denies authorization.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ProbeRole {
    Operator,
    Moderator,
}

impl ProbeRole {
    fn parse(s: &str) -> Option<Self> {
        match s {
            "operator" => Some(Self::Operator),
            "moderator" => Some(Self::Moderator),
            _ => None,
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::Operator => "operator",
            Self::Moderator => "moderator",
        }
    }
}

/// How the relay established the principal's role. Closed vocabulary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ProbeSource {
    Config,
    OwnerFallback,
    Db,
}

impl ProbeSource {
    fn parse(s: &str) -> Option<Self> {
        match s {
            "config" => Some(Self::Config),
            "owner_fallback" => Some(Self::OwnerFallback),
            "db" => Some(Self::Db),
            _ => None,
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::Config => "config",
            Self::OwnerFallback => "owner_fallback",
            Self::Db => "db",
        }
    }
}

impl ProbeWire {
    /// Validate the complete NIP-98 authorization invariant and return the
    /// typed principal only when every field is coherent with the relay
    /// contract: `status == "ok"`, `authMode == "nip98"`, a recognised
    /// non-null `role`/`source`, `canAct == true`, and
    /// `canStaff == (role == operator)`. Any deviation yields `None`, so the
    /// caller classifies the response `NotAdminApi` rather than trusting a
    /// fail-open 2xx.
    fn authorized_principal(&self) -> Option<(ProbeRole, ProbeSource)> {
        if self.status != "ok" || self.auth_mode != "nip98" {
            return None;
        }
        let role = ProbeRole::parse(self.role.as_deref()?)?;
        let source = ProbeSource::parse(self.source.as_deref()?)?;
        if !self.can_act || self.can_staff != (role == ProbeRole::Operator) {
            return None;
        }
        Some((role, source))
    }

    /// Validate the disabled-mode invariant for an unauthenticated 200:
    /// `status == "ok"`, `authMode == "disabled"`, no principal, no
    /// capabilities. Any deviation is a contract violation (token/nip98 must
    /// 401 an unauthenticated caller).
    fn is_coherent_disabled(&self) -> bool {
        self.status == "ok"
            && self.auth_mode == "disabled"
            && self.role.is_none()
            && self.source.is_none()
            && !self.can_act
            && !self.can_staff
    }
}

/// Parse a `/probe` response body into a [`ProbeWire`], returning `None` when
/// the Content-Type is not JSON or the body does not match the probe contract.
///
/// Strict typing rejects unrelated JSON endpoints: a response missing any
/// required field (`status`, `authMode`, `canAct`, `canStaff`) or carrying a
/// wrong-typed field fails to deserialise and yields `None`, so a non-admin
/// origin that happens to return JSON is classified `NotAdminApi` rather than
/// mistaken for the admin API.
fn parse_probe(content_type: &str, bytes: &[u8]) -> Option<ProbeWire> {
    if !content_type
        .to_ascii_lowercase()
        .starts_with("application/json")
    {
        return None;
    }
    serde_json::from_slice::<ProbeWire>(bytes).ok()
}

/// Extract the normalised Content-Type base value (strips parameters).
fn response_content_type(resp: &reqwest::Response) -> String {
    resp.headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .split(';')
        .next()
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase()
}

// ── Five typed data commands ──────────────────────────────────────────────

/// Fetch the reports list.
#[tauri::command]
pub async fn admin_list_reports(
    origin: String,
    query: AdminReportsQuery,
    state: tauri::State<'_, crate::app_state::AppState>,
) -> Result<serde_json::Value, String> {
    let origin = origin::AdminOrigin::parse(&origin)?;
    let q = routes::AdminQuery {
        community_id: query.community_id,
        status: query.status,
        report_type: query.report_type,
        target_kind: query.target_kind,
        after: query.after,
        before: query.before,
        limit: query.limit,
    };
    let url = origin.route_url(&routes::AdminRoute::ReportsList, &q);
    let bytes = fetch_admin_json(&url, SUCCESS_JSON_CAP, &state).await?;
    serde_json::from_slice(&bytes).map_err(|e| format!("invalid JSON from relay: {e}"))
}

/// Fetch a single report's detail.
#[tauri::command]
pub async fn admin_get_report(
    origin: String,
    id: String,
    state: tauri::State<'_, crate::app_state::AppState>,
) -> Result<serde_json::Value, String> {
    let origin = origin::AdminOrigin::parse(&origin)?;
    let id =
        uuid::Uuid::parse_str(&id).map_err(|_| "report id must be a valid UUID".to_string())?;
    let url = origin.route_url(
        &routes::AdminRoute::ReportDetail { id },
        &routes::AdminQuery::default(),
    );
    let bytes = fetch_admin_json(&url, SUCCESS_JSON_CAP, &state).await?;
    serde_json::from_slice(&bytes).map_err(|e| format!("invalid JSON from relay: {e}"))
}

/// Fetch the feedback list.
#[tauri::command]
pub async fn admin_list_feedback(
    origin: String,
    state: tauri::State<'_, crate::app_state::AppState>,
) -> Result<serde_json::Value, String> {
    let origin = origin::AdminOrigin::parse(&origin)?;
    let url = origin.route_url(
        &routes::AdminRoute::FeedbackList,
        &routes::AdminQuery::default(),
    );
    let bytes = fetch_admin_json(&url, SUCCESS_JSON_CAP, &state).await?;
    serde_json::from_slice(&bytes).map_err(|e| format!("invalid JSON from relay: {e}"))
}

/// Fetch a single feedback entry's detail (including imeta attachment metadata).
#[tauri::command]
pub async fn admin_get_feedback(
    origin: String,
    id: String,
    state: tauri::State<'_, crate::app_state::AppState>,
) -> Result<serde_json::Value, String> {
    let origin = origin::AdminOrigin::parse(&origin)?;
    let id =
        uuid::Uuid::parse_str(&id).map_err(|_| "feedback id must be a valid UUID".to_string())?;
    let url = origin.route_url(
        &routes::AdminRoute::FeedbackDetail { id },
        &routes::AdminQuery::default(),
    );
    let bytes = fetch_admin_json(&url, SUCCESS_JSON_CAP, &state).await?;
    serde_json::from_slice(&bytes).map_err(|e| format!("invalid JSON from relay: {e}"))
}

/// Resolve a report — POST /api/admin/v1/reports/{id}/resolve.
///
/// Body: `{action, request_id, expiration_secs?, reason?}`.
/// The `request_id` is a client-generated UUID for idempotency; the caller
/// must generate once per resolution attempt and reuse on retry.
#[tauri::command]
pub async fn admin_resolve_report(
    origin: String,
    id: String,
    body: serde_json::Value,
    state: tauri::State<'_, crate::app_state::AppState>,
) -> Result<serde_json::Value, String> {
    let origin = origin::AdminOrigin::parse(&origin)?;
    let id =
        uuid::Uuid::parse_str(&id).map_err(|_| "report id must be a valid UUID".to_string())?;
    let url = origin.route_url(
        &routes::AdminRoute::ReportResolve { id },
        &routes::AdminQuery::default(),
    );
    let body_bytes =
        serde_json::to_vec(&body).map_err(|e| format!("failed to serialise request body: {e}"))?;
    let bytes = post_admin_json(&url, &body_bytes, SUCCESS_JSON_CAP, &state).await?;
    serde_json::from_slice(&bytes).map_err(|e| format!("invalid JSON from relay: {e}"))
}

/// Reopen a resolved report — POST /api/admin/v1/reports/{id}/reopen.
///
/// Body: `{request_id, reason?}`. The `request_id` is a client-generated UUID
/// for idempotency; the caller must generate once per reopen attempt and reuse
/// on retry. Reopen is re-triage only — it moves a `resolved`/`dismissed`/
/// `escalated` report back to `open` and does not reverse any enforcement
/// action (bans, deletions) taken while it was resolved.
#[tauri::command]
pub async fn admin_reopen_report(
    origin: String,
    id: String,
    body: serde_json::Value,
    state: tauri::State<'_, crate::app_state::AppState>,
) -> Result<serde_json::Value, String> {
    let origin = origin::AdminOrigin::parse(&origin)?;
    let id =
        uuid::Uuid::parse_str(&id).map_err(|_| "report id must be a valid UUID".to_string())?;
    let url = origin.route_url(
        &routes::AdminRoute::ReportReopen { id },
        &routes::AdminQuery::default(),
    );
    let body_bytes =
        serde_json::to_vec(&body).map_err(|e| format!("failed to serialise request body: {e}"))?;
    let bytes = post_admin_json(&url, &body_bytes, SUCCESS_JSON_CAP, &state).await?;
    serde_json::from_slice(&bytes).map_err(|e| format!("invalid JSON from relay: {e}"))
}

/// Cancel a failed enforcement action — POST /api/admin/v1/reports/{id}/cancel.
///
/// Body: `{actionId}`. Cancel is the only recovery path for a pre-mutation
/// `failed` action: it returns the report to `open` for a fresh resolution.
/// The `actionId` fences the cancel to the failed action the operator observed;
/// a mismatch (already cancelled, superseded, or past the mutation point) is a
/// 409, which the caller treats as "refresh detail".
#[tauri::command]
pub async fn admin_cancel_report(
    origin: String,
    id: String,
    body: serde_json::Value,
    state: tauri::State<'_, crate::app_state::AppState>,
) -> Result<serde_json::Value, String> {
    let origin = origin::AdminOrigin::parse(&origin)?;
    let id =
        uuid::Uuid::parse_str(&id).map_err(|_| "report id must be a valid UUID".to_string())?;
    let url = origin.route_url(
        &routes::AdminRoute::ReportCancel { id },
        &routes::AdminQuery::default(),
    );
    let body_bytes =
        serde_json::to_vec(&body).map_err(|e| format!("failed to serialise request body: {e}"))?;
    let bytes = post_admin_json(&url, &body_bytes, SUCCESS_JSON_CAP, &state).await?;
    serde_json::from_slice(&bytes).map_err(|e| format!("invalid JSON from relay: {e}"))
}

/// Update feedback status — PATCH /api/admin/v1/feedback/{id}.
///
/// Body: `{status}` where status ∈ {"new","reviewed","archived"}.
#[tauri::command]
pub async fn admin_patch_feedback(
    origin: String,
    id: String,
    body: serde_json::Value,
    state: tauri::State<'_, crate::app_state::AppState>,
) -> Result<serde_json::Value, String> {
    let origin = origin::AdminOrigin::parse(&origin)?;
    let id =
        uuid::Uuid::parse_str(&id).map_err(|_| "feedback id must be a valid UUID".to_string())?;
    let url = origin.route_url(
        &routes::AdminRoute::FeedbackPatch { id },
        &routes::AdminQuery::default(),
    );
    let body_bytes =
        serde_json::to_vec(&body).map_err(|e| format!("failed to serialise request body: {e}"))?;
    let bytes = patch_admin_json(&url, &body_bytes, SUCCESS_JSON_CAP, &state).await?;
    serde_json::from_slice(&bytes).map_err(|e| format!("invalid JSON from relay: {e}"))
}

/// List operators — GET /api/admin/v1/operators.
///
/// Operator-only. Returns all effective principals with `effectiveRole` and
/// `sources[]` (`config`, `owner_fallback`, `db`).
#[tauri::command]
pub async fn admin_list_operators(
    origin: String,
    state: tauri::State<'_, crate::app_state::AppState>,
) -> Result<serde_json::Value, String> {
    let origin = origin::AdminOrigin::parse(&origin)?;
    let url = origin.route_url(
        &routes::AdminRoute::OperatorsList,
        &routes::AdminQuery::default(),
    );
    let bytes = fetch_admin_json(&url, SUCCESS_JSON_CAP, &state).await?;
    serde_json::from_slice(&bytes).map_err(|e| format!("invalid JSON from relay: {e}"))
}

/// Add or update an operator — PUT /api/admin/v1/operators/{pubkey}.
///
/// Operator-only. Body: `{role}` where role ∈ {"operator","moderator"}.
/// Returns 409 if the pubkey is config-backed (immutable via API).
#[tauri::command]
pub async fn admin_put_operator(
    origin: String,
    pubkey: String,
    body: serde_json::Value,
    state: tauri::State<'_, crate::app_state::AppState>,
) -> Result<serde_json::Value, String> {
    let origin = origin::AdminOrigin::parse(&origin)?;
    let pubkey =
        routes::HexPubkey::parse(&pubkey).map_err(|e| format!("invalid operator pubkey: {e}"))?;
    let url = origin.route_url(
        &routes::AdminRoute::OperatorPut { pubkey },
        &routes::AdminQuery::default(),
    );
    let body_bytes =
        serde_json::to_vec(&body).map_err(|e| format!("failed to serialise request body: {e}"))?;
    let bytes = put_admin_json(&url, &body_bytes, SUCCESS_JSON_CAP, &state).await?;
    serde_json::from_slice(&bytes).map_err(|e| format!("invalid JSON from relay: {e}"))
}

/// Remove an operator — DELETE /api/admin/v1/operators/{pubkey}.
///
/// Operator-only. Returns 409 if the pubkey is config-backed.
#[tauri::command]
pub async fn admin_delete_operator(
    origin: String,
    pubkey: String,
    state: tauri::State<'_, crate::app_state::AppState>,
) -> Result<serde_json::Value, String> {
    let origin = origin::AdminOrigin::parse(&origin)?;
    let pubkey =
        routes::HexPubkey::parse(&pubkey).map_err(|e| format!("invalid operator pubkey: {e}"))?;
    let url = origin.route_url(
        &routes::AdminRoute::OperatorDelete { pubkey },
        &routes::AdminQuery::default(),
    );
    let bytes = delete_admin_json(&url, SUCCESS_JSON_CAP, &state).await?;
    serde_json::from_slice(&bytes).map_err(|e| format!("invalid JSON from relay: {e}"))
}

/// Fetch a feedback attachment by SHA-256 hash.
///
/// The front-end MUST supply `expectedMime` and `expectedSize` from the
/// server-validated `imeta` fields returned by `admin_get_feedback`. The
/// command verifies the relay's `Content-Type` against `expectedMime` and
/// the actual byte count against `expectedSize`. Mismatch or over-cap yields
/// a stable typed error-code string.
///
/// Returns `tauri::ipc::Response` so bytes cross IPC as a raw `ArrayBuffer`.
#[tauri::command]
pub async fn admin_fetch_feedback_attachment(
    origin: String,
    feedback_id: String,
    sha256: String,
    expected_mime: String,
    expected_size: u64,
    state: tauri::State<'_, crate::app_state::AppState>,
) -> Result<tauri::ipc::Response, String> {
    use crate::relay::build_nip98_auth_header_for_keys;

    // Validate inputs before any network activity.
    let feedback_id = uuid::Uuid::parse_str(&feedback_id)
        .map_err(|_| "admin_attachment_invalid_feedback_id".to_string())?;
    let sha256 = routes::AttachmentHash::parse(&sha256)
        .map_err(|_| "admin_attachment_invalid_hash".to_string())?;
    if expected_size == 0 {
        return Err("admin_attachment_invalid_size".to_string());
    }
    if expected_size > ATTACHMENT_CAP {
        return Err("admin_attachment_too_large".to_string());
    }
    if expected_mime.is_empty() {
        return Err("admin_attachment_invalid_mime".to_string());
    }

    let origin = origin::AdminOrigin::parse(&origin)?;
    let url = origin.route_url(
        &routes::AdminRoute::FeedbackAttachment {
            id: feedback_id,
            sha256,
        },
        &routes::AdminQuery::default(),
    );

    let keys = state.signing_keys()?;
    let http_client = client::ADMIN_CLIENT
        .get()
        .ok_or_else(|| "admin client not initialised".to_string())?;

    let auth_header = build_nip98_auth_header_for_keys(&keys, &reqwest::Method::GET, &url, &[])
        .map_err(|e| format!("nip98 build failed: {e}"))?;

    let resp = http_client
        .get(&url)
        .header(reqwest::header::AUTHORIZATION, &auth_header)
        .send()
        .await
        .map_err(|e| {
            tracing::debug!(error = %e, "admin attachment fetch failed");
            "admin_attachment_network_error".to_string()
        })?;

    // One retry on 401 with a fresh NIP-98 event.
    if resp.status() == reqwest::StatusCode::UNAUTHORIZED {
        let auth_header2 =
            build_nip98_auth_header_for_keys(&keys, &reqwest::Method::GET, &url, &[])
                .map_err(|e| format!("nip98 build failed on retry: {e}"))?;
        let resp2 = http_client
            .get(&url)
            .header(reqwest::header::AUTHORIZATION, auth_header2)
            .send()
            .await
            .map_err(|_| "admin_attachment_network_error".to_string())?;
        return finish_attachment_response(resp2, &expected_mime, expected_size).await;
    }

    finish_attachment_response(resp, &expected_mime, expected_size).await
}

// ── Origin storage commands ───────────────────────────────────────────────

/// Core storage logic for `get_admin_origin`, parameterised by data directory
/// and resolved pubkey hex. No `tauri::State` — testable with `tempdir`.
///
/// Reads the per-pubkey JSON file, reparses the stored origin through
/// `AdminOrigin::parse()`, and returns the canonical string. Returns `None`
/// when no file exists. On malformed/invalid content, removes the file and
/// returns `Err` so the caller can surface a visible setup error.
pub(crate) fn get_admin_origin_core(
    data_dir: &std::path::Path,
    pubkey_hex: &str,
) -> Result<Option<String>, String> {
    let path = data_dir.join(format!("admin-console-origin-{pubkey_hex}.json"));
    if !path.exists() {
        return Ok(None);
    }
    let content = std::fs::read_to_string(&path)
        .map_err(|e| format!("failed to read admin console origin: {e}"))?;
    let stored: StoredAdminOrigin = match serde_json::from_str(&content) {
        Ok(v) => v,
        Err(e) => {
            let remove_result = std::fs::remove_file(&path);
            return Err(match remove_result {
                Ok(()) => format!("stored admin console origin is invalid (removed): {e}"),
                Err(re) => format!(
                    "stored admin console origin is invalid (quarantine failed — {re}): {e}"
                ),
            });
        }
    };
    match origin::AdminOrigin::parse(&stored.origin) {
        Ok(o) => Ok(Some(o.as_str().to_string())),
        Err(e) => {
            let remove_result = std::fs::remove_file(&path);
            Err(match remove_result {
                Ok(()) => format!("stored admin console origin is invalid (removed): {e}"),
                Err(re) => format!(
                    "stored admin console origin is invalid (quarantine failed — {re}): {e}"
                ),
            })
        }
    }
}

/// Core storage logic for `set_admin_origin`, parameterised by data directory
/// and resolved pubkey hex. No `tauri::State` — testable with `tempdir`.
///
/// Validates and persists `raw_origin`. Pass `None` to clear. Returns the
/// canonical origin string on success, or `None` on clear.
pub(crate) fn set_admin_origin_core(
    data_dir: &std::path::Path,
    pubkey_hex: &str,
    raw_origin: Option<String>,
) -> Result<Option<String>, String> {
    use crate::managed_agents::storage::atomic_write_json_restricted;
    let path = data_dir.join(format!("admin-console-origin-{pubkey_hex}.json"));
    match raw_origin {
        None => {
            if path.exists() {
                std::fs::remove_file(&path)
                    .map_err(|e| format!("failed to remove admin console origin: {e}"))?;
            }
            Ok(None)
        }
        Some(raw) => {
            let canonical = origin::AdminOrigin::parse(&raw)?.as_str().to_string();
            let payload = serde_json::to_vec_pretty(&StoredAdminOrigin {
                origin: canonical.clone(),
            })
            .map_err(|e| format!("failed to serialise admin console origin: {e}"))?;
            atomic_write_json_restricted(&path, &payload)?;
            Ok(Some(canonical))
        }
    }
}

/// Return the persisted admin console origin for the active pubkey, or `None`
/// if none has been saved yet.
///
/// `expected_pubkey` is checked against the active signing key before
/// reading. This is a defence-in-depth guard: if a delayed IPC call arrives
/// after the user has switched identities, the mismatch is caught here and the
/// read is rejected so stale-session data cannot surface in the new session.
///
/// The stored value is reparsed through `AdminOrigin::parse()` on every read.
/// If the stored content is invalid, it is removed and an error returned so
/// the settings card shows a visible setup error rather than silently degrading.
#[tauri::command]
pub fn get_admin_origin(
    expected_pubkey: Option<String>,
    app: tauri::AppHandle,
    state: tauri::State<'_, crate::app_state::AppState>,
) -> Result<Option<String>, String> {
    // Fail closed: never derive the pubkey from an error fallback.
    let pubkey = validate_pubkey_hex(state.signing_keys()?.public_key().to_hex())?;
    // If the caller supplied an expected pubkey, reject when it no longer
    // matches the active key — a delayed IPC from a prior session.
    if let Some(ref expected) = expected_pubkey {
        if *expected != pubkey {
            return Err(
                "admin origin read rejected: active identity changed since request was sent"
                    .to_string(),
            );
        }
    }
    use tauri::Manager as _;
    let dir = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("failed to resolve app data dir: {e}"))?;
    std::fs::create_dir_all(&dir).map_err(|e| format!("failed to create app data dir: {e}"))?;
    get_admin_origin_core(&dir, &pubkey)
}

/// Validate and persist the admin console origin for the active pubkey.
///
/// `expected_pubkey` guards against delayed IPC: if the active signing key no
/// longer matches `expected_pubkey`, the write is rejected to prevent a save
/// started under identity A from writing into identity B's storage namespace.
///
/// Passes `raw_origin` through `AdminOrigin::parse` to normalise and validate
/// it before writing. Pass `None` to clear the stored origin.
#[tauri::command]
pub fn set_admin_origin(
    raw_origin: Option<String>,
    expected_pubkey: Option<String>,
    app: tauri::AppHandle,
    state: tauri::State<'_, crate::app_state::AppState>,
) -> Result<Option<String>, String> {
    // Fail closed: never derive the pubkey from an error fallback.
    let pubkey = validate_pubkey_hex(state.signing_keys()?.public_key().to_hex())?;
    if let Some(ref expected) = expected_pubkey {
        if *expected != pubkey {
            return Err(
                "admin origin write rejected: active identity changed since request was sent"
                    .to_string(),
            );
        }
    }
    use tauri::Manager as _;
    let dir = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("failed to resolve app data dir: {e}"))?;
    std::fs::create_dir_all(&dir).map_err(|e| format!("failed to create app data dir: {e}"))?;
    set_admin_origin_core(&dir, &pubkey, raw_origin)
}

/// On-disk shape for the persisted admin console origin.
#[derive(serde::Serialize, serde::Deserialize)]
struct StoredAdminOrigin {
    origin: String,
}

/// Validate that `hex` is exactly 64 lowercase hexadecimal characters.
///
/// `nostr::Keys::public_key().to_hex()` always produces this form, but this
/// check serves as a defence-in-depth guard against future API changes or
/// unexpected fallbacks that could produce a non-canonical string and silently
/// corrupt the filename-based per-pubkey namespace.
fn validate_pubkey_hex(hex: String) -> Result<String, String> {
    if hex.len() == 64 && hex.chars().all(|c| matches!(c, '0'..='9' | 'a'..='f')) {
        Ok(hex)
    } else {
        Err("signing key produced an unexpected pubkey format; cannot scope storage".to_string())
    }
}

// ── NIP-11 admin-origin discovery ─────────────────────────────────────────

/// Minimal projection of the relay's NIP-11 information document — only the
/// field needed to auto-discover the admin console origin. Unknown fields are
/// ignored, so a full NIP-11 document deserializes cleanly.
#[derive(serde::Deserialize)]
struct AdminApiInfo {
    #[serde(default)]
    admin_api: Option<String>,
}

/// Validate a relay-advertised `admin_api` value into a canonical origin.
///
/// The value is untrusted relay input: it is accepted only if it passes the
/// same `AdminOrigin` validation applied to operator-entered URLs (loopback
/// `http` or any `https`, origin only — no path, query, fragment, or
/// credentials). An absent or invalid value yields `None` so the desktop falls
/// back to manual entry rather than probing a malformed origin.
fn admin_origin_from_nip11(info: &AdminApiInfo) -> Option<String> {
    let raw = info.admin_api.as_deref()?;
    origin::AdminOrigin::parse(raw)
        .ok()
        .map(|origin| origin.as_str().to_string())
}

/// Fetch the relay's NIP-11 document and extract a validated admin origin.
///
/// Returns `Ok(Some(origin))` when the relay advertises a valid `admin_api`,
/// `Ok(None)` when the field is absent or fails validation, and `Err` on a
/// transport or non-2xx failure. Split from the Tauri command so it can be
/// exercised against a live test server without constructing `AppState`.
async fn discover_admin_origin_at(
    client: &reqwest::Client,
    relay_http_base: &str,
) -> Result<Option<String>, String> {
    use crate::relay::{classify_request_error, parse_json_response, relay_error_message};

    let url = format!("{}/info", relay_http_base.trim_end_matches('/'));
    let response = client
        .get(url)
        .header("Accept", "application/nostr+json")
        .send()
        .await
        .map_err(|error| classify_request_error(&error))?;

    if !response.status().is_success() {
        return Err(relay_error_message(response).await);
    }

    let info = parse_json_response::<AdminApiInfo>(response).await?;
    Ok(admin_origin_from_nip11(&info))
}

/// Auto-discover the admin console origin from the connected relay's NIP-11
/// document. Returns the canonical origin when the relay advertises a valid
/// `admin_api`, or `None` when it does not — the caller falls back to manual
/// entry. Mirrors the native NIP-11 fetch used by `relay_requires_membership`.
#[tauri::command]
pub async fn admin_discover_origin(
    state: tauri::State<'_, crate::app_state::AppState>,
) -> Result<Option<String>, String> {
    let base = crate::relay::relay_api_base_url_with_override(&state);
    discover_admin_origin_at(&state.http_client, &base).await
}

#[cfg(test)]
#[path = "mod_tests.rs"]
mod tests;
