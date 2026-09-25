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
pub(crate) mod dns;
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
    delete_admin_json, fetch_admin_json, patch_admin_json, post_admin_json, put_admin_json,
};

// ── Typed mutation error ──────────────────────────────────────────────────

pub(crate) mod error;

mod attachment;
pub use error::AdminMutationError;

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
    /// Visibility scope forwarded to the relay's `scope` query parameter.
    /// `Some("all")` requests every status; `None` uses the relay's default
    /// (escalated-only). Only `"all"` is a valid value — the TypeScript layer
    /// constrains the type to `"all" | undefined`.
    pub scope: Option<String>,
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
/// 5. Any other 401 (including a `WWW-Authenticate: Bearer` challenge, which
///    is no longer a recognized Buzz admin mode) → `NotAdminApi`.
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

        // Any other 401 shape (including a Bearer challenge, which is no longer
        // a recognized Buzz admin mode) is an unrecognized auth challenge.
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
        scope: query.scope,
        cursor: None,
        community_host: None,
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
) -> Result<serde_json::Value, AdminMutationError> {
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
    serde_json::from_slice(&bytes).map_err(|e| format!("invalid JSON from relay: {e}").into())
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
) -> Result<serde_json::Value, AdminMutationError> {
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
    serde_json::from_slice(&bytes).map_err(|e| format!("invalid JSON from relay: {e}").into())
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
) -> Result<serde_json::Value, AdminMutationError> {
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
    serde_json::from_slice(&bytes).map_err(|e| format!("invalid JSON from relay: {e}").into())
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
) -> Result<serde_json::Value, AdminMutationError> {
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
    serde_json::from_slice(&bytes).map_err(|e| format!("invalid JSON from relay: {e}").into())
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
) -> Result<serde_json::Value, AdminMutationError> {
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
    serde_json::from_slice(&bytes).map_err(|e| format!("invalid JSON from relay: {e}").into())
}

/// Remove an operator — DELETE /api/admin/v1/operators/{pubkey}.
///
/// Operator-only. Returns 409 if the pubkey is config-backed.
#[tauri::command]
pub async fn admin_delete_operator(
    origin: String,
    pubkey: String,
    state: tauri::State<'_, crate::app_state::AppState>,
) -> Result<serde_json::Value, AdminMutationError> {
    let origin = origin::AdminOrigin::parse(&origin)?;
    let pubkey =
        routes::HexPubkey::parse(&pubkey).map_err(|e| format!("invalid operator pubkey: {e}"))?;
    let url = origin.route_url(
        &routes::AdminRoute::OperatorDelete { pubkey },
        &routes::AdminQuery::default(),
    );
    let bytes = delete_admin_json(&url, SUCCESS_JSON_CAP, &state).await?;
    serde_json::from_slice(&bytes).map_err(|e| format!("invalid JSON from relay: {e}").into())
}

/// Fetch a feedback attachment by SHA-256 hash for in-app preview.
///
/// The front-end MUST supply `expectedMime` and `expectedSize` from the
/// server-validated `imeta` fields returned by `admin_get_feedback`; see
/// [`attachment::fetch_feedback_attachment`] for the checks applied.
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
    let keys = state.signing_keys()?;
    attachment::fetch_feedback_attachment(
        &origin,
        &feedback_id,
        &sha256,
        &expected_mime,
        expected_size,
        &keys,
        helpers::AttachmentUse::Preview,
    )
    .await
    .map(tauri::ipc::Response::new)
}

/// Save a feedback attachment to a user-chosen path via the native save dialog.
///
/// Fetches through the same validated path as preview, so non-image bytes are
/// never left as an in-memory blob URL (a WKWebView no-op for `<a download>`).
/// Returns `Ok(true)` when the file was written, `Ok(false)` when the user
/// cancelled the dialog.
#[tauri::command]
pub async fn admin_save_attachment(
    origin: String,
    feedback_id: String,
    sha256: String,
    expected_mime: String,
    expected_size: u64,
    app: tauri::AppHandle,
    state: tauri::State<'_, crate::app_state::AppState>,
) -> Result<bool, String> {
    let keys = state.signing_keys()?;
    attachment::save_feedback_attachment(
        &origin,
        &feedback_id,
        &sha256,
        &expected_mime,
        expected_size,
        &keys,
        |name, filter, ext| async move {
            crate::commands::export_util::pick_save_path(&app, &name, filter, &[ext]).await
        },
    )
    .await
}

