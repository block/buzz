//! Unit and integration tests for `commands/admin/mod.rs` (split to keep `mod.rs`
//! under the 1000-line file-size ratchet).
//!
//! Included via `#[path = "mod_tests.rs"] mod tests;` at the bottom of `mod.rs`,
//! so `use super::*` gives access to all items in that module.

use super::*;
use crate::commands::admin::{origin::AdminOrigin, routes::AdminRoute};
use std::sync::Arc;

/// Type alias for the request inspector closure passed to `serve_sequence_inspect`.
type RequestInspector = std::sync::Arc<dyn Fn(usize, &[u8]) + Send + Sync>;

/// Parsed HTTP request data for transport-layer assertions.
#[derive(Debug)]
struct RequestRecord {
    method: String,
    path: String,
    auth: Option<String>,
}

// ── AdminOrigin × routes integration ─────────────────────────────────────

#[test]
fn reports_list_url_contains_api_prefix() {
    let o = AdminOrigin::parse("https://admin.example.com").unwrap();
    let url = o.route_url(&AdminRoute::ReportsList, &routes::AdminQuery::default());
    assert!(
        url.starts_with("https://admin.example.com/api/admin/v1/"),
        "URL must include /api/admin/v1/ prefix: {url}"
    );
}

#[test]
fn localhost_uses_http_prefix() {
    let o = AdminOrigin::parse("http://localhost:3000").unwrap();
    let url = o.route_url(&AdminRoute::FeedbackList, &routes::AdminQuery::default());
    assert!(url.starts_with("http://localhost:3000/api/admin/v1/"));
}

// ── Attachment command validation (calls production validators) ───────────

#[test]
fn attachment_hash_valid_lowercase_hex_accepted() {
    let result = routes::AttachmentHash::parse(&"a".repeat(64));
    assert!(result.is_ok(), "64 lowercase hex chars must be accepted");
}

#[test]
fn attachment_hash_uppercase_rejected_by_production_validator() {
    let result = routes::AttachmentHash::parse(&"A".repeat(64));
    assert!(
        result.is_err(),
        "uppercase hex must be rejected — relay returns 404 for uppercase hashes"
    );
}

#[test]
fn attachment_hash_63_chars_rejected_by_production_validator() {
    let result = routes::AttachmentHash::parse(&"a".repeat(63));
    assert!(result.is_err(), "63 chars must be rejected");
}

#[test]
fn feedback_id_malformed_uuid_rejected_by_production_validator() {
    let result = uuid::Uuid::parse_str("not-a-uuid");
    assert!(result.is_err(), "non-UUID feedback id must be rejected");
}

#[test]
fn feedback_id_slash_injection_rejected() {
    let result = uuid::Uuid::parse_str("../../../etc/passwd");
    assert!(
        result.is_err(),
        "path traversal in feedback id must be rejected"
    );
}

#[test]
fn feedback_id_query_injection_rejected() {
    let result = uuid::Uuid::parse_str("00000000-0000-0000-0000-000000000001?x=y");
    assert!(
        result.is_err(),
        "query injection in feedback id must be rejected"
    );
}

// ── Content-Type matching ─────────────────────────────────────────────────

#[test]
fn content_type_matching_is_case_insensitive_and_strips_params() {
    let raw = "Image/PNG; charset=binary";
    let normalised = raw.split(';').next().unwrap().trim().to_ascii_lowercase();
    assert_eq!(normalised, "image/png");
}

// ── parse_probe ───────────────────────────────────────────────────────────

/// A well-formed `/probe` response body. `role`/`source` are JSON literals
/// (`"operator"`, `null`, …) so the helper can build both nip98 and
/// token/disabled shapes.
fn probe_json(auth_mode: &str, role: &str, source: &str, can_act: bool, can_staff: bool) -> String {
    format!(
        r#"{{"status":"ok","authMode":"{auth_mode}","role":{role},"source":{source},"canAct":{can_act},"canStaff":{can_staff}}}"#
    )
}

