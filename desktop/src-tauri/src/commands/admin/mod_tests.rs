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
fn parse_probe_rejects_invalid_inputs() {
    // Table of structural rejection cases. Each row is (content_type, body_bytes, label).
    // Non-JSON content type, missing required field, wrong-typed field, and non-object bodies
    // must all return None — an unrelated endpoint must not classify as the admin API.
    let well_formed = probe_json("nip98", r#""operator""#, r#""config""#, true, true);
    let cases: &[(&str, &[u8], &str)] = &[
        ("text/html",       well_formed.as_bytes(), "non-JSON content type (text/html)"),
        ("",                well_formed.as_bytes(), "non-JSON content type (empty)"),
        // Missing `canStaff` — an unrelated JSON endpoint must not classify as admin API.
        ("application/json",
         br#"{"status":"ok","authMode":"nip98","role":"operator","source":"config","canAct":true}"#,
         "missing required field canStaff"),
        // `canAct` as a string, not a bool.
        ("application/json",
         br#"{"status":"ok","authMode":"nip98","role":"operator","source":"config","canAct":"yes","canStaff":true}"#,
         "wrong-typed field canAct"),
        ("application/json", b"[]",         "non-object: array"),
        ("application/json", b"\"string\"", "non-object: string"),
        ("application/json", b"not json",   "non-object: invalid JSON"),
    ];
    for (ct, body, label) in cases {
        assert!(
            parse_probe(ct, body).is_none(),
            "row must be rejected: {label}"
        );
    }
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
// — the `pub(crate)` functions parameterised by data directory, pubkey
// hex, and relay slug. No `tauri::State` needed; each test uses a `tempdir`
// for isolation.

const TEST_RELAY: &str = "relay.example.com";

/// App state whose active relay is `url`, exercising the production
/// relay-host scoping of the origin file.
fn relay_state(url: &str) -> crate::app_state::AppState {
    let state = crate::app_state::build_app_state();
    *state.relay_url_override.lock().unwrap() = Some(url.to_string());
    state
}

fn test_relay() -> crate::app_state::AppState {
    relay_state("wss://relay.example.com")
}

#[test]
fn storage_round_trip_returns_canonical_origin() {
    let dir = tempfile::tempdir().unwrap();
    let pubkey = "a".repeat(64);
    let origin = "https://admin.example.com";
    let canonical =
        set_admin_origin_core(dir.path(), &pubkey, &test_relay(), Some(origin.to_string()))
            .unwrap()
            .unwrap();
    assert!(
        canonical.starts_with("https://admin.example.com"),
        "canonical origin must start with the input origin: {canonical}"
    );
    let read_back = get_admin_origin_core(dir.path(), &pubkey, &test_relay())
        .unwrap()
        .unwrap();
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
        &test_relay(),
        Some("https://admin-a.example.com".to_string()),
    )
    .unwrap();
    set_admin_origin_core(
        dir.path(),
        &pubkey_b,
        &test_relay(),
        Some("https://admin-b.example.com".to_string()),
    )
    .unwrap();

    let a = get_admin_origin_core(dir.path(), &pubkey_a, &test_relay())
        .unwrap()
        .unwrap();
    let b = get_admin_origin_core(dir.path(), &pubkey_b, &test_relay())
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
fn storage_two_relays_are_isolated() {
    // Same pubkey — different relay slugs must not share storage.
    let dir = tempfile::tempdir().unwrap();
    let pubkey = "a".repeat(64);
    set_admin_origin_core(
        dir.path(),
        &pubkey,
        &relay_state("wss://prod.example.com"),
        Some("https://admin-prod.example.com".to_string()),
    )
    .unwrap();
    set_admin_origin_core(
        dir.path(),
        &pubkey,
        &relay_state("wss://staging.example.com:8443"),
        Some("https://admin-staging.example.com".to_string()),
    )
    .unwrap();

    let prod = get_admin_origin_core(
        dir.path(),
        &pubkey,
        &relay_state("https://prod.example.com"),
    )
    .unwrap()
    .unwrap();
    let staging = get_admin_origin_core(
        dir.path(),
        &pubkey,
        &relay_state("wss://Staging.Example.com/ws"),
    )
    .unwrap()
    .unwrap();
    assert!(
        prod.contains("admin-prod"),
        "prod relay must read its own origin: {prod}"
    );
    assert!(
        staging.contains("admin-staging"),
        "staging relay must read its own origin: {staging}"
    );
    assert!(
        !prod.contains("staging"),
        "prod relay must not read staging origin"
    );
    assert!(
        !staging.contains("prod"),
        "staging relay must not read prod origin"
    );
    // prod's file must not exist under the staging slug.
    let absent =
        get_admin_origin_core(dir.path(), &pubkey, &relay_state("wss://other.example.com"))
            .unwrap();
    assert_eq!(absent, None, "unknown relay slug must return None");
}

#[test]
fn storage_malformed_json_is_quarantined_and_returns_error() {
    let dir = tempfile::tempdir().unwrap();
    let pubkey = "c".repeat(64);
    // Write a corrupt file directly — bypassing set_admin_origin_core.
    let path = dir
        .path()
        .join(format!("admin-console-origin-{pubkey}-{TEST_RELAY}.json"));
    std::fs::write(&path, b"not valid json").unwrap();
    assert!(path.exists(), "corrupt file must exist before read");

    let result = get_admin_origin_core(dir.path(), &pubkey, &test_relay());
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
        .join(format!("admin-console-origin-{pubkey}-{TEST_RELAY}.json"));
    let payload = serde_json::json!({ "origin": "https://admin.example.com/forbidden/path" });
    std::fs::write(&path, serde_json::to_vec(&payload).unwrap()).unwrap();
    assert!(path.exists(), "seeded file must exist before read");

    let result = get_admin_origin_core(dir.path(), &pubkey, &test_relay());
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
        &test_relay(),
        Some("https://admin.example.com".to_string()),
    )
    .unwrap();
    let path = dir
        .path()
        .join(format!("admin-console-origin-{pubkey}-{TEST_RELAY}.json"));
    assert!(path.exists(), "file must exist after set");

    let result = set_admin_origin_core(dir.path(), &pubkey, &test_relay(), None).unwrap();
    assert_eq!(result, None, "clear must return None");
    assert!(!path.exists(), "clear must remove the file");
}

#[test]
fn storage_no_file_returns_none() {
    let dir = tempfile::tempdir().unwrap();
    let pubkey = "f".repeat(64);
    let result = get_admin_origin_core(dir.path(), &pubkey, &test_relay()).unwrap();
    assert_eq!(result, None, "absent file must return None");
}

// ── validate_pubkey_hex ───────────────────────────────────────────────────

#[test]
fn validate_pubkey_hex_cases() {
    // Direct assertions: valid input passes; uppercase, empty, and wrong-length inputs fail.
    assert!(
        validate_pubkey_hex("a".repeat(64)).is_ok(),
        "64 lowercase hex chars must pass"
    );
    assert!(
        validate_pubkey_hex("A".repeat(64)).is_err(),
        "uppercase hex must be rejected"
    );
    assert!(
        validate_pubkey_hex("".to_string()).is_err(),
        "empty string must be rejected"
    );
    assert!(
        validate_pubkey_hex("a".repeat(63)).is_err(),
        "63 chars must be rejected"
    );
}

// ── relay_host_slug ───────────────────────────────────────────────────────

fn slug(url: &str) -> String {
    relay_host_slug(&relay_state(url))
}

#[test]
fn relay_host_slug_strips_scheme_port_and_path() {
    assert_eq!(slug("wss://relay.example.com"), "relay.example.com");
    assert_eq!(slug("wss://relay.example.com:443/ws"), "relay.example.com");
    assert_eq!(slug("ws://staging.example.com:8080"), "staging.example.com");
}

#[test]
fn relay_host_slug_normalises_to_lowercase() {
    assert_eq!(slug("wss://Relay.EXAMPLE.COM"), "relay.example.com");
}

#[test]
fn relay_host_slug_two_relays_produce_distinct_slugs() {
    assert_ne!(
        slug("wss://relay.prod.example.com"),
        slug("wss://relay.staging.example.com")
    );
}

#[test]
fn relay_host_slug_keeps_distinct_ipv6_relays_apart() {
    // The old `split(':')` port-drop kept only `[2001` for both.
    assert_ne!(
        slug("wss://[2001:db8::1]"),
        slug("wss://[2001:db8::2]:8443")
    );
    assert_eq!(slug("wss://[2001:db8::1]"), "ip6_2001-db8-0-0-0-0-0-1");
}

#[test]
fn relay_host_slug_ipv6_never_matches_a_dns_slug() {
    // A DNS host spelled like the IPv6 slug still maps elsewhere.
    assert_ne!(
        slug("wss://[2001:db8::1]"),
        slug("wss://ip6-2001-db8-0-0-0-0-0-1.example")
    );
    assert!(!slug("wss://ip6_2001-db8-0-0-0-0-0-1").starts_with("ip6_"));
}

#[test]
fn storage_ipv6_relays_are_isolated_and_spelling_independent() {
    let dir = tempfile::tempdir().unwrap();
    let pubkey = "a".repeat(64);
    set_admin_origin_core(
        dir.path(),
        &pubkey,
        &relay_state("wss://[2001:db8::1]"),
        Some("https://admin-a.example.com".to_string()),
    )
    .unwrap();

    // Same first group, different address: nothing saved for it.
    assert_eq!(
        get_admin_origin_core(dir.path(), &pubkey, &relay_state("wss://[2001:db8::2]")).unwrap(),
        None
    );
    // Expanded, upper-case, with port: the same address reads the same file.
    let same = get_admin_origin_core(
        dir.path(),
        &pubkey,
        &relay_state("wss://[2001:0DB8:0:0:0:0:0:1]:8443/ws"),
    )
    .unwrap()
    .unwrap();
    assert!(same.contains("admin-a"), "{same}");

    // Clearing through the equivalent spelling removes that one file.
    set_admin_origin_core(
        dir.path(),
        &pubkey,
        &relay_state("wss://[2001:db8:0::1]"),
        None,
    )
    .unwrap();
    assert_eq!(
        get_admin_origin_core(dir.path(), &pubkey, &relay_state("wss://[2001:db8::1]")).unwrap(),
        None
    );
}

#[test]
fn storage_ignores_a_file_saved_under_the_old_ambiguous_ipv6_slug() {
    let dir = tempfile::tempdir().unwrap();
    let pubkey = "a".repeat(64);
    std::fs::write(
        dir.path()
            .join(format!("admin-console-origin-{pubkey}-2001.json")),
        r#"{"origin":"https://admin-other.example.com"}"#,
    )
    .unwrap();
    assert_eq!(
        get_admin_origin_core(dir.path(), &pubkey, &relay_state("wss://[2001:db8::1]")).unwrap(),
        None
    );
}

// ── Live stub helpers ─────────────────────────────────────────────────────

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
    client::init_admin_client().expect("client builds");
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
    client::init_admin_client().expect("client builds");
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

// ── admin_probe_inner end-to-end state machine ────────────────────────────

#[tokio::test]
async fn probe_inner_simple_response_classifications() {
    // Table of single-response probe outcomes. Each row: (status, content_type,
    // body, expected_result_label). Multi-step sequences (persistent-401,
    // NIP-98 challenge/authorized) stay as dedicated tests below.
    //
    // Rows:
    //   html-200:              text/html intercept page → NetworkOrIntercepted
    //   malformed-json-200:    application/json malformed body → NotAdminApi
    //   disabled-200:          canonical disabled probe → Disabled
    //   nip98-200-no-auth:     nip98 authMode without 401 challenge is a contract
    //                          violation → NotAdminApi (must not classify as Disabled)
    //   garbage-json-200:      valid JSON but not a probe envelope → NotAdminApi

    let disabled_body = probe_json("disabled", "null", "null", false, false);
    let disabled_static: &'static str = Box::leak(disabled_body.into_boxed_str());
    let nip98_body = probe_json("nip98", r#""operator""#, r#""config""#, true, true);
    let nip98_static: &'static str = Box::leak(nip98_body.into_boxed_str());

    struct Row {
        label: &'static str,
        status: &'static str,
        ct: &'static str,
        body: &'static str,
        is_match: fn(&AdminProbeResult) -> bool,
    }

    let rows = [
        Row {
            label: "html-200 → NetworkOrIntercepted",
            status: "200 OK",
            ct: "Content-Type: text/html; charset=utf-8\r\n",
            body: "<html>sign in</html>",
            is_match: |r| matches!(r, AdminProbeResult::NetworkOrIntercepted),
        },
        Row {
            label: "malformed-json-200 → NotAdminApi",
            status: "200 OK",
            ct: "Content-Type: application/json\r\n",
            body: "not valid json",
            is_match: |r| matches!(r, AdminProbeResult::NotAdminApi),
        },
        Row {
            label: "disabled-200 → Disabled",
            status: "200 OK",
            ct: "Content-Type: application/json\r\n",
            body: disabled_static,
            is_match: |r| matches!(r, AdminProbeResult::Disabled),
        },
        Row {
            label: "nip98-authmode-200-without-auth → NotAdminApi",
            status: "200 OK",
            ct: "Content-Type: application/json\r\n",
            body: nip98_static,
            is_match: |r| matches!(r, AdminProbeResult::NotAdminApi),
        },
        Row {
            label: "garbage-json-200 → NotAdminApi",
            status: "200 OK",
            ct: "Content-Type: application/json\r\n",
            body: "[1,2,3]",
            is_match: |r| matches!(r, AdminProbeResult::NotAdminApi),
        },
    ];

    for row in &rows {
        let addr = serve_sequence(vec![(row.status, row.ct, row.body)]).await;
        let result = admin_probe_inner(
            &format!("http://{addr}"),
            None::<fn(&str) -> Result<String, String>>,
        )
        .await
        .unwrap();
        assert!(
            (row.is_match)(&result),
            "row must match expected classification: {}",
            row.label
        );
    }
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
async fn probe_inner_bearer_401_is_not_admin_api() {
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
    // Bearer is no longer a recognized Buzz admin mode; an unrecognized 401
    // challenge classifies as NotAdminApi.
    assert!(matches!(result, AdminProbeResult::NotAdminApi));
}

// ── .localhost origin: end-to-end parse, route, connect ──────────────────────

/// Verifies that `http://admin.localhost:<port>` is accepted as a valid origin,
/// that the signing closure receives the preserved `admin.localhost` probe URL
/// (not rewritten to `127.0.0.1`), and that `ADMIN_CLIENT` actually routes
/// both requests in a NIP-98 sequence to the loopback listener.
///
/// Sequence: slot-0 = `401 Unauthorized + WWW-Authenticate: Nostr` (forces the
/// sign closure to fire), slot-1 = `200 OK + authorized nip98 JSON`. This
/// exercises the full `admin_probe_inner` NIP-98 path via the production
/// `ADMIN_CLIENT` that carries `LocalhostDnsResolver`.
///
/// Mutation evidence:
/// - Removing `.ends_with(".localhost")` from `is_loopback_host` in `origin.rs`
///   makes `AdminOrigin::parse` return `Err`; `admin_probe_inner` propagates
///   that as `Err` and the `.expect()` panics → RED before any network I/O.
/// - Removing the `.ends_with(".localhost")` branch from `LocalhostDnsResolver`
///   makes the connection time out on Linux/Windows CI (system GAI fails) →
///   `NetworkOrIntercepted`, not `Nip98Authorized` → the `matches!` assertion RED.
#[tokio::test]
async fn dot_localhost_origin_parses_and_probe_inner_reaches_loopback_via_nip98() {
    use std::sync::{Arc, Mutex};

    client::init_admin_client().expect("client builds");

    // Serve: slot-0 = 401 Nostr challenge, slot-1 = 200 authorized nip98 response.
    let probe_body = probe_json("nip98", "\"operator\"", "\"db\"", true, true);
    let probe_body_static: &'static str = Box::leak(probe_body.into_boxed_str());

    // Use serve_sequence_inspect to capture raw request bytes for Host assertions.
    let captured: Arc<Mutex<Vec<Vec<u8>>>> = Arc::new(Mutex::new(Vec::new()));
    let cap = Arc::clone(&captured);

    let addr = serve_sequence_inspect(
        vec![
            ("401 Unauthorized", "WWW-Authenticate: Nostr\r\n", ""),
            (
                "200 OK",
                "Content-Type: application/json\r\n",
                probe_body_static,
            ),
        ],
        Some(Arc::new(move |_idx, bytes: &[u8]| {
            cap.lock().unwrap().push(bytes.to_vec());
        })),
    )
    .await;

    let port = addr.port();

    // Capture the exact URL the signing closure receives — the production seam.
    let signed_urls: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let signed_urls_clone = Arc::clone(&signed_urls);
    let sign = move |url: &str| -> Result<String, String> {
        signed_urls_clone.lock().unwrap().push(url.to_owned());
        Ok("Nostr dGVzdA==".to_string())
    };

    let origin_str = format!("http://admin.localhost:{port}");
    let result = admin_probe_inner(&origin_str, Some(sign))
        .await
        .expect("admin_probe_inner must succeed on a valid admin.localhost origin");

    // The authorized 200 probe response must resolve to Nip98Authorized.
    assert!(
        matches!(result, AdminProbeResult::Nip98Authorized { .. }),
        "admin.localhost NIP-98 probe must resolve to Nip98Authorized; got {result:?}",
    );

    // The sign closure must have been called exactly once (on the Nostr challenge).
    let urls = signed_urls.lock().unwrap();
    assert_eq!(
        urls.len(),
        1,
        "sign closure must be called exactly once; got {} calls",
        urls.len(),
    );

    // The URL received by the signing closure must preserve `admin.localhost` — not 127.0.0.1.
    assert_eq!(
        urls[0],
        format!("http://admin.localhost:{port}/api/admin/v1/probe"),
        "signing closure must receive the preserved admin.localhost URL",
    );

    // Both requests must have arrived at the listener (2 slots served).
    let reqs = captured.lock().unwrap();
    assert_eq!(
        reqs.len(),
        2,
        "listener must receive exactly 2 requests (unauthenticated + authenticated); got {}",
        reqs.len(),
    );

    // Both requests must carry `host: admin.localhost:<port>` (case-insensitive header name).
    // The invariant is that the Host *value* preserves `admin.localhost`, not `127.0.0.1`.
    let expected_host_value = format!("admin.localhost:{port}");
    for (i, raw) in reqs.iter().enumerate() {
        let text = std::str::from_utf8(raw).expect("request must be valid UTF-8");
        let lower = text.to_lowercase();
        assert!(
            lower.contains(&format!("host: {expected_host_value}")),
            "request {i} must carry Host: {expected_host_value}; got headers:\n{text}",
        );
    }
}

// ── build_admin_mutation_request wire shape (N8: typed bodyless DELETE) ────────

/// Decode the NIP-98 event embedded in a `Nostr <base64>` Authorization value.
fn decode_nip98_event(auth_value: &str) -> serde_json::Value {
    use base64::Engine as _;
    let b64 = auth_value
        .strip_prefix("Nostr ")
        .expect("authorization header must be a NIP-98 `Nostr <base64>` value");
    let json = base64::engine::general_purpose::STANDARD
        .decode(b64)
        .expect("NIP-98 payload must be valid base64");
    serde_json::from_slice(&json).expect("NIP-98 payload must be a JSON event")
}

/// First value of the NIP-98 tag named `name`, if present.
fn nip98_tag<'a>(event: &'a serde_json::Value, name: &str) -> Option<&'a str> {
    event["tags"].as_array()?.iter().find_map(|t| {
        let arr = t.as_array()?;
        if arr.first()?.as_str()? == name {
            arr.get(1)?.as_str()
        } else {
            None
        }
    })
}

/// A bodyless DELETE built through the shared mutation helper must go on the
/// wire with NO `Content-Type` and NO body, and be NIP-98-signed over the empty
/// payload — byte-identical to the bare DELETE the relay verified before this
/// consolidation routed DELETE through `mutation_admin_json`.
///
/// Mutation evidence: setting `Content-Type`/a body on the `None` branch of
/// `build_admin_mutation_request`, or signing over anything but `&[]`, flips one
/// of the three wire assertions RED. The request is sent through the production
/// `ADMIN_CLIENT`, so a reqwest default header injection would also be caught.
#[tokio::test]
async fn bodyless_delete_wire_is_bare_and_signs_empty_payload() {
    use sha2::{Digest, Sha256};
    use std::sync::{Arc, Mutex};

    client::init_admin_client().expect("client builds");
    let http_client = client::ADMIN_CLIENT.get().unwrap();

    let captured: Arc<Mutex<Vec<u8>>> = Arc::new(Mutex::new(Vec::new()));
    let cap = Arc::clone(&captured);
    let addr = serve_sequence_inspect(
        vec![("200 OK", "Content-Type: application/json\r\n", "{}")],
        Some(Arc::new(move |_idx, bytes: &[u8]| {
            *cap.lock().unwrap() = bytes.to_vec();
        })),
    )
    .await;

    let keys = nostr::Keys::generate();
    let url = format!(
        "http://127.0.0.1:{}/api/admin/v1/operators/{}",
        addr.port(),
        "0".repeat(64)
    );
    let resp = helpers::build_admin_mutation_request(
        http_client,
        &keys,
        &reqwest::Method::DELETE,
        &url,
        None,
    )
    .expect("bodyless request builds")
    .send()
    .await
    .expect("request reaches the loopback listener");
    assert!(resp.status().is_success());

    let raw = captured.lock().unwrap().clone();
    let text = std::str::from_utf8(&raw).expect("request is valid UTF-8");
    let (headers, body) = text.split_once("\r\n\r\n").unwrap_or((text, ""));

    assert!(
        headers.starts_with("DELETE "),
        "request must be a DELETE; got:\n{headers}"
    );
    assert!(
        !headers.to_lowercase().contains("content-type"),
        "a bodyless DELETE must carry no Content-Type on the wire; got:\n{headers}"
    );
    assert!(
        body.is_empty(),
        "a bodyless DELETE must carry no wire body; got body {body:?}"
    );

    let auth_value = headers
        .lines()
        .find(|l| l.to_lowercase().starts_with("authorization:"))
        .and_then(|l| l.split_once(':'))
        .map(|(_, v)| v.trim())
        .expect("NIP-98 Authorization header present");
    let event = decode_nip98_event(auth_value);
    assert_eq!(nip98_tag(&event, "method"), Some("DELETE"));
    assert_eq!(nip98_tag(&event, "u"), Some(url.as_str()));
    assert_eq!(
        nip98_tag(&event, "payload"),
        Some(hex::encode(Sha256::digest(b"")).as_str()),
        "bodyless request must sign over the empty payload"
    );
}

/// The body-bearing branch of the same helper must instead declare
/// `application/json`, send the exact JSON bytes, and bind the NIP-98 `payload`
/// tag to the sha256 of those bytes — the contrast that makes the `Some`/`None`
/// split in `build_admin_mutation_request` falsifiable in both directions.
#[tokio::test]
async fn body_bearing_put_sets_content_type_and_signs_body() {
    use sha2::{Digest, Sha256};

    client::init_admin_client().expect("client builds");
    let http_client = client::ADMIN_CLIENT.get().unwrap();
    let keys = nostr::Keys::generate();
    let url = "https://admin.example.com/api/admin/v1/operators/0000000000000000000000000000000000000000000000000000000000000001";
    let body: &[u8] = br#"{"role":"moderator"}"#;

    let req = helpers::build_admin_mutation_request(
        http_client,
        &keys,
        &reqwest::Method::PUT,
        url,
        Some(body),
    )
    .expect("body-bearing request builds")
    .build()
    .expect("request is well-formed");

    assert_eq!(req.method(), reqwest::Method::PUT);
    assert_eq!(
        req.headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok()),
        Some("application/json"),
        "a body-bearing mutation must declare application/json"
    );
    assert_eq!(
        req.body().and_then(reqwest::Body::as_bytes),
        Some(body),
        "the exact JSON bytes must reach the wire"
    );

    let auth_value = req
        .headers()
        .get(reqwest::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .expect("NIP-98 Authorization header present");
    let event = decode_nip98_event(auth_value);
    assert_eq!(
        nip98_tag(&event, "payload"),
        Some(hex::encode(Sha256::digest(body)).as_str()),
        "body-bearing request must sign over the sha256 of the exact body"
    );
}

// ── Discovery same-host binding ───────────────────────────────────────────
//
// `advertised_host_matches_relay` is the production seam that decides whether a
// relay-advertised admin origin is trusted for auto-save + auto-probe. It gates
// the `same_host` flag returned by `discover_admin_origin_at`; the TypeScript
// layer only auto-probes (signing a NIP-98 header with the operator key) when
// that flag is set, so a mismatch here is the difference between offering a
// pre-fill and handing an attacker-advertised host an unconsented signature.

#[test]
fn advertised_host_trust_binding() {
    let cases = [
        // Positive: identical host → auto-probe permitted.
        (
            "https://admin.example.com",
            "https://admin.example.com",
            true,
            "identical host must bind for auto-probe",
        ),
        // Positive: port/path differ but host matches — operator may run the
        // admin console on a different port and still be same-host-bound.
        // (Scheme variation is not tested here; this fixture varies port only.)
        (
            "https://admin.example.com:8443",
            "https://admin.example.com/query",
            true,
            "host match must bind regardless of port or path",
        ),
        // Positive: case-only host variation — `AdminOrigin::parse` lowercases
        // the advertised host; relay-URL side compared case-insensitively.
        (
            "https://Admin.Example.Com",
            "https://ADMIN.EXAMPLE.COM",
            true,
            "case-only differences must still bind the same host",
        ),
        // Negative: cross-host advertisement must NOT bind for auto-probe —
        // a mismatch here would allow an attacker-advertised host to obtain a
        // NIP-98 signed request with the operator's key.
        (
            "https://attacker.example.com",
            "https://admin.example.com",
            false,
            "a cross-host advertisement must not bind for auto-probe",
        ),
    ];
    for (advertised_url, relay_url, expected, label) in cases {
        let advertised = AdminOrigin::parse(advertised_url).unwrap();
        assert_eq!(
            discovery::advertised_host_matches_relay(&advertised, relay_url),
            expected,
            "{label}",
        );
    }
}

// ── Native Save core over the wire ────────────────────────────────────────

const SAVE_FEEDBACK_ID: &str = "00000000-0000-0000-0000-00000000fb01";

fn save_sha() -> String {
    "ab".repeat(32)
}

#[tokio::test]
async fn save_core_writes_the_served_bytes_to_the_chosen_file() {
    let addr = serve_sequence(vec![(
        "200 OK",
        "Content-Type: application/pdf\r\n",
        "%PDF-1.7 saved",
    )])
    .await;
    let dir = tempfile::tempdir().unwrap();
    let dest = dir.path().join("saved.pdf");
    let pick_dest = dest.clone();
    let saved = attachment::save_feedback_attachment(
        &format!("http://127.0.0.1:{}", addr.port()),
        SAVE_FEEDBACK_ID,
        &save_sha(),
        "application/pdf",
        14,
        &nostr::Keys::generate(),
        |_, _, _| async move { Ok(Some(pick_dest)) },
    )
    .await;
    assert_eq!(saved, Ok(true));
    assert_eq!(std::fs::read(&dest).unwrap(), b"%PDF-1.7 saved");
}

#[tokio::test]
async fn save_core_fetch_failure_never_prompts() {
    let addr = serve_sequence(vec![("500 Internal Server Error", "", "")]).await;
    let saved = attachment::save_feedback_attachment(
        &format!("http://127.0.0.1:{}", addr.port()),
        SAVE_FEEDBACK_ID,
        &save_sha(),
        "application/pdf",
        14,
        &nostr::Keys::generate(),
        |_, _, _| -> std::future::Ready<Result<Option<std::path::PathBuf>, String>> {
            panic!("must not prompt after a failed fetch")
        },
    )
    .await;
    assert_eq!(saved, Err("admin_attachment_relay_error_500".to_string()));
}

/// The relay serves non-raster attachments as `application/octet-stream` +
/// `Content-Disposition: attachment` (`media.rs` response policy); the imeta
/// sidecar still says `application/pdf`.
const RELAY_OCTET_STREAM_HEADERS: &str = "Content-Type: application/octet-stream\r\nContent-Disposition: attachment\r\nX-Content-Type-Options: nosniff\r\n";

#[tokio::test]
async fn save_core_accepts_a_pdf_served_as_relay_octet_stream() {
    let addr = serve_sequence(vec![(
        "200 OK",
        RELAY_OCTET_STREAM_HEADERS,
        "%PDF-1.7 saved",
    )])
    .await;
    let dir = tempfile::tempdir().unwrap();
    let dest = dir.path().join("report.pdf");
    let pick_dest = dest.clone();
    let saved = attachment::save_feedback_attachment(
        &format!("http://127.0.0.1:{}", addr.port()),
        SAVE_FEEDBACK_ID,
        &save_sha(),
        "application/pdf",
        14,
        &nostr::Keys::generate(),
        |_, _, _| async move { Ok(Some(pick_dest)) },
    )
    .await;
    assert_eq!(saved, Ok(true));
    assert_eq!(std::fs::read(&dest).unwrap(), b"%PDF-1.7 saved");
}

#[tokio::test]
async fn preview_still_rejects_relay_octet_stream() {
    let addr = serve_sequence(vec![(
        "200 OK",
        RELAY_OCTET_STREAM_HEADERS,
        "%PDF-1.7 saved",
    )])
    .await;
    let fetched = attachment::fetch_feedback_attachment(
        &format!("http://127.0.0.1:{}", addr.port()),
        SAVE_FEEDBACK_ID,
        &save_sha(),
        "application/pdf",
        14,
        &nostr::Keys::generate(),
        helpers::AttachmentUse::Preview,
    )
    .await;
    assert_eq!(fetched, Err("admin_attachment_mime_mismatch".to_string()));
}

#[tokio::test]
async fn save_rejects_an_unrelated_content_type() {
    let addr = serve_sequence(vec![(
        "200 OK",
        "Content-Type: text/html\r\n",
        "%PDF-1.7 saved",
    )])
    .await;
    let saved = attachment::save_feedback_attachment(
        &format!("http://127.0.0.1:{}", addr.port()),
        SAVE_FEEDBACK_ID,
        &save_sha(),
        "application/pdf",
        14,
        &nostr::Keys::generate(),
        |_, _, _| -> std::future::Ready<Result<Option<std::path::PathBuf>, String>> {
            panic!("must not prompt on a type mismatch")
        },
    )
    .await;
    assert_eq!(saved, Err("admin_attachment_mime_mismatch".to_string()));
}

// ── Restrictions community scoping ────────────────────────────────────────

#[test]
fn restrictions_url_names_the_active_relay_authority() {
    let state = relay_state("wss://Community.Example.com:8443/ws");
    let pubkey = routes::HexPubkey::parse(&"ab".repeat(32)).unwrap();
    let url = restrictions_url(
        "https://admin.example.com",
        &routes::AdminRoute::MemberBanDelete { pubkey },
        None,
        "wss://Community.Example.com:8443/ws",
        &state,
    )
    .unwrap();
    assert!(
        url.ends_with("?communityHost=community.example.com%3A8443"),
        "{url}"
    );
    assert!(!url.contains("communityId"), "{url}");
}

#[test]
fn restrictions_url_carries_the_cursor_and_default_port_host() {
    let state = relay_state("wss://relay.example.com");
    let url = restrictions_url(
        "https://admin.example.com",
        &routes::AdminRoute::MemberRestrictionsList,
        Some("tok".to_string()),
        "wss://relay.example.com",
        &state,
    )
    .unwrap();
    assert!(
        url.ends_with("/members/restrictions?cursor=tok&communityHost=relay.example.com")
            || url.ends_with("/members/restrictions?communityHost=relay.example.com&cursor=tok"),
        "{url}"
    );
}

#[test]
fn restrictions_url_errors_when_the_relay_host_is_unresolvable() {
    let state = relay_state("not a url");
    let err = restrictions_url(
        "https://admin.example.com",
        &routes::AdminRoute::MemberRestrictionsList,
        None,
        "not a url",
        &state,
    )
    .unwrap_err();
    assert_eq!(err, "admin_community_host_unresolved");
}

#[test]
fn restrictions_url_rejects_a_caller_relay_that_no_longer_matches() {
    // The list loaded from relay A; the native relay has since switched to B.
    // Every restriction route must fail before building a request URL.
    let state = relay_state("wss://relay-b.example.com");
    let pubkey = routes::HexPubkey::parse(&"ab".repeat(32)).unwrap();
    for route in [
        routes::AdminRoute::MemberRestrictionsList,
        routes::AdminRoute::MemberBanDelete {
            pubkey: pubkey.clone(),
        },
        routes::AdminRoute::MemberTimeoutDelete { pubkey },
    ] {
        for expected in ["wss://relay-a.example.com", "", "  "] {
            let err = restrictions_url("https://admin.example.com", &route, None, expected, &state)
                .unwrap_err();
            assert_eq!(err, RELAY_SCOPE_CHANGED);
        }
    }
}

// ── Direct moderation actions ─────────────────────────────────────────────

const SIGNER: &str = "11111111111111111111111111111111111111111111111111111111111111aa";

fn intent(action: DirectAction, expiration_secs: Option<u64>) -> AdminDirectIntent {
    AdminDirectIntent {
        origin: "https://admin.example.com".to_string(),
        expected_relay: "wss://relay.example.com".to_string(),
        expected_pubkey: SIGNER.to_string(),
        community_host: "Team.Example.com".to_string(),
        action,
        target: "ab".repeat(32),
        request_id: uuid::Uuid::nil(),
        reason: Some("spam".to_string()),
        expiration_secs,
    }
}

const BASE: &str = "https://relay.example.com";

#[test]
fn direct_action_sends_the_typed_host_only_as_the_query() {
    let (url, body) =
        direct_action_request(&intent(DirectAction::Timeout, Some(60)), BASE, SIGNER).unwrap();
    let t = "ab".repeat(32);
    assert!(
        url.ends_with(&format!(
            "/api/admin/v1/members/{t}/timeout?communityHost=team.example.com"
        )),
        "{url}"
    );
    let body: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(
        body,
        serde_json::json!({
            "requestId": uuid::Uuid::nil(),
            "reason": "spam",
            "expirationSecs": 60,
        })
    );
    let (url, body) =
        direct_action_request(&intent(DirectAction::Delete, None), BASE, SIGNER).unwrap();
    assert!(url.contains(&format!("/events/{t}/delete?")), "{url}");
    assert!(!String::from_utf8(body).unwrap().contains("expirationSecs"));
}

#[test]
fn direct_action_refuses_a_changed_signer_or_relay() {
    // Confirmed under SIGNER on relay.example.com; each mismatch fails pre-send.
    let other = "22".repeat(32);
    for (signer, base) in [
        (other.as_str(), BASE),
        (SIGNER, "https://relay-b.example.com"),
    ] {
        assert!(direct_action_request(&intent(DirectAction::Ban, None), base, signer).is_err());
    }
    let mut blank = intent(DirectAction::Ban, None);
    blank.expected_pubkey = " ".to_string();
    assert!(direct_action_request(&blank, BASE, SIGNER).is_err());
}

#[test]
fn direct_action_rejects_bad_host_target_and_duration() {
    let mut bad_host = intent(DirectAction::Ban, None);
    bad_host.community_host = "https://team.example.com/x".to_string();
    let mut bad_target = intent(DirectAction::Ban, None);
    bad_target.target = "AB".repeat(32);
    for bad in [
        bad_host,
        bad_target,
        intent(DirectAction::Timeout, None),
        intent(DirectAction::Timeout, Some(0)),
        intent(DirectAction::Ban, Some(60)),
    ] {
        assert!(
            direct_action_request(&bad, BASE, SIGNER).is_err(),
            "{bad:?}"
        );
    }
}

/// AppState signing as `keys` on relay.example.com, plus a direct intent
/// confirmed under `keys` that targets the loopback admin origin at `addr`.
fn direct_state_and_intent(
    keys: &nostr::Keys,
    addr: std::net::SocketAddr,
    reason: &str,
) -> (crate::app_state::AppState, AdminDirectIntent) {
    let state = crate::app_state::build_app_state();
    *state.keys.lock().unwrap() = keys.clone();
    *state.relay_url_override.lock().unwrap() = Some("wss://relay.example.com".to_string());
    let mut intent = intent(DirectAction::Ban, None);
    intent.origin = format!("http://127.0.0.1:{}", addr.port());
    intent.expected_pubkey = keys.public_key().to_hex();
    intent.reason = Some(reason.to_string());
    (state, intent)
}

/// Mutation evidence: removing the guard call in `send_admin_mutation` lets
/// both key-backup reasons reach the listener and flips the count RED.
#[tokio::test]
async fn direct_action_refuses_key_backup_reason_before_any_request() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    let hits = Arc::new(AtomicUsize::new(0));
    let seen = Arc::clone(&hits);
    let addr = serve_sequence_inspect(
        vec![(
            "200 OK",
            "Content-Type: application/json\r\n",
            r#"{"state":"applied"}"#,
        )],
        Some(Arc::new(move |_, _| {
            seen.fetch_add(1, Ordering::SeqCst);
        })),
    )
    .await;
    let keys = nostr::Keys::generate();
    for reason in ["see ncryptsec1qgg9947", "see NCRYPTSEC1QGG9947"] {
        let (state, intent) = direct_state_and_intent(&keys, addr, reason);
        let err = send_direct_action(&intent, keys.clone(), &state)
            .await
            .unwrap_err();
        assert!(format!("{err:?}").contains("key-backup"), "{err:?}");
    }
    assert_eq!(
        hits.load(Ordering::SeqCst),
        0,
        "a key backup reached the relay"
    );
    let (state, intent) = direct_state_and_intent(&keys, addr, "spam");
    send_direct_action(&intent, keys.clone(), &state)
        .await
        .expect("an ordinary reason still sends");
    assert_eq!(hits.load(Ordering::SeqCst), 1);
}

