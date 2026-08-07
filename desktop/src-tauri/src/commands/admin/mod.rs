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
pub(crate) mod origin;
pub(crate) mod routes;

// ── Response size caps ────────────────────────────────────────────────────

/// Success-JSON cap: reports list returns up to 200 rows, each note field
/// can reach the 256 KiB event-content cap. Sized for the worst case.
const SUCCESS_JSON_CAP: u64 = 52_428_800; // 50 MiB

/// Error-body cap: relay error responses are brief JSON envelopes.
const ERROR_BODY_CAP: u64 = 65_536; // 64 KiB

/// Attachment preview cap. 10 MiB is generous for images and small documents
/// while protecting against accidental OOM.
const ATTACHMENT_CAP: u64 = 10_485_760; // 10 MiB

// ── Typed probe result ────────────────────────────────────────────────────

/// Result of an `admin_probe` call. Each variant maps to a distinct UI state.
/// Tauri serialises this as `{ "state": "<camelCaseVariant>" }`.
#[derive(Debug, serde::Serialize)]
#[serde(tag = "state", rename_all = "camelCase")]
pub enum AdminProbeResult {
    /// NIP-98 mode is active and the current app keypair is on the allowlist.
    Nip98Authorized,
    /// NIP-98 mode is active but the app keypair was rejected after a signed
    /// attempt. Likely: pubkey not in `BUZZ_ADMIN_PUBKEYS`, clock skew, or
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
/// 1. Send an unauthenticated GET to `/api/admin/v1/reports?limit=1`.
/// 2. Detect HTML/interception pages (Cloudflare Access, captive portals)
///    from Content-Type and final URL host → `NetworkOrIntercepted`.
/// 3. 200 + valid JSON list shape → `Disabled` (admin accessible without cred).
/// 4. 401 + `WWW-Authenticate: Nostr` → NIP-98 mode. Retry with a freshly
///    signed kind-27235. 200 + valid list shape → `Nip98Authorized`;
///    non-200 → `Nip98Denied`.
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
    let url = origin.route_url(
        &routes::AdminRoute::ReportsList,
        &routes::AdminQuery {
            limit: Some(1),
            ..Default::default()
        },
    );

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

