// Tests for commands/media_download.rs — split into a sibling file to keep
// media_download.rs under the per-file line cap.

use super::*;

#[test]
fn snapshot_kind_json_returns_json_kind_and_correct_cap() {
    let kind = snapshot_kind_for_filename("analyst.agent.json").unwrap();
    assert_eq!(kind, SnapshotFileKind::AgentJson);
    assert_eq!(kind.cap(), MAX_SNAPSHOT_JSON_BYTES as u64);
}

#[test]
fn snapshot_kind_png_returns_png_kind_and_correct_cap() {
    let kind = snapshot_kind_for_filename("analyst.agent.png").unwrap();
    assert_eq!(kind, SnapshotFileKind::AgentPng);
    assert_eq!(kind.cap(), MAX_SNAPSHOT_PNG_BYTES as u64);
}

#[test]
fn snapshot_kind_plain_json_rejected() {
    assert!(snapshot_kind_for_filename("data.json").is_err());
}

#[test]
fn snapshot_kind_deceptive_name_rejected() {
    // foo.agent.json.exe must not match .agent.json
    assert!(snapshot_kind_for_filename("foo.agent.json.exe").is_err());
}

#[test]
fn snapshot_kind_plain_png_rejected() {
    assert!(snapshot_kind_for_filename("photo.png").is_err());
}

#[test]
fn snapshot_kind_agent_json_only_rejected() {
    // "agent.json" without the leading dot — plain filename, not the suffix
    assert!(snapshot_kind_for_filename("agentjson").is_err());
}

#[test]
fn snapshot_kind_team_extensions_are_case_insensitive_and_scale_caps() {
    let json = snapshot_kind_for_filename("review.TEAM.JSON").unwrap();
    let png = snapshot_kind_for_filename("review.TEAM.PNG").unwrap();
    assert_eq!(json, SnapshotFileKind::TeamJson);
    assert_eq!(png, SnapshotFileKind::TeamPng);
    assert_eq!(json.cap(), 25 * 1024 * 1024);
    assert_eq!(png.cap(), 50 * 1024 * 1024);
}

#[test]
fn fetch_boundary_team_png_filename_with_json_bytes_rejected() {
    let bytes = br#"{"format":"buzz-team-snapshot","version":1}"#;
    let kind = snapshot_kind_for_filename("review.team.png").unwrap();
    let error = ensure_bytes_match_kind(bytes, kind).unwrap_err();
    assert!(error.contains(".team.png") && error.contains("not a PNG"));
}

#[test]
fn fetch_boundary_team_declared_size_over_cap_rejected() {
    let kind = snapshot_kind_for_filename("review.team.json").unwrap();
    assert!(ensure_declared_size_within_cap(MAX_TEAM_SNAPSHOT_JSON_BYTES, kind).is_ok());
    let error =
        ensure_declared_size_within_cap(MAX_TEAM_SNAPSHOT_JSON_BYTES + 1, kind).unwrap_err();
    assert!(error.contains("25 MiB"));
}

// ── Focused boundary tests: format mismatch and consistency ──────────────
//
// These tests exercise the guard logic that fetch_snapshot_bytes applies
// after the bounded fetch + hash check.  The validation has two layers:
//
//   1. Magic-byte kind check: filename kind (from snapshot_kind_for_filename)
//      must match the actual byte format (PNG magic or absence of it).
//   2. decode_snapshot_from_bytes: rejects malformed manifests including
//      JSON with level:none + non-empty entries.
//
// We verify each rejection path directly — no live HTTP required.

