use super::*;

/// NIP-49 spec vector — a real ncryptsec blob for injection payloads.
const NCRYPTSEC: &str = "ncryptsec1qgg9947rlpvqu76pj5ecreduf9jxhselq2nae2kghhvd5g7dgjtcxfqtd67p9m0w57lspw8gsq6yphnm8623nsl8xn9j4jdzz84zm3frztj3z7s35vpzmqf6ksu8r89qk5z2zxfmu5gv8th8wclt0h4p";

fn assert_guard_error(err: &str) {
    assert!(
        err.contains("key-backup material"),
        "expected the egress-guard error, got: {err}"
    );
}

// ── Guard unit behavior ───────────────────────────────────────────────────────

#[test]
fn rejects_ncryptsec_anywhere_in_text() {
    assert_guard_error(&assert_no_key_backup(NCRYPTSEC, "test").unwrap_err());
    assert_guard_error(
        &assert_no_key_backup(
            &format!("{{\"content\":\"my backup: {NCRYPTSEC}\"}}"),
            "test",
        )
        .unwrap_err(),
    );
}

/// Bech32 permits an all-uppercase encoding of the same payload — an
/// uppercased valid backup must not bypass the guard (text and bytes).
/// Mixed case is invalid bech32 (cannot decode) and is deliberately not
/// blocked.
#[test]
fn rejects_uppercase_ncryptsec() {
    let upper = NCRYPTSEC.to_ascii_uppercase();
    assert_guard_error(&assert_no_key_backup(&upper, "test").unwrap_err());
    assert_guard_error(&assert_no_key_backup_bytes(upper.as_bytes(), "test").unwrap_err());
    // Mixed case cannot decode; not blocked.
    assert!(assert_no_key_backup("nCrYpTsEc1qgg9947r", "test").is_ok());
}

#[test]
fn passes_clean_payloads_including_raw_nsec() {
    assert!(assert_no_key_backup("hello world", "test").is_ok());
    assert!(assert_no_key_backup("", "test").is_ok());
    // Scope is ncryptsec1 ONLY: raw nsec intentionally transits the encrypted
    // pairing session and must NOT be blocked (plan D4 / pairing.rs).
    let nsec = nostr::ToBech32::to_bech32(nostr::Keys::generate().secret_key()).unwrap();
    assert!(assert_no_key_backup(&nsec, "test").is_ok());
    // Near-miss prefixes are not blocked.
    assert!(assert_no_key_backup("ncryptsec", "test").is_ok());
}

#[test]
fn byte_variant_matches_text_variant() {
    assert_guard_error(&assert_no_key_backup_bytes(NCRYPTSEC.as_bytes(), "test").unwrap_err());
    assert!(assert_no_key_backup_bytes(b"clean body", "test").is_ok());
    // Invalid UTF-8 around an intact ncryptsec substring must still trip the
    // guard (from_utf8_lossy preserves the ASCII run).
    let mut body = vec![0xff, 0xfe];
    body.extend_from_slice(NCRYPTSEC.as_bytes());
    body.push(0xff);
    assert_guard_error(&assert_no_key_backup_bytes(&body, "test").unwrap_err());
}

#[test]
fn error_names_the_boundary_context() {
    let err = assert_no_key_backup(NCRYPTSEC, "huddle STT publish").unwrap_err();
    assert!(err.contains("huddle STT publish"), "{err}");
}

// ── Runtime injection per boundary ────────────────────────────────────────────
//
// Each test drives the real production function with an ncryptsec-bearing
// payload and asserts the guard aborts the operation before any network I/O
// (no listener exists at the target address; a distinctive guard error — not
// a connection error — proves the abort happened first).
//
// Boundaries 6 and 7 (`submit_engram_event` twins) are module-private inside
// `commands`; their injection tests live next to them:
//   - commands/team_snapshot/tests.rs::egress_guard_boundary
//   - commands/personas/snapshot/import.rs::egress_guard_tests

/// Boundary 1: `relay/submit.rs` `submit_event_at_with_keys` (the funnel for
/// all `submit_event*` variants).
#[tokio::test]
async fn boundary_submit_event_at_with_keys_blocks_ncryptsec() {
    let state = crate::app_state::build_app_state();
    let keys = nostr::Keys::generate();
    let builder = nostr::EventBuilder::new(nostr::Kind::Custom(9), NCRYPTSEC);
    let err = crate::relay::submit_event_at_with_keys(
        builder,
        &state,
        "http://127.0.0.1:9", // discard port — must never be reached
        &keys,
    )
    .await
    .unwrap_err();
    assert_guard_error(&err);
}