    // Step 3: success without auth → disabled mode (if body is a valid list).
    if resp.status().is_success() {
        let content_type = response_content_type(&resp);
        let bytes = read_bounded(resp, SUCCESS_JSON_CAP).await?;
        return if looks_like_admin_list(&content_type, &bytes) {
            Ok(AdminProbeResult::Disabled)
        } else {
            Ok(AdminProbeResult::NotAdminApi)
        };
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
                // Validate the Nostr header shape was accepted (not just any 2xx).
                let content_type = response_content_type(&auth_resp);
                let bytes = read_bounded(auth_resp, SUCCESS_JSON_CAP).await?;
                return if looks_like_admin_list(&content_type, &bytes) {
                    Ok(AdminProbeResult::Nip98Authorized)
                } else {
                    // Endpoint exists but didn't return the expected list shape.
                    Ok(AdminProbeResult::NotAdminApi)
                };
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

/// Returns true when `content_type` is JSON and `bytes` deserialises to a
/// JSON array matching the `/api/admin/v1/reports` shape.
///
/// Rules:
/// - Content-Type must start with `application/json` (case-insensitive).
/// - Body must be a JSON array.
/// - Non-empty arrays must have every element deserialise against the
///   `AdminReport` wire contract (camelCase, `rename_all = "camelCase"`).
///   An empty array is valid — a fresh relay with no reports returns `[]`.
/// - Partial / garbage elements (`{"id":null}`, `7`, `"garbage"`) are rejected.
///
/// This prevents unrelated endpoints that return JSON arrays from being
/// misclassified as the admin API.
fn looks_like_admin_list(content_type: &str, bytes: &[u8]) -> bool {
    // Require JSON Content-Type.
    if !content_type
        .to_ascii_lowercase()
        .starts_with("application/json")
    {
        return false;
    }
    // Body must be a JSON array.
    let arr = match serde_json::from_slice::<serde_json::Value>(bytes) {
        Ok(serde_json::Value::Array(a)) => a,
        _ => return false,
    };
    // Empty array is valid (fresh relay with no reports).
    if arr.is_empty() {
        return true;
    }
    // Non-empty: every element must deserialise against the AdminReport probe DTO.
    // The wire shape is camelCase (serde rename_all = "camelCase").
    arr.iter()
        .all(|v| serde_json::from_value::<AdminReportProbeDto>(v.clone()).is_ok())
}

/// Full `AdminReport` wire contract used for probe validation.
///
/// Mirrors the camelCase serialisation of `AdminReport` in
/// `crates/buzz-db/src/admin_moderation.rs:24-55` exactly — required fields
/// are typed strictly, optional fields use `Option<T>` with real types.
/// This ensures that a response with `createdAt: null` or a malformed optional
/// field (e.g. `channelId: 7`) is rejected, not silently classified as the
/// admin API.
#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct AdminReportProbeDto {
    #[allow(dead_code)]
    id: uuid::Uuid,
    #[allow(dead_code)]
    community_id: uuid::Uuid,
    #[allow(dead_code)]
    community_host: String,
    #[allow(dead_code)]
    report_event_id: String,
    #[allow(dead_code)]
    reporter_pubkey: String,
    #[allow(dead_code)]
    target_kind: String,
    #[allow(dead_code)]
    target: String,
    #[allow(dead_code)]
    channel_id: Option<uuid::Uuid>,
    #[allow(dead_code)]
    report_type: String,
    #[allow(dead_code)]
    note: Option<String>,
    #[allow(dead_code)]
    status: String,
    #[allow(dead_code)]
    resolved_by: Option<String>,
    #[allow(dead_code)]
    resolved_at: Option<chrono::DateTime<chrono::Utc>>,
    #[allow(dead_code)]
    action_id: Option<uuid::Uuid>,
    #[allow(dead_code)]
    created_at: chrono::DateTime<chrono::Utc>,
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

// ── Internal helpers ──────────────────────────────────────────────────────

/// Fetch a JSON endpoint with NIP-98 auth, one 401-retry, and a size cap.
async fn fetch_admin_json(
    url: &str,
    cap: u64,
    state: &tauri::State<'_, crate::app_state::AppState>,
) -> Result<Vec<u8>, String> {
    use crate::relay::build_nip98_auth_header_for_keys;

    let keys = state.signing_keys()?;
    let http_client = client::ADMIN_CLIENT
        .get()
        .ok_or_else(|| "admin client not initialised".to_string())?;

    let auth_header = build_nip98_auth_header_for_keys(&keys, &reqwest::Method::GET, url, &[])
        .map_err(|e| format!("nip98 build failed: {e}"))?;

    let resp = http_client
        .get(url)
        .header(reqwest::header::AUTHORIZATION, &auth_header)
        .send()
        .await
        .map_err(|e| crate::relay::classify_request_error(&e))?;

    // One retry on 401 with a fresh NIP-98 event (new nonce).
    if resp.status() == reqwest::StatusCode::UNAUTHORIZED {
        let auth_header2 = build_nip98_auth_header_for_keys(&keys, &reqwest::Method::GET, url, &[])
            .map_err(|e| format!("nip98 build failed on retry: {e}"))?;
        let resp2 = http_client
            .get(url)
            .header(reqwest::header::AUTHORIZATION, auth_header2)
            .send()
            .await
            .map_err(|e| crate::relay::classify_request_error(&e))?;
        return read_admin_response(resp2, cap, ERROR_BODY_CAP).await;
    }

    read_admin_response(resp, cap, ERROR_BODY_CAP).await
}

/// Stream and validate an attachment response, enforcing Content-Type, size,
/// and the cap.
async fn finish_attachment_response(
    resp: reqwest::Response,
    expected_mime: &str,
    expected_size: u64,
) -> Result<tauri::ipc::Response, String> {
    use futures_util::StreamExt;

    if resp.status().is_redirection() {
        return Err("admin_attachment_redirect".to_string());
    }
    if !resp.status().is_success() {
        return Err(format!(
            "admin_attachment_relay_error_{}",
            resp.status().as_u16()
        ));
    }

    // Verify Content-Type before reading the body.
    let content_type = resp
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .split(';')
        .next()
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase();
    if content_type != expected_mime.trim().to_ascii_lowercase() {
        return Err("admin_attachment_mime_mismatch".to_string());
    }

    // Content-Length preflight.
    if let Some(cl) = resp.content_length() {
        if cl > ATTACHMENT_CAP {
            return Err("admin_attachment_too_large".to_string());
        }
        if cl != expected_size {
            return Err("admin_attachment_size_mismatch".to_string());
        }
    }

    // Stream with running byte counter.
    let mut bytes: Vec<u8> = Vec::new();
    let mut stream = resp.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|_| "admin_attachment_stream_error".to_string())?;
        if bytes.len() as u64 + chunk.len() as u64 > ATTACHMENT_CAP {
            return Err("admin_attachment_too_large".to_string());
        }
        bytes.extend_from_slice(&chunk);
    }

    // Final size check.
    if bytes.len() as u64 != expected_size {
        return Err("admin_attachment_size_mismatch".to_string());
    }

    Ok(tauri::ipc::Response::new(bytes))
}

/// Read a response body up to `success_cap` bytes on 2xx, `error_cap` on
/// non-2xx. Redirects are treated as errors (the no-redirect client surfaced
/// them rather than following).
async fn read_admin_response(
    resp: reqwest::Response,
    success_cap: u64,
    error_cap: u64,
) -> Result<Vec<u8>, String> {
    use futures_util::StreamExt;

    if resp.status().is_redirection() {
        return Err(format!(
            "admin API returned a {} redirect (not followed)",
            resp.status()
        ));
    }

    let (is_success, cap) = if resp.status().is_success() {
        (true, success_cap)
    } else {
        (false, error_cap)
    };

    if let Some(cl) = resp.content_length() {
        if cl > cap {
            return Err(format!(
                "admin response too large ({cl} bytes, cap {cap} bytes)"
            ));
        }
    }

    let mut bytes: Vec<u8> = Vec::new();
    let mut stream = resp.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| format!("admin response stream error: {e}"))?;
        if bytes.len() as u64 + chunk.len() as u64 > cap {
            return Err(format!("admin response too large (cap {cap} bytes)"));
        }
        bytes.extend_from_slice(&chunk);
    }

    if !is_success {
        let body = String::from_utf8_lossy(&bytes);
        return Err(format!("admin API error: {body}"));
    }

    Ok(bytes)
}

#[cfg(test)]
#[path = "mod_tests.rs"]
mod tests;