// ── Member restrictions ───────────────────────────────────────────────────

/// Shown by the UI when a restriction call targets a relay other than the one
/// its list loaded from.
const RELAY_SCOPE_CHANGED: &str =
    "active community changed since restrictions loaded; nothing was sent. Reload to continue.";

/// Build a restrictions-route URL scoped to the active relay's community.
///
/// The community is named by the active relay's host authority — the same
/// `relay_url_authority` the relay's own connection binding uses — so the relay
/// resolves its tenant. The desktop's local community ids are never sent.
///
/// `expected_relay` is the relay the caller's restriction list loaded from. The
/// native relay is read once; if it no longer matches, the call fails before
/// any request so a workspace switch cannot retarget a page or removal.
fn restrictions_url(
    origin: &str,
    route: &routes::AdminRoute,
    cursor: Option<String>,
    expected_relay: &str,
    state: &crate::app_state::AppState,
) -> Result<String, String> {
    let origin = origin::AdminOrigin::parse(origin)?;
    let relay_base = crate::relay::relay_api_base_url_with_override(state);
    if expected_relay.trim().is_empty()
        || crate::relay::assert_expected_relay_scope(Some(expected_relay), &relay_base).is_err()
    {
        return Err(RELAY_SCOPE_CHANGED.to_string());
    }
    let host = buzz_core_pkg::tenant::relay_url_authority(&relay_base);
    if host.is_empty() {
        return Err("admin_community_host_unresolved".to_string());
    }
    let q = routes::AdminQuery {
        community_host: Some(host),
        cursor,
        ..Default::default()
    };
    Ok(origin.route_url(route, &q))
}

/// List active bans and timeouts for the active relay's community —
/// GET /api/admin/v1/members/restrictions?communityHost={host}[&cursor={token}].
///
/// Returns `{ items: [...], nextCursor: string|null }`. Pass the previous
/// page's `nextCursor` as `cursor` to fetch the next page (relay page size 200).
#[tauri::command]
pub async fn admin_list_restrictions(
    origin: String,
    cursor: Option<String>,
    expected_relay: String,
    state: tauri::State<'_, crate::app_state::AppState>,
) -> Result<serde_json::Value, String> {
    let url = restrictions_url(
        &origin,
        &routes::AdminRoute::MemberRestrictionsList,
        cursor,
        &expected_relay,
        &state,
    )?;
    let bytes = fetch_admin_json(&url, SUCCESS_JSON_CAP, &state).await?;
    serde_json::from_slice(&bytes).map_err(|e| format!("invalid JSON from relay: {e}"))
}

/// Lift an active ban — DELETE /api/admin/v1/members/{pubkey}/ban?communityHost={host}.
///
/// Returns 204 on success, 409 when no active ban exists for this member.
/// A 409 is surfaced as an `AdminMutationError` so the UI can handle it
/// gracefully ("no active ban").
#[tauri::command]
pub async fn admin_lift_ban(
    origin: String,
    pubkey: String,
    expected_relay: String,
    state: tauri::State<'_, crate::app_state::AppState>,
) -> Result<(), AdminMutationError> {
    let pubkey =
        routes::HexPubkey::parse(&pubkey).map_err(|e| format!("invalid member pubkey: {e}"))?;
    let url = restrictions_url(
        &origin,
        &routes::AdminRoute::MemberBanDelete { pubkey },
        None,
        &expected_relay,
        &state,
    )?;
    // 204 No Content: empty body is the success signal. delete_admin_json
    // returns Ok(vec![]) for 204; we discard the bytes and return ().
    let _bytes = delete_admin_json(&url, SUCCESS_JSON_CAP, &state).await?;
    Ok(())
}