/// The identity is swapped to B after the snapshot is taken; the first send
/// and the 401 retry must both still be signed by the confirmed key A.
///
/// Mutation evidence: signing with a re-read `state.signing_keys()` inside
/// `send_direct_action` (the pre-fix shape) signs as B and flips this RED.
#[tokio::test]
async fn direct_action_signs_only_with_the_validated_snapshot() {
    use std::sync::Mutex;
    let auths: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let rec = Arc::clone(&auths);
    let addr = serve_sequence_inspect(
        vec![
            ("401 Unauthorized", "WWW-Authenticate: Nostr\r\n", ""),
            (
                "200 OK",
                "Content-Type: application/json\r\n",
                r#"{"state":"applied"}"#,
            ),
        ],
        Some(Arc::new(move |_, bytes: &[u8]| {
            let text = String::from_utf8_lossy(bytes);
            let auth = text
                .lines()
                .find_map(|l| {
                    l.strip_prefix("authorization: ")
                        .or(l.strip_prefix("Authorization: "))
                })
                .expect("NIP-98 header present");
            let pubkey = decode_nip98_event(auth.trim())["pubkey"]
                .as_str()
                .unwrap()
                .to_string();
            rec.lock().unwrap().push(pubkey);
        })),
    )
    .await;
    let a = nostr::Keys::generate();
    let (state, intent) = direct_state_and_intent(&a, addr, "spam");
    let snapshot = state.signing_keys().unwrap();
    *state.keys.lock().unwrap() = nostr::Keys::generate(); // identity import of B
    send_direct_action(&intent, snapshot, &state)
        .await
        .expect("A-confirmed action sends as A");
    assert_eq!(*auths.lock().unwrap(), vec![a.public_key().to_hex(); 2]);
}