/// Boundary 2: `relay.rs` `sync_managed_agent_profile` (agent kind:0 profile).
#[tokio::test]
async fn boundary_sync_managed_agent_profile_blocks_ncryptsec() {
    let state = crate::app_state::build_app_state();
    let keys = nostr::Keys::generate();
    let err = crate::relay::sync_managed_agent_profile(
        &state,
        "ws://127.0.0.1:9",
        &keys,
        &format!("agent {NCRYPTSEC}"),
        None,
        None,
    )
    .await
    .unwrap_err();
    assert_guard_error(&err);
}

/// Boundary 3: `relay/submit.rs` `submit_signed_event_at_with_keys` — the
/// pre-signed entry into the boundary-1 funnel (main's submit refactor
/// replaced `relay.rs` `submit_signed_event` with this scoped form).
#[tokio::test]
async fn boundary_submit_signed_event_at_with_keys_blocks_ncryptsec() {
    let state = crate::app_state::build_app_state();
    let keys = nostr::Keys::generate();
    let event = nostr::EventBuilder::new(nostr::Kind::Custom(9), NCRYPTSEC)
        .sign_with_keys(&keys)
        .unwrap();
    let err = crate::relay::submit_signed_event_at_with_keys(
        &event,
        &state,
        "http://127.0.0.1:9", // discard port — must never be reached
        &keys,
    )
    .await
    .unwrap_err();
    assert_guard_error(&err);
}

/// Boundary 4: `relay.rs` `submit_signed_event_with_keys`.
#[tokio::test]
async fn boundary_submit_signed_event_with_keys_blocks_ncryptsec() {
    let state = crate::app_state::build_app_state();
    *state.relay_url_override.lock().unwrap() = Some("ws://127.0.0.1:9".to_string());
    let keys = nostr::Keys::generate();
    let event = nostr::EventBuilder::new(nostr::Kind::Custom(9), NCRYPTSEC)
        .sign_with_keys(&keys)
        .unwrap();
    let err = crate::relay::submit_signed_event_with_keys(&event, &state, &keys, None)
        .await
        .unwrap_err();
    assert_guard_error(&err);
}

/// Boundary 5: huddle STT publisher (`huddle/pipeline.rs`).
#[test]
fn boundary_huddle_stt_blocks_ncryptsec() {
    let keys = nostr::Keys::generate();
    let channel = uuid::Uuid::new_v4();
    let builder =
        crate::events::build_message(channel, NCRYPTSEC, None, &[], &[], &[], &[]).unwrap();
    let err = crate::huddle::pipeline::sign_and_guard_stt_body(builder, &keys).unwrap_err();
    assert_guard_error(&err);

    // Clean transcripts pass through the same seam.
    let builder =
        crate::events::build_message(channel, "hello huddle", None, &[], &[], &[], &[]).unwrap();
    assert!(crate::huddle::pipeline::sign_and_guard_stt_body(builder, &keys).is_ok());
}

/// Boundary 8: native websocket send loop — the single choke point for all
/// webview-originated relay websocket frames.
#[tokio::test]
async fn boundary_native_websocket_blocks_ncryptsec() {
    let manager = crate::native_websocket::WebSocketManager::default();
    // Text frame: guard fires before the connection lookup, so no connection
    // is needed — and the error must be the guard's, not "not found".
    let err = crate::native_websocket::send_message(
        &manager,
        1,
        crate::native_websocket::WebSocketMessage::Text(format!(
            "[\"EVENT\",{{\"content\":\"{NCRYPTSEC}\"}}]"
        )),
    )
    .await
    .unwrap_err();
    assert_guard_error(&err);

    // Binary frame variant.
    let err = crate::native_websocket::send_message(
        &manager,
        1,
        crate::native_websocket::WebSocketMessage::Binary(NCRYPTSEC.as_bytes().to_vec()),
    )
    .await
    .unwrap_err();
    assert_guard_error(&err);

    // Clean frames fall through to normal handling ("connection not found"
    // here — the guard did not reject them).
    let err = crate::native_websocket::send_message(
        &manager,
        1,
        crate::native_websocket::WebSocketMessage::Text("[\"REQ\",\"sub\",{}]".to_string()),
    )
    .await
    .unwrap_err();
    assert!(err.contains("not found"), "{err}");
}