#[test]
fn parse_probe_operator_nip98() {
    let body = probe_json("nip98", r#""operator""#, r#""config""#, true, true);
    let p = parse_probe("application/json", body.as_bytes()).expect("valid operator probe");
    assert_eq!(p.auth_mode, "nip98");
    assert_eq!(p.role.as_deref(), Some("operator"));
    assert_eq!(p.source.as_deref(), Some("config"));
}

#[test]
fn parse_probe_disabled_has_null_role() {
    let body = probe_json("disabled", "null", "null", false, false);
    let p = parse_probe("application/json", body.as_bytes()).expect("valid disabled probe");
    assert_eq!(p.auth_mode, "disabled");
    assert_eq!(p.role, None);
    assert_eq!(p.source, None);
}

#[test]
fn parse_probe_rejects_non_json_content_type() {
    let body = probe_json("nip98", r#""operator""#, r#""config""#, true, true);
    assert!(parse_probe("text/html", body.as_bytes()).is_none());
    assert!(parse_probe("", body.as_bytes()).is_none());
}

#[test]
fn parse_probe_rejects_missing_required_field() {
    // Missing `canStaff` — an unrelated JSON endpoint must not classify as the
    // admin API.
    let body =
        r#"{"status":"ok","authMode":"nip98","role":"operator","source":"config","canAct":true}"#;
    assert!(parse_probe("application/json", body.as_bytes()).is_none());
}

#[test]
fn parse_probe_rejects_wrong_typed_field() {
    // `canAct` as a string, not a bool.
    let body = r#"{"status":"ok","authMode":"nip98","role":"operator","source":"config","canAct":"yes","canStaff":true}"#;
    assert!(parse_probe("application/json", body.as_bytes()).is_none());
}

#[test]
fn parse_probe_rejects_non_object() {
    assert!(parse_probe("application/json", b"[]").is_none());
    assert!(parse_probe("application/json", b"\"string\"").is_none());
    assert!(parse_probe("application/json", b"not json").is_none());
}

// ── authorized_principal / is_coherent_disabled invariants ────────────────
//
// Structural deserialisation (parse_probe) is necessary but not sufficient:
// a 2xx body must also satisfy the full relay contract before its state is
// trusted. These pin every branch of that invariant.

/// Parse a body known to be structurally valid, then validate it.
fn authorized(body: &str) -> Option<(ProbeRole, ProbeSource)> {
    parse_probe("application/json", body.as_bytes())
        .expect("structurally valid probe")
        .authorized_principal()
}

#[test]
fn authorized_principal_accepts_every_coherent_shape() {
    // Operator (canStaff true) and moderator (canStaff false) across all three
    // recognised sources — each must yield the typed principal.
    let cases: &[(String, ProbeRole, ProbeSource)] = &[
        (
            probe_json("nip98", r#""operator""#, r#""config""#, true, true),
            ProbeRole::Operator,
            ProbeSource::Config,
        ),
        (
            probe_json("nip98", r#""operator""#, r#""owner_fallback""#, true, true),
            ProbeRole::Operator,
            ProbeSource::OwnerFallback,
        ),
        (
            probe_json("nip98", r#""moderator""#, r#""db""#, true, false),
            ProbeRole::Moderator,
            ProbeSource::Db,
        ),
    ];
    for (body, role, source) in cases {
        assert_eq!(authorized(body), Some((*role, *source)));
    }
}

#[test]
fn authorized_principal_rejects_every_incoherent_shape() {
    // Structurally valid probe bodies the relay never emits under nip98;
    // accepting any is fail-open. One invariant broken per row, top to bottom:
    // wrong status, non-nip98 authMode, missing role, unknown role, missing
    // source, unknown source, false canAct, operator lacking canStaff,
    // moderator carrying canStaff.
    let pj = probe_json;
    let cases = [
        pj("nip98", r#""operator""#, r#""config""#, true, true)
            .replace(r#""status":"ok""#, r#""status":"error""#),
        pj("token", "null", "null", false, false),
        pj("nip98", "null", r#""config""#, true, true),
        pj("nip98", r#""superuser""#, r#""config""#, true, true),
        pj("nip98", r#""operator""#, "null", true, true),
        pj("nip98", r#""operator""#, r#""ldap""#, true, true),
        pj("nip98", r#""operator""#, r#""config""#, false, true),
        pj("nip98", r#""operator""#, r#""config""#, true, false),
        pj("nip98", r#""moderator""#, r#""config""#, true, true),
    ];
    for (i, body) in cases.iter().enumerate() {
        assert_eq!(authorized(body), None, "row {i} must be rejected");
    }
}

#[test]
fn is_coherent_disabled_accepts_only_canonical_disabled() {
    // Canonical disabled authorizes; nip98 mode and any disabled body claiming
    // a role/source or a capability is incoherent and must be rejected.
    let disabled = probe_json("disabled", "null", "null", false, false);
    assert!(parse_probe("application/json", disabled.as_bytes())
        .unwrap()
        .is_coherent_disabled());

    let incoherent = [
        probe_json("nip98", r#""operator""#, r#""config""#, true, true),
        probe_json("disabled", r#""operator""#, "null", false, false),
        probe_json("disabled", "null", "null", true, false),
    ];
    for body in &incoherent {
        assert!(!parse_probe("application/json", body.as_bytes())
            .unwrap()
            .is_coherent_disabled());
    }
}

// ── Storage core through production code ─────────────────────────────────
//
// All tests call `get_admin_origin_core` / `set_admin_origin_core` directly
// — the `pub(crate)` functions parameterised by data directory and pubkey
// hex. No `tauri::State` needed; each test uses a `tempdir` for isolation.

#[test]
fn storage_round_trip_returns_canonical_origin() {
    let dir = tempfile::tempdir().unwrap();
    let pubkey = "a".repeat(64);
    let origin = "https://admin.example.com";
    let canonical = set_admin_origin_core(dir.path(), &pubkey, Some(origin.to_string()))
        .unwrap()
        .unwrap();
    assert!(
        canonical.starts_with("https://admin.example.com"),
        "canonical origin must start with the input origin: {canonical}"
    );
    let read_back = get_admin_origin_core(dir.path(), &pubkey).unwrap().unwrap();
    assert_eq!(
        canonical, read_back,
        "read-back must match the canonical form returned by set"
    );
}

#[test]
fn storage_two_identities_are_isolated() {
    let dir = tempfile::tempdir().unwrap();
    let pubkey_a = "a".repeat(64);
    let pubkey_b = "b".repeat(64);
    set_admin_origin_core(
        dir.path(),
        &pubkey_a,
        Some("https://admin-a.example.com".to_string()),
    )
    .unwrap();
    set_admin_origin_core(
        dir.path(),
        &pubkey_b,
        Some("https://admin-b.example.com".to_string()),
    )
    .unwrap();

    let a = get_admin_origin_core(dir.path(), &pubkey_a)
        .unwrap()
        .unwrap();
    let b = get_admin_origin_core(dir.path(), &pubkey_b)
        .unwrap()
        .unwrap();
    assert!(
        a.contains("admin-a"),
        "pubkey_a must read its own origin: {a}"
    );
    assert!(
        b.contains("admin-b"),
        "pubkey_b must read its own origin: {b}"
    );
    // No cross-read: each key sees only its own value.
    assert!(
        !a.contains("admin-b"),
        "pubkey_a must not read pubkey_b's origin"
    );
    assert!(
        !b.contains("admin-a"),
        "pubkey_b must not read pubkey_a's origin"
    );
}

#[test]
fn storage_malformed_json_is_quarantined_and_returns_error() {
    let dir = tempfile::tempdir().unwrap();
    let pubkey = "c".repeat(64);
    // Write a corrupt file directly — bypassing set_admin_origin_core.
    let path = dir
        .path()
        .join(format!("admin-console-origin-{pubkey}.json"));
    std::fs::write(&path, b"not valid json").unwrap();
    assert!(path.exists(), "corrupt file must exist before read");

    let result = get_admin_origin_core(dir.path(), &pubkey);
    assert!(
        result.is_err(),
        "malformed JSON must return Err: {result:?}"
    );
    // Quarantine: the file must have been removed.
    assert!(
        !path.exists(),
        "quarantine failed: corrupt file must be removed after error"
    );
}

#[test]
fn storage_forbidden_path_bearing_origin_is_quarantined_and_returns_error() {
    let dir = tempfile::tempdir().unwrap();
    let pubkey = "d".repeat(64);
    // Write a file whose stored origin contains a path component —
    // AdminOrigin::parse must reject it, triggering quarantine.
    let path = dir
        .path()
        .join(format!("admin-console-origin-{pubkey}.json"));
    let payload = serde_json::json!({ "origin": "https://admin.example.com/forbidden/path" });
    std::fs::write(&path, serde_json::to_vec(&payload).unwrap()).unwrap();
    assert!(path.exists(), "seeded file must exist before read");

    let result = get_admin_origin_core(dir.path(), &pubkey);
    assert!(
        result.is_err(),
        "origin with path must return Err on reparse: {result:?}"
    );
    assert!(
        !path.exists(),
        "quarantine failed: forbidden-origin file must be removed after error"
    );
}

#[test]
fn storage_clear_removes_file() {
    let dir = tempfile::tempdir().unwrap();
    let pubkey = "e".repeat(64);
    set_admin_origin_core(
        dir.path(),
        &pubkey,
        Some("https://admin.example.com".to_string()),
    )
    .unwrap();
    let path = dir
        .path()
        .join(format!("admin-console-origin-{pubkey}.json"));
    assert!(path.exists(), "file must exist after set");

    let result = set_admin_origin_core(dir.path(), &pubkey, None).unwrap();
    assert_eq!(result, None, "clear must return None");
    assert!(!path.exists(), "clear must remove the file");
}

#[test]
fn storage_no_file_returns_none() {
    let dir = tempfile::tempdir().unwrap();
    let pubkey = "f".repeat(64);
    let result = get_admin_origin_core(dir.path(), &pubkey).unwrap();
    assert_eq!(result, None, "absent file must return None");
}

// ── validate_pubkey_hex ───────────────────────────────────────────────────

#[test]
fn pubkey_hex_valid_64_lowercase() {
    assert!(validate_pubkey_hex("a".repeat(64)).is_ok());
}

#[test]
fn pubkey_hex_uppercase_rejected() {
    assert!(validate_pubkey_hex("A".repeat(64)).is_err());
}

#[test]
fn pubkey_hex_empty_rejected() {
    assert!(validate_pubkey_hex("".to_string()).is_err());
}

#[test]
fn pubkey_hex_63_chars_rejected() {
    assert!(validate_pubkey_hex("a".repeat(63)).is_err());
}

// ── Live stub helpers ─────────────────────────────────────────────────────

/// Build a fake Response using a live TCP listener.
async fn fake_response(status: u16, headers: &str, body: &str) -> reqwest::Response {
    use std::io::{Read, Write};
    client::init_admin_client();
    let client = client::ADMIN_CLIENT.get().unwrap();

    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let body_bytes = body.as_bytes().to_vec();
    let body_len = body_bytes.len();
    let response = format!(
        "HTTP/1.1 {status} OK\r\nContent-Length: {body_len}\r\n{headers}Connection: close\r\n\r\n"
    );
    let response_bytes = response.into_bytes();
    std::thread::spawn(move || {
        if let Ok((mut stream, _)) = listener.accept() {
            let mut buf = [0u8; 4096];
            let _ = stream.read(&mut buf);
            let _ = stream.write_all(&response_bytes);
            let _ = stream.write_all(&body_bytes);
            let _ = stream.flush();
        }
    });
    client
        .get(format!("http://{addr}/api/admin/v1/reports"))
        .send()
        .await
        .unwrap()
}

/// Serve sequential HTTP responses from a background thread.
///
/// For each request the listener reads the raw HTTP bytes, calls the
/// provided inspector closure with the raw request bytes and slot index,
/// then sends the pre-configured response. The inspector records request
/// details post-hoc for assertion after the probe completes.
async fn serve_sequence_inspect(
    responses: Vec<(&'static str, &'static str, &'static str)>,
    inspect: Option<RequestInspector>,
) -> std::net::SocketAddr {
    use std::io::{Read, Write};
    client::init_admin_client();
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    std::thread::spawn(move || {
        for (idx, (status, headers, body)) in responses.into_iter().enumerate() {
            if let Ok((mut stream, _)) = listener.accept() {
                let mut buf = [0u8; 8192];
                let n = stream.read(&mut buf).unwrap_or(0);
                // Invoke the inspector with the raw request bytes.
                if let Some(ref f) = inspect {
                    f(idx, &buf[..n]);
                }
                let body_bytes = body.as_bytes();
                let response = format!(
                    "HTTP/1.1 {status}\r\nContent-Length: {}\r\n{headers}Connection: close\r\n\r\n",
                    body_bytes.len()
                );
                let _ = stream.write_all(response.as_bytes());
                let _ = stream.write_all(body_bytes);
                let _ = stream.flush();
            }
        }
    });
    addr
}

/// Serve sequential responses without request inspection (backward compat).
async fn serve_sequence(
    responses: Vec<(&'static str, &'static str, &'static str)>,
) -> std::net::SocketAddr {
    serve_sequence_inspect(responses, None).await
}

/// Serve a two-slot NIP-98 stub where the second response is gated on the
/// received Authorization header matching `expected_token`.
///
/// Slot 0: always 401 Unauthorized + `WWW-Authenticate: Nostr` (triggers retry).
/// Slot 1: 200 OK with JSON body if the received Authorization header equals
///         `expected_token`; plain 401 (no Nostr challenge) otherwise — a mismatch
///         means the production header call was missing, so the probe returns
///         Nip98Denied and the caller's `Nip98Authorized` assertion fails.
///
/// Both slots are recorded in the returned `Arc<Mutex<Vec<RequestRecord>>>`.
async fn serve_gated_nip98(
    expected_token: String,
    authorized_body: &'static str,
) -> (
    std::net::SocketAddr,
    Arc<std::sync::Mutex<Vec<RequestRecord>>>,
) {
    use std::io::{Read, Write};
    client::init_admin_client();
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let records: Arc<std::sync::Mutex<Vec<RequestRecord>>> =
        Arc::new(std::sync::Mutex::new(Vec::new()));
    let records_bg = Arc::clone(&records);
    std::thread::spawn(move || {
        for slot in 0..2usize {
            if let Ok((mut stream, _)) = listener.accept() {
                let mut buf = [0u8; 8192];
                let n = stream.read(&mut buf).unwrap_or(0);
                let text = std::str::from_utf8(&buf[..n]).unwrap_or("");
                // Parse request line and Authorization header.
                let first_line = text.lines().next().unwrap_or("");
                let mut parts = first_line.splitn(3, ' ');
                let method = parts.next().unwrap_or("").to_string();
                let path = parts.next().unwrap_or("").to_string();
                let auth = text
                    .lines()
                    .find(|l| l.to_ascii_lowercase().starts_with("authorization:"))
                    .map(|l| l[l.find(':').unwrap() + 1..].trim().to_string());
                records_bg.lock().unwrap().push(RequestRecord {
                    method,
                    path,
                    auth: auth.clone(),
                });
                // Gate: slot 0 always challenges; slot 1 returns 200 only on
                // header match, 401 (no challenge) otherwise.
                let (status, headers, body): (&str, &str, &str) = if slot == 0 {
                    ("401 Unauthorized", "WWW-Authenticate: Nostr\r\n", "")
                } else if auth.as_deref() == Some(expected_token.as_str()) {
                    (
                        "200 OK",
                        "Content-Type: application/json\r\n",
                        authorized_body,
                    )
                } else {
                    // Mismatch or absent header → plain 401 (no Nostr challenge).
                    // admin_probe_inner sees a non-Nostr 401 after the retry and
                    // returns Nip98Denied, causing the caller's Nip98Authorized
                    // assertion to fail — which is the intended mutation catch.
                    ("401 Unauthorized", "", "")
                };
                let resp = format!(
                    "HTTP/1.1 {status}\r\nContent-Length: {}\r\n{headers}Connection: close\r\n\r\n",
                    body.len()
                );
                let _ = stream.write_all(resp.as_bytes());
                let _ = stream.write_all(body.as_bytes());
                let _ = stream.flush();
            }
        }
    });
    (addr, records)
}

// ── is_probe_response_intercepted ────────────────────────────────────────

#[tokio::test]
async fn probe_html_200_classified_as_intercepted() {
    let resp = fake_response(
        200,
        "Content-Type: text/html; charset=utf-8\r\n",
        "<html><body>Sign in</body></html>",
    )
    .await;
    assert!(is_probe_response_intercepted(&resp));
}

#[tokio::test]
async fn probe_json_200_not_classified_as_intercepted() {
    let resp = fake_response(
        200,
        "Content-Type: application/json\r\n",
        r#"{"status":"ok","authMode":"disabled","role":null,"source":null,"canAct":false,"canStaff":false}"#,
    )
    .await;
    assert!(!is_probe_response_intercepted(&resp));
}

#[tokio::test]
async fn probe_json_200_with_valid_probe_parses() {
    let body = probe_json("nip98", r#""operator""#, r#""config""#, true, true);
    let resp = fake_response(200, "Content-Type: application/json\r\n", &body).await;
    assert!(!is_probe_response_intercepted(&resp));
    let ct = response_content_type(&resp);
    let bytes = read_bounded(resp, PROBE_JSON_CAP).await.unwrap();
    assert!(parse_probe(&ct, &bytes).is_some());
}

#[tokio::test]
async fn probe_json_200_bare_garbage_not_admin_api() {
    let resp = fake_response(200, "Content-Type: application/json\r\n", "[1,2,3]").await;
    let ct = response_content_type(&resp);
    let bytes = read_bounded(resp, PROBE_JSON_CAP).await.unwrap();
    assert!(parse_probe(&ct, &bytes).is_none());
}

// ── admin_probe_inner end-to-end state machine ────────────────────────────

#[tokio::test]
async fn probe_inner_html_200_is_network_or_intercepted() {
    let addr = serve_sequence(vec![(
        "200 OK",
        "Content-Type: text/html\r\n",
        "<html>sign in</html>",
    )])
    .await;
    let result = admin_probe_inner(
        &format!("http://{addr}"),
        None::<fn(&str) -> Result<String, String>>,
    )
    .await
    .unwrap();
    assert!(matches!(result, AdminProbeResult::NetworkOrIntercepted));
}

#[tokio::test]
async fn probe_inner_malformed_json_200_is_not_admin_api() {
    let addr = serve_sequence(vec![(
        "200 OK",
        "Content-Type: application/json\r\n",
        "not valid json",
    )])
    .await;
    let result = admin_probe_inner(
        &format!("http://{addr}"),
        None::<fn(&str) -> Result<String, String>>,
    )
    .await
    .unwrap();
    assert!(matches!(result, AdminProbeResult::NotAdminApi));
}

#[tokio::test]
async fn probe_inner_disabled_probe_200_is_disabled() {
    let body = probe_json("disabled", "null", "null", false, false);
    let body_static: &'static str = Box::leak(body.into_boxed_str());
    let addr = serve_sequence(vec![(
        "200 OK",
        "Content-Type: application/json\r\n",
        body_static,
    )])
    .await;
    let result = admin_probe_inner(
        &format!("http://{addr}"),
        None::<fn(&str) -> Result<String, String>>,
    )
    .await
    .unwrap();
    assert!(matches!(result, AdminProbeResult::Disabled));
}

#[tokio::test]
async fn probe_inner_nip98_authmode_200_without_auth_is_not_admin_api() {
    // A relay must 401 an unauthenticated caller in nip98/token mode. A 200
    // carrying `authMode: "nip98"` (no 401 challenge) is a contract violation
    // and must not be classified as Disabled.
    let body = probe_json("nip98", r#""operator""#, r#""config""#, true, true);
    let body_static: &'static str = Box::leak(body.into_boxed_str());
    let addr = serve_sequence(vec![(
        "200 OK",
        "Content-Type: application/json\r\n",
        body_static,
    )])
    .await;
    let result = admin_probe_inner(
        &format!("http://{addr}"),
        None::<fn(&str) -> Result<String, String>>,
    )
    .await
    .unwrap();
    assert!(matches!(result, AdminProbeResult::NotAdminApi));
}

#[tokio::test]
async fn probe_inner_bare_garbage_200_is_not_admin_api() {
    // A JSON body that isn't a probe envelope must not classify as admin API.
    let addr = serve_sequence(vec![(
        "200 OK",
        "Content-Type: application/json\r\n",
        "[1,2,3]",
    )])
    .await;
    let result = admin_probe_inner(
        &format!("http://{addr}"),
        None::<fn(&str) -> Result<String, String>>,
    )
    .await
    .unwrap();
    assert!(matches!(result, AdminProbeResult::NotAdminApi));
}

#[tokio::test]
async fn probe_inner_persistent_401_is_nip98_denied() {
    let addr = serve_sequence(vec![
        ("401 Unauthorized", "WWW-Authenticate: Nostr\r\n", ""),
        ("401 Unauthorized", "", ""),
    ])
    .await;
    let sign = |_url: &str| -> Result<String, String> { Ok("Nostr dGVzdA==".to_string()) };
    let result = admin_probe_inner(&format!("http://{addr}"), Some(sign))
        .await
        .unwrap();
    assert!(matches!(result, AdminProbeResult::Nip98Denied));
}

#[tokio::test]
async fn probe_inner_nip98_challenge_then_json_200_is_authorized_and_asserts_auth_header() {
    // Verifies:
    //  1. probe state machine produces Nip98Authorized on a Nostr 401→200 sequence.
    //  2. The second request carries an Authorization header equal to the signing
    //     closure's token — tested by the gated stub: slot 1 returns 200 only
    //     when the received Authorization header matches the expected token; any
    //     mismatch or absent header returns a plain 401, making the state machine
    //     return Nip98Denied and failing the Nip98Authorized assertion.
    //  3. The first request carries no Authorization header.
    //  4. Deleting the `.header(AUTHORIZATION, …)` production line causes the
    //     stub to receive no header on slot 1, return 401, and the test fails.

    let expected_token = "Nostr dGVzdA==".to_string();
    let expected_token_for_sign = expected_token.clone();

    let valid_body = probe_json("nip98", r#""operator""#, r#""config""#, true, true);
    let valid_body_static: &'static str = Box::leak(valid_body.into_boxed_str());

    // serve_gated_nip98: slot 0 always challenges; slot 1 checks the Authorization
    // header and returns 200 on match, 401 on mismatch/absent.
    let (addr, records) = serve_gated_nip98(expected_token, valid_body_static).await;

    let sign = move |_url: &str| -> Result<String, String> { Ok(expected_token_for_sign.clone()) };
    let result = admin_probe_inner(&format!("http://{addr}"), Some(sign))
        .await
        .unwrap();

    // The relay-resolved role/source must be carried through to the UI so the
    // Staffing tab renders for an operator.
    assert!(
        matches!(
            &result,
            AdminProbeResult::Nip98Authorized { role, source }
                if role.as_deref() == Some("operator") && source.as_deref() == Some("config")
        ),
        "expected Nip98Authorized operator/config, got {result:?}"
    );

    let records = records.lock().unwrap();
    assert_eq!(records.len(), 2, "exactly two requests must have been made");

    // Request 0: unauthenticated GET — no Authorization header.
    assert_eq!(
        records[0].method, "GET",
        "slot-0 must be GET; got {:?}",
        records[0].method
    );
    assert!(
        records[0].path.contains("/api/admin/v1/probe"),
        "slot-0 must target the probe endpoint; got {:?}",
        records[0].path
    );
    assert!(
        records[0].auth.is_none(),
        "slot-0 must carry no Authorization; got {:?}",
        records[0].auth
    );

    // Request 1: authenticated retry — Authorization must equal the signing token.
    // The stub already enforced this (returned 200 only on match), so this
    // post-hoc assertion documents the observed value for auditability.
    assert_eq!(
        records[1].method, "GET",
        "slot-1 must be GET; got {:?}",
        records[1].method
    );
    assert!(
        records[1].path.contains("/api/admin/v1/probe"),
        "slot-1 must target the probe endpoint; got {:?}",
        records[1].path
    );
    assert_eq!(
        records[1].auth.as_deref(),
        Some("Nostr dGVzdA=="),
        "slot-1 Authorization must equal the signing closure token"
    );
}

#[tokio::test]
async fn probe_inner_missing_auth_header_fails_to_authorize() {
    // Verifies the no-sign path: when no signing closure is provided and the
    // server issues a Nostr challenge, admin_probe_inner returns Nip98Denied.
    // The production code only calls `sign(url)?` when a signing closure is
    // Some; passing None causes the signing step to be skipped entirely, so
    // no Authorization header is attached and the probe returns Nip98Denied
    // without making a second request.
    let addr = serve_sequence(vec![(
        "401 Unauthorized",
        "WWW-Authenticate: Nostr\r\n",
        "",
    )])
    .await;
    let result = admin_probe_inner(
        &format!("http://{addr}"),
        None::<fn(&str) -> Result<String, String>>,
    )
    .await
    .unwrap();
    assert!(matches!(result, AdminProbeResult::Nip98Denied));
}

#[tokio::test]
async fn probe_inner_authenticated_302_is_network_or_intercepted() {
    let addr = serve_sequence(vec![
        ("401 Unauthorized", "WWW-Authenticate: Nostr\r\n", ""),
        (
            "302 Found",
            "Location: https://cloudflareaccess.com/\r\n",
            "",
        ),
    ])
    .await;
    let sign = |_url: &str| -> Result<String, String> { Ok("Nostr dGVzdA==".to_string()) };
    let result = admin_probe_inner(&format!("http://{addr}"), Some(sign))
        .await
        .unwrap();
    assert!(
        matches!(result, AdminProbeResult::NetworkOrIntercepted),
        "authenticated 302 must be NetworkOrIntercepted, got {result:?}"
    );
}

#[tokio::test]
async fn probe_inner_bearer_401_is_token_mode() {
    let addr = serve_sequence(vec![(
        "401 Unauthorized",
        "WWW-Authenticate: Bearer realm=\"admin\"\r\n",
        "",
    )])
    .await;
    let result = admin_probe_inner(
        &format!("http://{addr}"),
        None::<fn(&str) -> Result<String, String>>,
    )
    .await
    .unwrap();
    assert!(matches!(result, AdminProbeResult::TokenMode));
}

#[tokio::test]
async fn probe_inner_no_sign_on_nostr_challenge_is_nip98_denied() {
    let addr = serve_sequence(vec![(
        "401 Unauthorized",
        "WWW-Authenticate: Nostr\r\n",
        "",
    )])
    .await;
    let result = admin_probe_inner(
        &format!("http://{addr}"),
        None::<fn(&str) -> Result<String, String>>,
    )
    .await
    .unwrap();
    assert!(matches!(result, AdminProbeResult::Nip98Denied));
}

// ── NIP-11 admin-origin discovery ─────────────────────────────────────────

// Pure-function tests for admin_origin_from_nip11 — a thin wrapper over
// AdminOrigin::parse (whose rules are exhaustively covered in origin.rs).
// These pin only the wrapper's own three outcomes.

#[test]
fn discover_parse_accepts_valid_value() {
    let info = AdminApiInfo {
        admin_api: Some("http://127.0.0.1:3000".to_string()),
    };
    assert_eq!(
        admin_origin_from_nip11(&info).as_deref(),
        Some("http://127.0.0.1:3000")
    );
}

#[test]
fn discover_parse_none_when_absent() {
    let info = AdminApiInfo { admin_api: None };
    assert_eq!(admin_origin_from_nip11(&info), None);
}

#[test]
fn discover_parse_rejects_invalid_value() {
    // A non-loopback http origin is rejected by AdminOrigin::parse — the
    // untrusted advertised value must not become a probe target.
    let info = AdminApiInfo {
        admin_api: Some("http://admin.example.com".to_string()),
    };
    assert_eq!(admin_origin_from_nip11(&info), None);
}

#[tokio::test]
async fn discover_returns_origin_when_admin_api_advertised() {
    // The relay's NIP-11 doc advertises a valid loopback admin_api → the
    // canonical origin is returned so the desktop can auto-probe it.
    let body =
        r#"{"name":"Buzz Relay","supported_nips":[1,11],"admin_api":"http://127.0.0.1:3000"}"#;
    let addr = serve_sequence(vec![(
        "200 OK",
        "Content-Type: application/nostr+json\r\n",
        body,
    )])
    .await;
    let client = reqwest::Client::new();
    let result = discover_admin_origin_at(&client, &format!("http://{addr}"))
        .await
        .unwrap();
    assert_eq!(result.as_deref(), Some("http://127.0.0.1:3000"));
}

#[tokio::test]
async fn discover_returns_none_when_admin_api_absent() {
    // A relay that does not configure an admin surface omits admin_api → the
    // desktop falls back to manual entry (Ok(None), never an error).
    let body = r#"{"name":"Buzz Relay","supported_nips":[1,11]}"#;
    let addr = serve_sequence(vec![(
        "200 OK",
        "Content-Type: application/nostr+json\r\n",
        body,
    )])
    .await;
    let client = reqwest::Client::new();
    let result = discover_admin_origin_at(&client, &format!("http://{addr}"))
        .await
        .unwrap();
    assert_eq!(result, None);
}

#[tokio::test]
async fn discover_returns_none_when_advertised_value_invalid() {
    // The relay advertises an invalid admin_api (non-loopback http). It is
    // rejected by AdminOrigin::parse; discovery returns Ok(None) — no crash,
    // no probe of the insecure endpoint.
    let body = r#"{"admin_api":"http://admin.example.com"}"#;
    let addr = serve_sequence(vec![(
        "200 OK",
        "Content-Type: application/nostr+json\r\n",
        body,
    )])
    .await;
    let client = reqwest::Client::new();
    let result = discover_admin_origin_at(&client, &format!("http://{addr}"))
        .await
        .unwrap();
    assert_eq!(result, None);
}

#[tokio::test]
async fn discover_errors_on_non_2xx() {
    // A relay error (5xx) is a transport-level failure, not "no admin_api" —
    // it must surface as Err rather than silently returning None.
    let addr = serve_sequence(vec![("500 Internal Server Error", "", "")]).await;
    let client = reqwest::Client::new();
    let result = discover_admin_origin_at(&client, &format!("http://{addr}")).await;
    assert!(
        result.is_err(),
        "non-2xx must surface as Err; got {result:?}"
    );
}

#[tokio::test]
async fn discover_requests_info_path_with_nostr_accept_header() {
    // The discovery fetch must hit `/info` with the NIP-11 Accept header —
    // the same contract as relay_requires_membership.
    let captured: Arc<std::sync::Mutex<Vec<u8>>> = Arc::new(std::sync::Mutex::new(Vec::new()));
    let captured_bg = Arc::clone(&captured);
    let body = r#"{"admin_api":"http://127.0.0.1:3000"}"#;
    let addr = serve_sequence_inspect(
        vec![("200 OK", "Content-Type: application/nostr+json\r\n", body)],
        Some(Arc::new(move |_idx, bytes: &[u8]| {
            captured_bg.lock().unwrap().extend_from_slice(bytes);
        })),
    )
    .await;
    let client = reqwest::Client::new();
    let _ = discover_admin_origin_at(&client, &format!("http://{addr}"))
        .await
        .unwrap();
    let request = String::from_utf8_lossy(&captured.lock().unwrap()).to_string();
    assert!(
        request.starts_with("GET /info "),
        "discovery must GET /info; got: {:?}",
        request.lines().next()
    );
    assert!(
        request
            .to_ascii_lowercase()
            .contains("accept: application/nostr+json"),
        "discovery must send the NIP-11 Accept header"
    );
}