#[test]
fn fetch_boundary_png_filename_with_json_bytes_rejected() {
    use crate::managed_agents::agent_snapshot::{
        encode_snapshot_json, AgentSnapshot, AgentSnapshotDefinition, AgentSnapshotMemory,
        AgentSnapshotProfile, FORMAT_DISCRIMINATOR, FORMAT_VERSION,
    };
    let snapshot = AgentSnapshot {
        format: FORMAT_DISCRIMINATOR.to_string(),
        version: FORMAT_VERSION,
        definition: AgentSnapshotDefinition {
            name: "test".to_string(),
            source_is_builtin: false,
            system_prompt: None,
            runtime: None,
            model: None,
            provider: None,
            parallelism: None,
            respond_to: None,
            respond_to_allowlist: vec![],
            name_pool: vec![],
            idle_timeout_seconds: None,
            max_turn_duration_seconds: None,
        },
        profile: AgentSnapshotProfile {
            display_name: "Test".to_string(),
            about: None,
            avatar_data_url: None,
            avatar_url: None,
        },
        memory: AgentSnapshotMemory {
            level: crate::managed_agents::agent_snapshot::MemoryLevel::None,
            entries: vec![],
        },
    };
    let json_bytes = encode_snapshot_json(&snapshot).unwrap();
    // .agent.png filename → Png kind; JSON bytes must be rejected.
    let kind = snapshot_kind_for_filename("analyst.agent.png").unwrap();
    let result = ensure_bytes_match_kind(&json_bytes, kind);
    assert!(
        result.is_err(),
        ".agent.png filename with JSON bytes must be rejected by the magic-byte guard"
    );
    assert!(
        result.unwrap_err().contains("not a PNG"),
        "error must describe the mismatch"
    );
}

#[test]
fn fetch_boundary_png_filename_with_memory_bearing_json_bytes_rejected() {
    use crate::managed_agents::agent_snapshot::{
        encode_snapshot_json, AgentSnapshot, AgentSnapshotDefinition, AgentSnapshotMemory,
        AgentSnapshotMemoryEntry, AgentSnapshotProfile, FORMAT_DISCRIMINATOR, FORMAT_VERSION,
    };
    // This is the trust-hole case: memory-bearing JSON delivered under a
    // .agent.png label to bypass the PNG no-memory policy.
    let snapshot = AgentSnapshot {
        format: FORMAT_DISCRIMINATOR.to_string(),
        version: FORMAT_VERSION,
        definition: AgentSnapshotDefinition {
            name: "test".to_string(),
            source_is_builtin: false,
            system_prompt: None,
            runtime: None,
            model: None,
            provider: None,
            parallelism: None,
            respond_to: None,
            respond_to_allowlist: vec![],
            name_pool: vec![],
            idle_timeout_seconds: None,
            max_turn_duration_seconds: None,
        },
        profile: AgentSnapshotProfile {
            display_name: "Test".to_string(),
            about: None,
            avatar_data_url: None,
            avatar_url: None,
        },
        memory: AgentSnapshotMemory {
            level: crate::managed_agents::agent_snapshot::MemoryLevel::Everything,
            entries: vec![AgentSnapshotMemoryEntry {
                slug: "core".to_string(),
                body: "Secret memory.".to_string(),
            }],
        },
    };
    let json_bytes = encode_snapshot_json(&snapshot).unwrap();
    let kind = snapshot_kind_for_filename("analyst.agent.png").unwrap();
    let result = ensure_bytes_match_kind(&json_bytes, kind);
    assert!(
        result.is_err(),
        ".agent.png filename with memory-bearing JSON bytes must be rejected"
    );
}