// ── Structural tripwires ──────────────────────────────────────────────────────

fn src_rust_files() -> Vec<std::path::PathBuf> {
    fn walk(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                walk(&path, out);
            } else if path.extension().and_then(|e| e.to_str()) == Some("rs") {
                out.push(path);
            }
        }
    }
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut out = Vec::new();
    walk(&root, &mut out);
    out
}

/// Site-granular `/events` inventory: `(file suffix, expected non-comment
/// `/events` occurrences, expected guard call sites — full-path calls into
/// the egress-guard module)`.
///
/// Every entry pairs the URL-construction count with the guard-call count for
/// that file, so BOTH of these fail the scan (not just a brand-new file):
///   - adding an unguarded ninth `/events` site inside an already-listed file
///     (count goes up without a matching table update), and
///   - removing/refactoring away a guard call while its egress site remains.
///
/// Updating a row here is the deliberate act that must accompany wiring the
/// guard + adding an injection test for the new site.
const EVENTS_INVENTORY: &[(&str, usize, usize)] = &[
    // Production egress boundaries (see egress_guard.rs table):
    ("src/relay.rs", 2, 2),                             // boundaries 2, 4
    ("src/relay/submit.rs", 1, 1),                      // boundaries 1 + 3 (shared funnel)
    ("src/huddle/pipeline.rs", 1, 1),                   // boundary 5
    ("src/commands/team_snapshot.rs", 1, 1),            // boundary 6
    ("src/commands/personas/snapshot/import.rs", 2, 1), // boundary 7 + its in-file injection-test fixture URL
    ("src/native_websocket.rs", 0, 2),                  // boundary 8 (WS frames; no events URL)
    // Test-only fixtures — no production egress, no guard:
    ("src/relay_admission.rs", 1, 0),
    ("src/archive/mod_tests.rs", 1, 0),
    ("src/managed_agents/persona_events/tests.rs", 1, 0),
    ("src/commands/team_snapshot/tests.rs", 1, 0),
    // Mock-relay route in its in-file tests; production publish goes through
    // the guarded boundary-1 funnel (`submit_signed_event_at_with_keys`).
    ("src/commands/personas/sharing.rs", 1, 0),
];

// Needles are assembled at runtime so this scan file itself contains no
// contiguous match and needs no self-referential inventory row.
fn events_needle() -> String {
    ["/ev", "ents"].concat()
}
fn guard_needle() -> String {
    ["egress_guard::", "assert_no_key_backup"].concat()
}

/// Pure scan core over `(relative path, content)` pairs. Returns violations;
/// empty means every file matches its inventory row exactly (files absent
/// from the table are expected to have zero `/events` sites and zero guard
/// calls).
fn events_inventory_violations(files: &[(String, String)]) -> Vec<String> {
    let events = events_needle();
    let guard = guard_needle();
    let mut violations = Vec::new();

    for (rel, content) in files {
        let expected = EVENTS_INVENTORY
            .iter()
            .find(|(suffix, _, _)| rel.ends_with(suffix))
            .map(|&(_, e, g)| (e, g))
            .unwrap_or((0, 0));

        let mut event_sites = Vec::new();
        for (i, line) in content.lines().enumerate() {
            if line.trim_start().starts_with("//") {
                continue; // doc/comment mentions
            }
            if line.contains(&events) {
                event_sites.push(format!("  {rel}:{}: {}", i + 1, line.trim()));
            }
        }
        let guard_count = content.matches(&guard).count();

        if (event_sites.len(), guard_count) != expected {
            violations.push(format!(
                "{rel}: found {} events-URL site(s) + {} guard call(s), inventory \
                 expects {} + {}. Sites found:\n{}",
                event_sites.len(),
                guard_count,
                expected.0,
                expected.1,
                if event_sites.is_empty() {
                    "  (none)".to_string()
                } else {
                    event_sites.join("\n")
                },
            ));
        }
    }
    violations
}

fn read_src_files() -> Vec<(String, String)> {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    src_rust_files()
        .into_iter()
        .map(|path| {
            let rel = path
                .strip_prefix(root)
                .unwrap()
                .to_string_lossy()
                .replace('\\', "/");
            let content = std::fs::read_to_string(&path).unwrap();
            (rel, content)
        })
        .collect()
}

