//! Native read-error tests: the production reader against a raw HTTP stub.

use super::super::read_stub::{response, serve_raw, signer_of};
use super::*;

async fn read_error(responses: Vec<Vec<u8>>) -> serde_json::Value {
    let (port, _) = serve_raw(responses);
    let url = format!(
        "http://127.0.0.1:{port}/api/admin/v1/events/{}",
        "ab".repeat(32)
    );
    let err = send_admin_read(&nostr::Keys::generate(), &url, SUCCESS_JSON_CAP)
        .await
        .unwrap_err();
    serde_json::to_value(err).unwrap()
}

fn meta(v: &serde_json::Value) -> serde_json::Value {
    serde_json::json!({
        "relayStatus": v["relayStatus"],
        "bodyComplete": v["bodyComplete"],
        "bodyEmpty": v["bodyEmpty"],
        "code": v["code"],
    })
}

#[tokio::test]
async fn empty_404_and_405_read_as_complete_and_empty() {
    for status in ["404 Not Found", "405 Method Not Allowed"] {
        let v = read_error(vec![response(status, "")]).await;
        let code: u16 = status[..3].parse().unwrap();
        assert_eq!(
            meta(&v),
            serde_json::json!({"relayStatus": code, "bodyComplete": true, "bodyEmpty": true, "code": null})
        );
    }
}

#[tokio::test]
async fn coded_404_carries_the_relay_code() {
    let body = r#"{"error":{"code":"event_not_found","message":"no such event"}}"#;
    let v = read_error(vec![response("404 Not Found", body)]).await;
    assert_eq!(
        meta(&v),
        serde_json::json!({"relayStatus": 404, "bodyComplete": true, "bodyEmpty": false, "code": "event_not_found"})
    );
}

#[tokio::test]
async fn truncated_and_over_cap_bodies_are_incomplete() {
    // Declares 100 bytes, sends 2, then closes: the stream errors mid-body.
    let truncated =
        b"HTTP/1.1 404 Not Found\r\nContent-Length: 100\r\nConnection: close\r\n\r\n{}".to_vec();
    let over_cap = format!(
        "HTTP/1.1 404 Not Found\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        ERROR_BODY_CAP + 1
    )
    .into_bytes();
    for raw in [truncated, over_cap] {
        let v = read_error(vec![raw]).await;
        assert_eq!(
            meta(&v),
            serde_json::json!({"relayStatus": 404, "bodyComplete": false, "bodyEmpty": false, "code": null})
        );
    }
}

#[tokio::test]
async fn transport_failure_has_no_status() {
    client::init_admin_client().expect("client builds");
    let port = std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port(); // listener dropped: connection refused
    let url = format!("http://127.0.0.1:{port}/api/admin/v1/communities");
    let err = send_admin_read(&nostr::Keys::generate(), &url, SUCCESS_JSON_CAP)
        .await
        .unwrap_err();
    assert_eq!(
        meta(&serde_json::to_value(err).unwrap()),
        serde_json::json!({"relayStatus": null, "bodyComplete": false, "bodyEmpty": false, "code": null})
    );
}

#[tokio::test]
async fn a_401_retry_is_signed_by_the_same_keys() {
    let keys = nostr::Keys::generate();
    let (port, seen) = serve_raw(vec![
        response("401 Unauthorized", ""),
        response("200 OK", r#"{"items":[],"nextCursor":null}"#),
    ]);
    let url = format!("http://127.0.0.1:{port}/api/admin/v1/communities?q=te");
    let bytes = send_admin_read(&keys, &url, SUCCESS_JSON_CAP)
        .await
        .unwrap();
    assert_eq!(bytes, br#"{"items":[],"nextCursor":null}"#);
    let seen = seen.lock().unwrap();
    assert_eq!(seen.len(), 2);
    for request in seen.iter() {
        assert!(
            request.starts_with("GET /api/admin/v1/communities?q=te "),
            "{request}"
        );
        assert_eq!(signer_of(request), keys.public_key().to_hex());
    }
}

#[tokio::test]
async fn a_search_carrying_a_key_backup_is_never_sent() {
    client::init_admin_client().expect("client builds");
    let backup = "ncryptsec1qgg9947rlpvqu76pj5ecreduf9jxhselq2nae2kghhvd5g7dgjtc";
    for q in [
        backup.to_string(),
        format!("alice {backup} bob"),
        backup.to_ascii_uppercase(),
    ] {
        let (port, seen) = serve_raw(vec![response("200 OK", "{}")]);
        let query = routes::AdminQuery {
            q: Some(q),
            ..Default::default()
        };
        let url = format!(
            "http://127.0.0.1:{port}/api/admin/v1/communities?{}",
            query.to_query_string()
        );
        let err = send_admin_read(&nostr::Keys::generate(), &url, SUCCESS_JSON_CAP)
            .await
            .unwrap_err();
        assert!(err.message.contains("NIP-49 key-backup"), "{}", err.message);
        assert!(seen.lock().unwrap().is_empty(), "request reached the relay");
    }
}

#[test]
fn read_urls_validate_the_explicit_host_and_encode_the_query() {
    let url = read_url(
        "https://admin.example.com",
        &routes::AdminRoute::MembersSearch,
        routes::AdminQuery {
            community_host: Some(" Team.Example.com ".to_string()),
            q: Some("al ice&x".to_string()),
            ..Default::default()
        },
    )
    .unwrap();
    assert!(
        url.starts_with("https://admin.example.com/api/admin/v1/members/search?"),
        "{url}"
    );
    assert!(url.contains("communityHost=team.example.com"), "{url}");
    assert!(url.contains("q=al+ice%26x"), "{url}");
    let bad = read_url(
        "https://admin.example.com",
        &routes::AdminRoute::MembersSearch,
        routes::AdminQuery {
            community_host: Some("https://team.example.com/x".to_string()),
            ..Default::default()
        },
    );
    assert!(bad.unwrap_err().starts_with("invalid community host"));
}
