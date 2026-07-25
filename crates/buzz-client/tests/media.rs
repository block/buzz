use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::body::{Body, Bytes};
use axum::extract::{Path, State};
use axum::http::{HeaderMap, Method, Response, StatusCode};
use axum::routing::any;
use axum::Router;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use buzz_client::{BuzzClient, BuzzClientConfig, BuzzIdentity, ClientError};
use nostr::{Event, JsonUtil, Keys, Kind};
use serde_json::json;
use sha2::{Digest, Sha256};

type Responder = dyn Fn(usize, &CapturedRequest) -> MediaResponse + Send + Sync;

#[derive(Clone)]
struct MediaState {
    requests: Arc<Mutex<Vec<CapturedRequest>>>,
    responder: Arc<Responder>,
}

#[derive(Clone)]
struct CapturedRequest {
    method: Method,
    path: String,
    headers: HeaderMap,
    body: Bytes,
}

struct MediaResponse {
    status: StatusCode,
    body: Vec<u8>,
    content_type: Option<&'static str>,
    location: Option<&'static str>,
}

impl MediaResponse {
    fn json(status: StatusCode, value: serde_json::Value) -> Self {
        Self {
            status,
            body: value.to_string().into_bytes(),
            content_type: Some("application/json"),
            location: None,
        }
    }
}

async fn media_handler(
    Path(path): Path<String>,
    State(state): State<MediaState>,
    method: Method,
    headers: HeaderMap,
    body: Bytes,
) -> Response<Body> {
    let request = CapturedRequest {
        method,
        path: format!("/{path}"),
        headers,
        body,
    };
    let attempt = {
        let mut requests = state.requests.lock().unwrap();
        requests.push(request.clone());
        requests.len()
    };
    let response = (state.responder)(attempt, &request);
    let mut builder = Response::builder()
        .status(response.status)
        .header("content-length", response.body.len());
    if let Some(content_type) = response.content_type {
        builder = builder.header("content-type", content_type);
    }
    if let Some(location) = response.location {
        builder = builder.header("location", location);
    }
    builder.body(Body::from(response.body)).unwrap()
}

async fn media_relay<F>(responder: F) -> (String, Arc<Mutex<Vec<CapturedRequest>>>)
where
    F: Fn(usize, &CapturedRequest) -> MediaResponse + Send + Sync + 'static,
{
    let requests = Arc::new(Mutex::new(Vec::new()));
    let state = MediaState {
        requests: requests.clone(),
        responder: Arc::new(responder),
    };
    let app = Router::new()
        .route("/{*path}", any(media_handler))
        .with_state(state);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    (format!("http://{address}"), requests)
}

fn client(relay_url: &str, with_auth_tag: bool) -> BuzzClient {
    let agent = Keys::generate();
    let auth_tag = with_auth_tag.then(|| {
        buzz_sdk::nip_oa::compute_auth_tag(&Keys::generate(), &agent.public_key(), "").unwrap()
    });
    let mut config = BuzzClientConfig::new(relay_url);
    config.retry_policy.max_retry_delay = Duration::ZERO;
    BuzzClient::new(
        config,
        BuzzIdentity::from_keys(agent, auth_tag.as_deref()).unwrap(),
    )
    .unwrap()
}

fn upload_descriptor(request: &CapturedRequest, mime_type: &str) -> serde_json::Value {
    let sha256 = hex::encode(Sha256::digest(&request.body));
    json!({
        "url": format!("http://relay.test/media/{sha256}"),
        "sha256": sha256,
        "size": request.body.len(),
        "type": mime_type,
        "uploaded": 1
    })
}

fn blossom_auth_event(headers: &HeaderMap) -> Event {
    let header = headers
        .get("authorization")
        .unwrap()
        .to_str()
        .unwrap()
        .strip_prefix("Nostr ")
        .unwrap();
    let json = URL_SAFE_NO_PAD.decode(header).unwrap();
    Event::from_json(json).unwrap()
}

fn tag_value<'a>(event: &'a Event, name: &str) -> Option<&'a str> {
    event.tags.iter().find_map(|tag| {
        let values = tag.as_slice();
        (values.first().map(String::as_str) == Some(name))
            .then(|| values.get(1).map(String::as_str))
            .flatten()
    })
}

#[tokio::test]
async fn upload_calculates_hash_and_forwards_blossom_and_owner_auth() {
    let bytes = b"portable document".to_vec();
    let expected_hash = hex::encode(Sha256::digest(&bytes));
    let (relay_url, requests) = media_relay(|_, request| {
        MediaResponse::json(
            StatusCode::OK,
            upload_descriptor(request, "application/pdf"),
        )
    })
    .await;
    let expected_server = relay_url.strip_prefix("http://").unwrap().to_string();
    let client = client(&relay_url, true);

    let descriptor = client
        .upload_bytes(bytes.clone(), "application/pdf")
        .await
        .unwrap();

    assert_eq!(descriptor.sha256, expected_hash);
    assert_eq!(descriptor.size, bytes.len() as u64);
    assert_eq!(descriptor.mime_type, "application/pdf");
    let requests = requests.lock().unwrap();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].method, Method::PUT);
    assert_eq!(requests[0].path, "/upload");
    assert_eq!(requests[0].headers["content-type"], "application/pdf");
    assert_eq!(requests[0].headers["x-sha-256"], expected_hash);
    assert!(requests[0].headers.contains_key("x-auth-tag"));
    let auth = blossom_auth_event(&requests[0].headers);
    auth.verify().unwrap();
    assert_eq!(auth.kind, Kind::Custom(24242));
    assert_eq!(tag_value(&auth, "t"), Some("upload"));
    assert_eq!(tag_value(&auth, "x"), Some(expected_hash.as_str()));
    assert_eq!(tag_value(&auth, "server"), Some(expected_server.as_str()));
}