#[test]
fn fetch_boundary_json_filename_with_png_bytes_rejected() {
    use crate::managed_agents::agent_snapshot::{
        encode_snapshot_png, AgentSnapshot, AgentSnapshotDefinition, AgentSnapshotMemory,
        AgentSnapshotProfile, FORMAT_DISCRIMINATOR, FORMAT_VERSION,
    };
    let snapshot = AgentSnapshot {
        format: FORMAT_DISCRIMINATOR.to_string(),
        version: FORMAT_VERSION,
        definition: AgentSnapshotDefinition {
            name: "test".to_string(),
            source_is_builtin: false,
            system_prompt: None,
            runtime: None,
            model: None,
            provider: None,
            parallelism: None,
            respond_to: None,
            respond_to_allowlist: vec![],
            name_pool: vec![],
            idle_timeout_seconds: None,
            max_turn_duration_seconds: None,
        },
        profile: AgentSnapshotProfile {
            display_name: "Test".to_string(),
            about: None,
            avatar_data_url: None,
            avatar_url: None,
        },
        memory: AgentSnapshotMemory {
            level: crate::managed_agents::agent_snapshot::MemoryLevel::None,
            entries: vec![],
        },
    };
    let png_bytes = encode_snapshot_png(&snapshot, None).unwrap();
    // .agent.json filename → Json kind; PNG bytes must be rejected.
    let kind = snapshot_kind_for_filename("analyst.agent.json").unwrap();
    let result = ensure_bytes_match_kind(&png_bytes, kind);
    assert!(
        result.is_err(),
        ".agent.json filename with PNG bytes must be rejected by the magic-byte guard"
    );
    assert!(
        result.unwrap_err().contains("bytes are a PNG"),
        "error must describe the mismatch"
    );
}

#[test]
fn decode_boundary_json_none_level_with_entries_rejected() {
    use crate::commands::personas::decode_snapshot_from_bytes;
    // Construct JSON bytes directly: level=none but entries non-empty.
    // encode_snapshot_json does not guard against this, so we can produce it.
    let raw = serde_json::json!({
        "format": "buzz-agent-snapshot",
        "version": 1,
        "definition": { "name": "test" },
        "profile": { "displayName": "Test" },
        "memory": {
            "level": "none",
            "entries": [{"slug": "core", "body": "leaked"}]
        }
    });
    let bytes = serde_json::to_vec(&raw).unwrap();
    let result = decode_snapshot_from_bytes(&bytes);
    assert!(
        result.is_err(),
        "JSON with level:none + non-empty entries must be rejected by decode_snapshot_from_bytes"
    );
    assert!(
        result
            .unwrap_err()
            .contains("'none' but entries are present"),
        "error must describe the consistency violation"
    );
}

const RELAY_BASE: &str = "https://relay.example.com";

#[test]
fn test_validate_download_url_valid_relay_url() {
    assert!(validate_download_url(
        "https://relay.example.com/media/abcdef1234567890.jpg",
        RELAY_BASE,
    )
    .is_ok());
}

#[test]
fn test_validate_download_url_valid_relay_url_png() {
    assert!(
        validate_download_url("https://relay.example.com/media/abc123.png", RELAY_BASE,).is_ok()
    );
}

#[test]
fn test_validate_download_url_non_relay_origin_rejected() {
    let result = validate_download_url("https://evil.example.com/media/abc123.jpg", RELAY_BASE);
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("relay origin"));
}

#[test]
fn test_validate_download_url_private_ip_rejected() {
    let result = validate_download_url("http://169.254.169.254/latest/meta-data/", RELAY_BASE);
    assert!(result.is_err());
}

#[test]
fn test_validate_download_url_loopback_rejected() {
    let result = validate_download_url("http://127.0.0.1/media/abc.jpg", RELAY_BASE);
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("relay origin"));
}

#[test]
fn test_validate_download_url_localhost_allowed_for_localhost_relay() {
    assert!(validate_download_url(
        "http://localhost:3000/media/abc.jpg",
        "http://localhost:3000",
    )
    .is_ok());
}

#[test]
fn test_validate_download_url_missing_media_path_rejected() {
    let result = validate_download_url("https://relay.example.com/other/abc.jpg", RELAY_BASE);
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("/media/"));
}

#[test]
fn test_validate_download_url_non_https_scheme_rejected() {
    let result = validate_download_url("ftp://relay.example.com/media/abc.jpg", RELAY_BASE);
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("HTTPS"));
}

#[test]
fn test_validate_download_url_http_non_localhost_rejected() {
    let result = validate_download_url("http://relay.example.com/media/abc.jpg", RELAY_BASE);
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("HTTPS"));
}

