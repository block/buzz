use super::*;
use axum::body::Bytes;
use axum::http::{HeaderMap, Method, StatusCode, Uri};
use axum::Router;
use tokio::sync::mpsc;

// Include duplicates, escaping, literal '+', nested schemes and a trailing slash.
const QUERY: &str = "token=a%2fb+%20&token=second&url=wss://other.example/&next=/";

#[tokio::test]
async fn endpoint_requests_preserve_path_query_and_signed_url() {
    for base_path in ["", "/", "/nostr", "/nostr/", "/n%2Fostr/"] {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let origin = format!("http://{address}");
        let (tx, mut rx) = mpsc::channel(16);
        let app = Router::new().fallback(
            move |method: Method, uri: Uri, headers: HeaderMap, _body: Bytes| {
                let tx = tx.clone();
                async move {
                    tx.send((method, uri.clone(), headers)).await.unwrap();
                    if uri.path().ends_with("/media/upload") {
                        (StatusCode::OK, r#"{"url":"http://example.test/media/blob","sha256":"abc","size":8,"type":"image/png","uploaded":0}"#)
                    } else if uri.path().ends_with("/upload") {
                        // Exercise the legacy fallback as well as the primary upload.
                        (StatusCode::NOT_FOUND, "{}")
                    } else {
                        (StatusCode::OK, "[]")
                    }
                }
            },
        );
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let base = normalize_relay_url(&format!("ws://{address}{base_path}?{QUERY}"));
        let client = BuzzClient::new(base, Keys::generate(), None, None).unwrap();
        let prefix = base_path.trim_end_matches('/');
        let filter = serde_json::json!({"kinds": [1]});

        // Each call below exercises a distinct production request construction site.
        let mut expected = Vec::new();
        client.get_public("/info").await.unwrap();
        expected.push(("GET", "/info".to_owned(), QUERY.to_owned(), false));
        client.query(&filter).await.unwrap();
        expected.push(("POST", "/query".to_owned(), QUERY.to_owned(), true));
        client.count(&filter).await.unwrap();
        expected.push(("POST", "/count".to_owned(), QUERY.to_owned(), true));
        client
            .get_authed("/moderation/reports?status=open&limit=20")
            .await
            .unwrap();
        expected.push((
            "GET",
            "/moderation/reports".to_owned(),
            format!("{QUERY}&status=open&limit=20"),
            true,
        ));
        client
            .post_json_authed("/gifs/search", &serde_json::json!({}))
            .await
            .unwrap();
        expected.push(("POST", "/gifs/search".to_owned(), QUERY.to_owned(), true));
        for kind in [1, 9040] {
            let event = EventBuilder::new(Kind::Custom(kind), "test")
                .sign_with_keys(client.keys())
                .unwrap();
            client.submit_event(event).await.unwrap();
            expected.push(("POST", "/events".to_owned(), QUERY.to_owned(), true));
        }
        let file = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(file.path(), b"\x89PNG\r\n\x1a\n").unwrap();
        client
            .upload_file(file.path().to_str().unwrap())
            .await
            .unwrap();
        for path in ["/upload", "/media/upload"] {
            expected.push(("PUT", path.to_owned(), QUERY.to_owned(), false));
        }
        let hash = "ab".repeat(32);
        client.download_media(&hash).await.unwrap();
        expected.push(("GET", format!("/media/{hash}"), QUERY.to_owned(), false));

        for (method, route, query, nip98) in expected {
            let (received_method, uri, headers) =
                tokio::time::timeout(Duration::from_secs(5), rx.recv())
                    .await
                    .unwrap()
                    .unwrap();
            assert_eq!(received_method, method);
            assert_eq!(uri.path(), format!("{prefix}{route}"));
            assert_eq!(uri.query(), Some(query.as_str()));
            if nip98 {
                let auth = headers["authorization"]
                    .to_str()
                    .unwrap()
                    .strip_prefix("Nostr ")
                    .unwrap();
                let event: nostr::Event =
                    serde_json::from_slice(&B64.decode(auth).unwrap()).unwrap();
                event.verify().unwrap();
                let signed_url = event.tags.iter().find(|t| t.as_slice()[0] == "u").unwrap();
                assert_eq!(signed_url.as_slice()[1], format!("{origin}{uri}"));
            }
        }
        assert!(rx.try_recv().is_err(), "unexpected extra request");
        server.abort();
    }
}