/// Lift an active timeout — DELETE /api/admin/v1/members/{pubkey}/timeout?communityHost={host}.
///
/// Returns 204 on success, 409 when no active timeout exists for this member.
#[tauri::command]
pub async fn admin_lift_timeout(
    origin: String,
    pubkey: String,
    expected_relay: String,
    state: tauri::State<'_, crate::app_state::AppState>,
) -> Result<(), AdminMutationError> {
    let pubkey =
        routes::HexPubkey::parse(&pubkey).map_err(|e| format!("invalid member pubkey: {e}"))?;
    let url = restrictions_url(
        &origin,
        &routes::AdminRoute::MemberTimeoutDelete { pubkey },
        None,
        &expected_relay,
        &state,
    )?;
    let _bytes = delete_admin_json(&url, SUCCESS_JSON_CAP, &state).await?;
    Ok(())
}

// ── Direct moderation actions ─────────────────────────────────────────────

/// A direct ban, timeout, or delete as the confirm dialog froze it. The same
/// intent (including `request_id`) is resent verbatim on every retry; only the
/// NIP-98 signature is minted fresh per attempt.
#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AdminDirectIntent {
    pub origin: String,
    /// Relay the intent was confirmed against.
    pub expected_relay: String,
    /// Signer the intent was confirmed under.
    pub expected_pubkey: String,
    /// Typed community host; sent only as the `communityHost` query parameter.
    pub community_host: String,
    pub action: DirectAction,
    /// Member pubkey for ban/timeout, event id for delete (64 lowercase hex).
    pub target: String,
    pub request_id: uuid::Uuid,
    pub reason: Option<String>,
    pub expiration_secs: Option<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DirectAction {
    Ban,
    Timeout,
    Delete,
}

/// Build the URL and body for a direct action, refusing it before any request
/// when the active relay or signer no longer matches what was confirmed.
fn direct_action_request(
    intent: &AdminDirectIntent,
    relay_base: &str,
    signer_hex: &str,
) -> Result<(String, Vec<u8>), String> {
    let origin = origin::AdminOrigin::parse(&intent.origin)?;
    if intent.expected_relay.trim().is_empty()
        || crate::relay::assert_expected_relay_scope(Some(&intent.expected_relay), relay_base)
            .is_err()
    {
        return Err(RELAY_SCOPE_CHANGED.to_string());
    }
    if intent.expected_pubkey.trim().is_empty()
        || crate::relay::assert_expected_signer(Some(&intent.expected_pubkey), signer_hex).is_err()
    {
        return Err(
            "active identity changed since the action was confirmed; nothing was sent".to_string(),
        );
    }
    let host = buzz_core_pkg::tenant::validate_community_host(intent.community_host.trim())
        .map_err(|e| format!("invalid community host: {e}"))?;
    let target =
        routes::HexPubkey::parse(&intent.target).map_err(|e| format!("invalid target: {e}"))?;
    let route = match intent.action {
        DirectAction::Ban => routes::AdminRoute::MemberBan { pubkey: target },
        DirectAction::Timeout => routes::AdminRoute::MemberTimeout { pubkey: target },
        DirectAction::Delete => routes::AdminRoute::EventDelete { id: target },
    };
    let expiration_secs = match (intent.action, intent.expiration_secs) {
        (DirectAction::Timeout, Some(secs)) if secs > 0 => Some(secs),
        (DirectAction::Timeout, _) => return Err("timeout needs a positive duration".to_string()),
        (_, Some(_)) => return Err("only a timeout takes a duration".to_string()),
        (_, None) => None,
    };
    let mut body = serde_json::json!({ "requestId": intent.request_id });
    if let Some(reason) = &intent.reason {
        body["reason"] = reason.clone().into();
    }
    if let Some(secs) = expiration_secs {
        body["expirationSecs"] = secs.into();
    }
    let q = routes::AdminQuery {
        community_host: Some(host),
        ..Default::default()
    };
    let body = serde_json::to_vec(&body).map_err(|e| format!("failed to serialise body: {e}"))?;
    Ok((origin.route_url(&route, &q), body))
}