/// Inventory completeness: every `/events` URL-construction site in
/// `desktop/src-tauri/src` must match the site-granular inventory above. A
/// future ninth submission path — in a NEW file or an ALREADY-LISTED one —
/// fails this test until its guard is wired, its injection test exists, and
/// its inventory row is updated.
#[test]
fn events_url_inventory_is_fully_guarded() {
    let violations = events_inventory_violations(&read_src_files());
    assert!(
        violations.is_empty(),
        "events-URL egress inventory drift — wire crate::egress_guard, add an \
         injection test, then update EVENTS_INVENTORY:\n{}",
        violations.join("\n")
    );
}

/// Mutation-style proof of the tripwire's guarantee: an unguarded ninth
/// `/events` site added to an already-inventoried file (relay.rs) is caught.
#[test]
fn inventory_scan_catches_new_site_in_allowlisted_file() {
    let mut files = read_src_files();
    let relay = files
        .iter_mut()
        .find(|(rel, _)| rel.ends_with("src/relay.rs"))
        .expect("relay.rs must be in the scan set");
    relay.1.push_str(&format!(
        "\nfn sneaky_ninth_site(base: &str) -> String {{ format!(\"{{base}}{}\") }}\n",
        events_needle()
    ));
    let violations = events_inventory_violations(&files);
    assert!(
        violations.iter().any(|v| v.contains("src/relay.rs")),
        "an unguarded ninth events-URL site in relay.rs must trip the scan: {violations:?}"
    );
}

/// The pairing also fires in reverse: a guard call deleted while its egress
/// site remains is caught.
#[test]
fn inventory_scan_catches_removed_guard_call() {
    let mut files = read_src_files();
    let relay = files
        .iter_mut()
        .find(|(rel, _)| rel.ends_with("src/relay.rs"))
        .expect("relay.rs must be in the scan set");
    relay.1 = relay.1.replacen(&guard_needle(), "removed_guard", 1);
    let violations = events_inventory_violations(&files);
    assert!(
        violations.iter().any(|v| v.contains("src/relay.rs")),
        "a removed guard call in relay.rs must trip the scan: {violations:?}"
    );
}

/// A brand-new file with an `/events` site (no inventory row) is caught.
#[test]
fn inventory_scan_catches_new_unlisted_file() {
    let mut files = read_src_files();
    files.push((
        "src/brand_new_egress.rs".to_string(),
        format!("let url = format!(\"{{}}{}\", base);", events_needle()),
    ));
    let violations = events_inventory_violations(&files);
    assert!(
        violations
            .iter()
            .any(|v| v.contains("src/brand_new_egress.rs")),
        "{violations:?}"
    );
}

/// Source allowlist: NIP-49 material handling is confined to the identity /
/// backup / import / guard files. Anything else touching ncryptsec or the
/// nip49 codec is structural drift.
#[test]
fn ncryptsec_handling_is_confined_to_allowlisted_files() {
    let allowlist: &[&str] = &[
        "src/key_backup.rs",
        "src/key_backup_tests.rs",
        "src/egress_guard.rs",
        "src/egress_guard_tests.rs",
        "src/commands/identity.rs",
        "src/commands/identity_key_backup_tests.rs",
        "src/lib.rs", // module registration + invoke handler
        // boundary wiring (guard call sites name the module, not the codec):
        "src/relay.rs",
        "src/relay/submit.rs",
        "src/huddle/pipeline.rs",
        "src/commands/team_snapshot.rs",
        "src/commands/team_snapshot/tests.rs",
        "src/commands/personas/snapshot/import.rs",
        "src/native_websocket.rs",
    ];

    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut violations = Vec::new();
    for path in src_rust_files() {
        let rel = path
            .strip_prefix(root)
            .unwrap()
            .to_string_lossy()
            .replace('\\', "/");
        if allowlist.iter().any(|a| rel.ends_with(a)) {
            continue;
        }
        let content = std::fs::read_to_string(&path).unwrap();
        for needle in ["ncryptsec", "EncryptedSecretKey", "nip49"] {
            if content.contains(needle) {
                violations.push(format!("{rel}: contains {needle:?}"));
            }
        }
    }
    assert!(
        violations.is_empty(),
        "NIP-49 material outside allowlisted files:\n{}",
        violations.join("\n")
    );
}