#[tokio::test]
async fn upload_falls_back_only_when_primary_endpoint_is_absent() {
    let (relay_url, requests) = media_relay(|_, request| {
        if request.path == "/upload" {
            MediaResponse::json(StatusCode::NOT_FOUND, json!({"error": "missing"}))
        } else {
            MediaResponse::json(StatusCode::OK, upload_descriptor(request, "image/png"))
        }
    })
    .await;
    let client = client(&relay_url, false);

    assert!(client
        .upload_bytes(b"png".to_vec(), "image/png")
        .await
        .is_ok());
    let requests = requests.lock().unwrap();
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[0].path, "/upload");
    assert_eq!(requests[1].path, "/media/upload");
}

#[tokio::test]
async fn upload_retries_transient_failures_with_identical_bytes() {
    let (relay_url, requests) = media_relay(|attempt, request| {
        if attempt == 1 {
            MediaResponse::json(StatusCode::SERVICE_UNAVAILABLE, json!({"error": "retry"}))
        } else {
            MediaResponse::json(StatusCode::OK, upload_descriptor(request, "image/webp"))
        }
    })
    .await;
    let client = client(&relay_url, false);

    assert!(client
        .upload_bytes(b"image".to_vec(), "image/webp")
        .await
        .is_ok());
    let requests = requests.lock().unwrap();
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[0].body, requests[1].body);
}

#[tokio::test]
async fn upload_rejects_invalid_mime_and_mismatched_descriptor() {
    let (relay_url, requests) = media_relay(|_, _| {
        MediaResponse::json(
            StatusCode::OK,
            json!({
                "url": "http://relay.test/media/bad",
                "sha256": "0".repeat(64),
                "size": 99,
                "type": "image/png",
                "uploaded": 1
            }),
        )
    })
    .await;
    let client = client(&relay_url, false);

    assert!(matches!(
        client.upload_bytes(vec![1], "not-a-mime").await,
        Err(ClientError::InvalidMedia(_))
    ));
    assert!(requests.lock().unwrap().is_empty());
    assert!(matches!(
        client.upload_bytes(vec![1], "image/png").await,
        Err(ClientError::Protocol(_))
    ));
}

#[tokio::test]
async fn download_validates_path_and_returns_response_metadata() {
    let hash = "a".repeat(64);
    let expected_path = format!("/media/{hash}.png");
    let (relay_url, requests) = media_relay(move |_, request| {
        assert_eq!(request.path, expected_path);
        MediaResponse {
            status: StatusCode::OK,
            body: b"image bytes".to_vec(),
            content_type: Some("image/png"),
            location: None,
        }
    })
    .await;
    let expected_server = relay_url.strip_prefix("http://").unwrap().to_string();
    let client = client(&relay_url, true);

    let download = client.download_media(&format!("{hash}.png")).await.unwrap();

    assert_eq!(download.bytes, Bytes::from_static(b"image bytes"));
    assert_eq!(download.mime_type.as_deref(), Some("image/png"));
    assert_eq!(download.content_length, Some(11));
    let requests = requests.lock().unwrap();
    assert_eq!(requests[0].method, Method::GET);
    assert!(requests[0].headers.contains_key("x-auth-tag"));
    let auth = blossom_auth_event(&requests[0].headers);
    assert_eq!(tag_value(&auth, "t"), Some("get"));
    assert_eq!(tag_value(&auth, "server"), Some(expected_server.as_str()));
    assert!(tag_value(&auth, "x").is_none());
}

#[tokio::test]
async fn download_refuses_unsafe_or_cross_origin_inputs_before_request() {
    let (relay_url, requests) = media_relay(|_, _| MediaResponse {
        status: StatusCode::OK,
        body: Vec::new(),
        content_type: None,
        location: None,
    })
    .await;
    let client = client(&relay_url, false);
    for input in [
        "../secret",
        "abc.png",
        "a/evil".repeat(8).as_str(),
        "https://evil.example/media/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa.png",
        "ftp://relay.example/media/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa.png",
    ] {
        assert!(
            matches!(
                client.download_media(input).await,
                Err(ClientError::InvalidMedia(_))
            ),
            "{input:?} should be rejected"
        );
    }
    assert!(requests.lock().unwrap().is_empty());
}

#[tokio::test]
async fn download_does_not_follow_redirects_with_credentials() {
    let hash = "b".repeat(64);
    let (relay_url, requests) = media_relay(|_, _| MediaResponse {
        status: StatusCode::FOUND,
        body: Vec::new(),
        content_type: None,
        location: Some("https://evil.example/stolen"),
    })
    .await;
    let client = client(&relay_url, true);

    assert!(matches!(
        client.download_media(&hash).await,
        Err(ClientError::Relay { status: 302, .. })
    ));
    assert_eq!(requests.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn download_retries_transient_failures() {
    let hash = "c".repeat(64);
    let (relay_url, requests) = media_relay(|attempt, _| {
        if attempt == 1 {
            MediaResponse::json(StatusCode::BAD_GATEWAY, json!({"error": "retry"}))
        } else {
            MediaResponse {
                status: StatusCode::OK,
                body: b"media".to_vec(),
                content_type: Some("application/octet-stream"),
                location: None,
            }
        }
    })
    .await;
    let client = client(&relay_url, false);

    assert_eq!(
        client.download_media(&hash).await.unwrap().bytes,
        Bytes::from_static(b"media")
    );
    assert_eq!(requests.lock().unwrap().len(), 2);
}