/// Ban, time out, or delete without a report — POST
/// /api/admin/v1/{members/{pubkey}/ban|members/{pubkey}/timeout|events/{id}/delete}?communityHost=.
///
/// Returns the relay's `{actionId, state, replayed}` (200) or `{state:"pending"}` (202).
#[tauri::command]
pub async fn admin_direct_action(
    intent: AdminDirectIntent,
    state: tauri::State<'_, crate::app_state::AppState>,
) -> Result<serde_json::Value, AdminMutationError> {
    let keys = state.signing_keys()?;
    send_direct_action(&intent, keys, &state).await
}

/// Validate `intent` against the `keys` snapshot and sign every request with
/// that same snapshot, so an identity swapped in after the snapshot can never
/// sign an action confirmed under the previous one.
async fn send_direct_action(
    intent: &AdminDirectIntent,
    keys: nostr::Keys,
    state: &crate::app_state::AppState,
) -> Result<serde_json::Value, AdminMutationError> {
    let relay_base = crate::relay::relay_api_base_url_with_override(state);
    let (url, body) = direct_action_request(intent, &relay_base, &keys.public_key().to_hex())?;
    let bytes = helpers::send_admin_mutation(
        &keys,
        reqwest::Method::POST,
        &url,
        Some(&body),
        SUCCESS_JSON_CAP,
    )
    .await?;
    serde_json::from_slice(&bytes).map_err(|e| format!("invalid JSON from relay: {e}").into())
}

// ── Origin storage commands ───────────────────────────────────────────────