#[test]
fn test_validate_download_url_root_path_rejected() {
    let result = validate_download_url("https://relay.example.com/", RELAY_BASE);
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("/media/"));
}

// Video Download reuses `download_file`, which runs the same
// `validate_download_url` gate as image download. `validate_download_url`
// is extension-agnostic (it only checks scheme, origin, and the `/media/`
// path prefix), so a relay-hosted mp4/webm passes exactly like an image,
// and an off-relay or private-host video is rejected exactly like an
// off-relay image. These cases pin that parity so a future change can't
// silently narrow the video download path's SSRF protection.
#[test]
fn test_validate_download_url_valid_relay_video_mp4() {
    assert!(validate_download_url(
        "https://relay.example.com/media/abcdef1234567890.mp4",
        RELAY_BASE,
    )
    .is_ok());
}

#[test]
fn test_validate_download_url_valid_relay_video_webm() {
    assert!(
        validate_download_url("https://relay.example.com/media/abc123.webm", RELAY_BASE).is_ok()
    );
}

#[test]
fn test_validate_download_url_non_relay_video_rejected() {
    let result = validate_download_url("https://evil.example.com/media/clip.mp4", RELAY_BASE);
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("relay origin"));
}

#[test]
fn test_validate_download_url_private_host_video_rejected() {
    // Off-relay private host serving a video must be rejected before any
    // fetch — same SSRF gate as image download.
    let result = validate_download_url("http://127.0.0.1/media/clip.mp4", RELAY_BASE);
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("relay origin"));
}

/// Redirect-hop SSRF guard: the media fetch client must NOT follow a 3xx,
/// and the command-facing error must identify the refused redirect.
///
/// `validate_download_url` only vets the *initial* URL, so a relay that
/// returned a redirect to an off-origin or private host would, under a
/// redirect-following client, forward the minted media Authorization
/// header across origins. The client `build_media_fetch_client()` produces
/// (the same one `fetch_blob_bytes_with_cap` uses via `AppState`) is built
/// with `redirect::Policy::none()`, so the 302 comes back verbatim and
/// `redirect_refusal_error` — the same mapping the command applies — turns
/// it into an actionable redirect error, not a silent cross-origin fetch.
///
/// A loopback `std::net::TcpListener` (no extra tokio feature) serves one
/// raw `302` pointing at an off-origin target and records how many
/// connections it accepts.
#[tokio::test]
async fn media_fetch_client_does_not_follow_redirects() {
    use std::io::{Read, Write};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let connections = Arc::new(AtomicUsize::new(0));

    let server_connections = Arc::clone(&connections);
    let server = std::thread::spawn(move || {
        // Accept exactly one connection; if the client followed the
        // redirect it would open a second one to the (unrelated) target,
        // but that target is never this server, so a second accept here
        // would only happen on an unexpected retry. We serve one 302 and
        // return, so the count stays at 1 for a compliant no-redirect client.
        if let Ok((mut stream, _)) = listener.accept() {
            server_connections.fetch_add(1, Ordering::SeqCst);
            let mut buf = [0u8; 1024];
            let _ = stream.read(&mut buf);
            let response = "HTTP/1.1 302 Found\r\n\
                 Location: http://169.254.169.254/latest/meta-data/\r\n\
                 Content-Length: 0\r\n\
                 Connection: close\r\n\r\n";
            let _ = stream.write_all(response.as_bytes());
            let _ = stream.flush();
        }
    });

    // Drive the exact client the command path uses, not an ad-hoc one.
    let client = crate::app_state::build_media_fetch_client()
        .expect("media fetch client must build with no-redirect policy");
    let resp = client
        .get(format!("http://{addr}/media/clip.mp4"))
        .timeout(std::time::Duration::from_secs(5))
        .send()
        .await
        .expect("request should complete without following the redirect");

    // The 302 is returned verbatim — not the 169.254.x target's response.
    assert_eq!(resp.status().as_u16(), 302);
    assert!(!resp.status().is_success());

    // The command maps that status through `redirect_refusal_error`; the
    // user-facing error must name the redirect, not read as a generic
    // relay failure.
    let err =
        redirect_refusal_error(resp.status()).expect("a 3xx must map to a redirect-refusal error");
    assert!(
        err.contains("redirect") && err.contains("302"),
        "error must identify the refused 302 redirect, got: {err}",
    );

    server.join().unwrap();
    assert_eq!(
        connections.load(Ordering::SeqCst),
        1,
        "exactly one request must be issued — the redirect must not be followed",
    );
}

#[test]
fn build_media_fetch_client_succeeds_with_no_redirect_policy() {
    // The fail-closed invariant: construction must not silently degrade to
    // a redirect-following client. If this ever starts failing, startup
    // panics loudly (see `build_app_state`) rather than substituting an
    // insecure client.
    assert!(
        crate::app_state::build_media_fetch_client().is_ok(),
        "media fetch client must build; a redirect-following fallback is forbidden",
    );
}

#[test]
fn redirect_refusal_error_only_fires_for_3xx() {
    // 3xx → redirect-identifying error; success/non-3xx → None (fall
    // through to the normal success or relay-error handling).
    assert!(redirect_refusal_error(reqwest::StatusCode::FOUND).is_some());
    assert!(redirect_refusal_error(reqwest::StatusCode::TEMPORARY_REDIRECT).is_some());
    assert!(redirect_refusal_error(reqwest::StatusCode::OK).is_none());
    assert!(redirect_refusal_error(reqwest::StatusCode::NOT_FOUND).is_none());
}

#[test]
fn markdown_doc_cap_is_two_mib() {
    assert_eq!(MAX_MARKDOWN_DOC_BYTES, 2 * 1024 * 1024);
}

#[test]
fn declared_length_over_markdown_cap_refused_before_body() {
    let err =
        declared_length_refusal_error(Some(MAX_MARKDOWN_DOC_BYTES + 1), MAX_MARKDOWN_DOC_BYTES)
            .expect("length just over the cap must be refused");
    assert!(err.contains("too large"), "{err}");
    // A 50 MiB advertisement passes the generic download cap but must be
    // refused by the markdown viewer's cap before any body bytes are read.
    assert!(
        declared_length_refusal_error(Some(MAX_DOWNLOAD_BYTES), MAX_MARKDOWN_DOC_BYTES).is_some()
    );
}

#[test]
fn declared_length_at_cap_or_absent_is_admitted() {
    assert!(
        declared_length_refusal_error(Some(MAX_MARKDOWN_DOC_BYTES), MAX_MARKDOWN_DOC_BYTES)
            .is_none()
    );
    // Absent Content-Length is admitted here by design — the streaming
    // accumulator below still owns the cap, so a stripped or dishonest
    // header cannot buy a larger download.
    assert!(declared_length_refusal_error(None, MAX_MARKDOWN_DOC_BYTES).is_none());
}

#[test]
fn streamed_bytes_hit_markdown_cap_without_content_length() {
    // Simulate a chunked transfer with no (or dishonest) Content-Length: the
    // running total must abort the moment a chunk would cross the cap, and
    // nothing past the cap may be buffered.
    let cap = MAX_MARKDOWN_DOC_BYTES;
    let mut bytes = Vec::new();
    let chunk = vec![b'a'; 1024 * 1024]; // 1 MiB chunks
    append_chunk_within_cap(&mut bytes, &chunk, cap).expect("first MiB fits");
    append_chunk_within_cap(&mut bytes, &chunk, cap).expect("exactly at the cap is allowed");
    assert_eq!(bytes.len() as u64, cap);
    let err = append_chunk_within_cap(&mut bytes, b"x", cap)
        .expect_err("one byte past the cap must abort");
    assert!(err.contains("too large"), "{err}");
    assert_eq!(
        bytes.len() as u64,
        cap,
        "no bytes past the cap were buffered"
    );
}