/// Core storage logic for `get_admin_origin`, parameterised by data directory
/// and resolved pubkey hex; the file is scoped to the state's active relay host.
///
/// Reads the per-pubkey-per-relay JSON file, reparses the stored origin through
/// `AdminOrigin::parse()`, and returns the canonical string. Returns `None`
/// when no file exists. On malformed/invalid content, removes the file and
/// returns `Err` so the caller can surface a visible setup error.
pub(crate) fn get_admin_origin_core(
    data_dir: &std::path::Path,
    pubkey_hex: &str,
    state: &crate::app_state::AppState,
) -> Result<Option<String>, String> {
    let path = admin_origin_path(data_dir, pubkey_hex, state);
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
/// and resolved pubkey hex; the file is scoped to the state's active relay host.
///
/// Validates and persists `raw_origin`. Pass `None` to clear. Returns the
/// canonical origin string on success, or `None` on clear.
pub(crate) fn set_admin_origin_core(
    data_dir: &std::path::Path,
    pubkey_hex: &str,
    state: &crate::app_state::AppState,
    raw_origin: Option<String>,
) -> Result<Option<String>, String> {
    use crate::managed_agents::storage::atomic_write_json_restricted;
    let path = admin_origin_path(data_dir, pubkey_hex, state);
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

/// Return the persisted admin console origin for the active pubkey and connected
/// relay, or `None` if none has been saved yet.
///
/// The origin is stored per `(pubkey, relay_host)` so a prod-connected build
/// never reads a staging origin and vice versa. `expected_pubkey` is checked
/// against the active signing key before reading as a defence-in-depth guard
/// against delayed IPC from a prior session.
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
    get_admin_origin_core(&dir, &pubkey, &state)
}

/// Validate and persist the admin console origin for the active pubkey and
/// connected relay.
///
/// The origin is stored per `(pubkey, relay_host)` so prod and staging builds
/// never share or overwrite each other's saved origin. `expected_pubkey` guards
/// against delayed IPC: if the active signing key no longer matches
/// `expected_pubkey`, the write is rejected.
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
    set_admin_origin_core(&dir, &pubkey, &state, raw_origin)
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

/// Origin file for `(pubkey, active relay host)`. Origins saved under the
/// pre-per-relay name (`admin-console-origin-{pubkey}.json`) are not migrated.
fn admin_origin_path(
    data_dir: &std::path::Path,
    pubkey_hex: &str,
    state: &crate::app_state::AppState,
) -> std::path::PathBuf {
    let relay_slug = relay_host_slug(state);
    data_dir.join(format!(
        "admin-console-origin-{pubkey_hex}-{relay_slug}.json"
    ))
}

/// Derive a safe filename slug from the connected relay's host.
///
/// Extracts the host from the active relay HTTP base URL (e.g.
/// `https://relay.example.com` → `relay.example.com`) and strips any
/// characters that are not safe in filenames across all platforms.
/// Falls back to `"default"` on a malformed or absent URL so storage
/// never breaks during relay bootstrapping.
fn relay_host_slug(state: &crate::app_state::AppState) -> String {
    let base = crate::relay::relay_api_base_url_with_override(state);
    // `Url` lowercases DNS hosts and canonicalizes IPv6 literals, so every
    // spelling of one address yields one slug. The port is dropped on
    // purpose: `relay.example.com:8080` and `relay.example.com` share an entry.
    match url::Url::parse(&base)
        .ok()
        .and_then(|u| u.host().map(|h| h.to_owned()))
    {
        // `_` never appears in a DNS slug, so IPv6 slugs cannot collide with
        // one; all eight segments are kept, so they cannot collide with each other.
        Some(url::Host::Ipv6(addr)) => format!(
            "ip6_{}",
            addr.segments()
                .iter()
                .map(|seg| format!("{seg:x}"))
                .collect::<Vec<_>>()
                .join("-")
        ),
        Some(url::Host::Ipv4(addr)) => addr.to_string(),
        Some(url::Host::Domain(host)) => {
            // Retain only hostname-safe chars so the slug is filename-safe.
            let slug: String = host
                .chars()
                .map(|c| {
                    if c.is_ascii_alphanumeric() || c == '-' || c == '.' {
                        c.to_ascii_lowercase()
                    } else {
                        '-'
                    }
                })
                .collect();
            let slug = slug.trim_matches('-');
            if slug.is_empty() {
                "default".to_string()
            } else {
                slug.to_string()
            }
        }
        None => "default".to_string(),
    }
}

// ── NIP-11 admin-origin discovery ─────────────────────────────────────────

mod discovery;

/// A discovered admin console origin plus its same-host trust binding.
///
/// Serialised for the webview as `{ origin, sameHost }`. `sameHost` is `true`
/// only when the advertised origin's host matches the connected relay's host;
/// the TypeScript layer auto-saves + auto-probes a same-host origin but treats a
/// cross-host advertisement (`sameHost == false`) as pre-fill-only, so the
/// operator's key never signs a NIP-98 challenge against an unrelated,
/// relay-advertised host without explicit confirmation. See `discovery.rs` for
/// the full trust model.
#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiscoveredAdminOrigin {
    origin: String,
    same_host: bool,
}

/// Auto-discover the admin console origin from the connected relay's NIP-11
/// document. Returns the canonical origin and its `sameHost` flag when the relay
/// advertises a valid `admin_api` that does not resolve to a private/reserved
/// target, or `None` otherwise.
///
/// `sameHost` binds the discovered origin to the connected relay's host: a
/// same-host origin is auto-saved and auto-probed by design (intentional
/// first-mount UX), while a cross-host advertisement is surfaced only as a
/// pre-fill the operator must explicitly save. This prevents a malicious relay
/// from advertising an attacker-controlled `admin_api` and harvesting an
/// unconsented NIP-98 signature the moment `admin_probe` runs; residual exposure
/// is bounded because the NIP-98 header binds the exact URL, method, and payload.
/// Mirrors the native NIP-11 fetch used by `relay_requires_membership`.
#[tauri::command]
pub async fn admin_discover_origin(
    state: tauri::State<'_, crate::app_state::AppState>,
) -> Result<Option<DiscoveredAdminOrigin>, String> {
    let base = crate::relay::relay_api_base_url_with_override(&state);
    discovery::discover_admin_origin_at(&state.http_client, &base).await
}

#[cfg(test)]
#[path = "mod_tests.rs"]
mod tests;
