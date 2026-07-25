use std::path::{Path, PathBuf};
use std::time::Duration;

use axum::http::StatusCode;
use buzz_local_relay::{serve, LocalRelay, StorageMode};
use futures_util::{SinkExt, StreamExt};
use nostr::Event;
use serde_json::{json, Value};
use tokio::net::TcpListener;
use tokio::task::JoinHandle;
use tokio_tungstenite::tungstenite::Message;
use uuid::Uuid;

struct RunningRelay {
    http_origin: String,
    websocket_url: String,
    task: JoinHandle<()>,
}

impl RunningRelay {
    async fn start(journal_path: PathBuf) -> Self {
        let relay = LocalRelay::open(StorageMode::Durable(journal_path))
            .await
            .expect("portable relay opens");
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("loopback listener binds");
        let address = listener.local_addr().expect("listener has an address");
        let task = tokio::spawn(async move {
            serve(listener, relay).await.expect("portable relay serves");
        });
        Self {
            http_origin: format!("http://{address}"),
            websocket_url: format!("ws://{address}/"),
            task,
        }
    }

    async fn stop(self) {
        self.task.abort();
        let _ = self.task.await;
    }
}

fn fixture_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../specs/fixtures/portable-relay/core-v0.1.json")
}

fn string_at<'a>(value: &'a Value, pointer: &str) -> &'a str {
    value
        .pointer(pointer)
        .and_then(Value::as_str)
        .unwrap_or_else(|| panic!("conformance vector must define {pointer}"))
}

async fn post_json(client: &reqwest::Client, origin: &str, path: &str, body: &Value) -> Value {
    let response = client
        .post(format!("{origin}{path}"))
        .json(body)
        .send()
        .await
        .expect("portable operation responds");
    assert_eq!(response.status(), StatusCode::OK);
    response
        .json()
        .await
        .expect("portable operation returns JSON")
}

async fn websocket_history(url: &str, message_type: &str, filters: &Value) -> Value {
    let (mut websocket, _) = tokio_tungstenite::connect_async(url)
        .await
        .expect("portable WebSocket connects");
    websocket
        .send(Message::Text(
            json!([message_type, "portable-conformance", filters[0]])
                .to_string()
                .into(),
        ))
        .await
        .expect("portable REQ sends");

    let mut observed_event = None;
    loop {
        let frame = tokio::time::timeout(Duration::from_secs(2), websocket.next())
            .await
            .expect("portable history frame arrives")
            .expect("portable WebSocket remains open")
            .expect("portable history frame is valid");
        let value: Value = serde_json::from_str(&frame.into_text().expect("history frame is text"))
            .expect("history frame is JSON");
        match value.pointer("/0").and_then(Value::as_str) {
            Some("EVENT") => observed_event = value.get(2).cloned(),
            Some("EOSE") => break,
            other => panic!("unexpected portable history frame: {other:?}"),
        }
    }
    observed_event.expect("portable history includes the fixture event")
}

#[tokio::test]
async fn laptop_adapter_preserves_the_portable_signed_event_vector() {
    let vector_path = fixture_path();
    let vector: Value = serde_json::from_str(
        &std::fs::read_to_string(&vector_path).expect("conformance vector reads"),
    )
    .expect("conformance vector parses");
    assert_eq!(string_at(&vector, "/profile"), "portable-relay-core-v0.1");

    let event_fixture = vector_path
        .parent()
        .expect("vector has a parent")
        .join(string_at(&vector, "/event_fixture"));
    let event_value: Value = serde_json::from_str(
        &std::fs::read_to_string(event_fixture).expect("signed event fixture reads"),
    )
    .expect("signed event fixture parses");
    let event: Event =
        serde_json::from_value(event_value.clone()).expect("signed event fixture is a Nostr event");
    assert_eq!(event.id.to_hex(), string_at(&vector, "/event_id"));

    let filters = vector
        .pointer("/operations/query/filters")
        .expect("vector defines query filters");
    let expected_ids = vector
        .pointer("/operations/query/expected_event_ids")
        .expect("vector defines expected query IDs");
    let expected_count = vector
        .pointer("/operations/count/expected")
        .and_then(Value::as_u64)
        .expect("vector defines expected count");
    let journal_path = std::env::temp_dir().join(format!(
        "buzz-portable-conformance-{}.ndjson",
        Uuid::new_v4()
    ));
    let client = reqwest::Client::new();
    assert_eq!(string_at(&vector, "/operations/submit/http/method"), "POST");
    assert_eq!(string_at(&vector, "/operations/query/http/method"), "POST");
    assert_eq!(string_at(&vector, "/operations/count/http/method"), "POST");

    let first_runtime = RunningRelay::start(journal_path.clone()).await;
    let submit_result = post_json(
        &client,
        &first_runtime.http_origin,
        string_at(&vector, "/operations/submit/http/path"),
        &event_value,
    )
    .await;
    assert_eq!(
        submit_result.pointer("/accepted"),
        vector.pointer("/operations/submit/expected/accepted")
    );
    assert_eq!(
        submit_result.pointer("/event_id"),
        vector.pointer("/operations/submit/expected/event_id")
    );

    let query_result = post_json(
        &client,
        &first_runtime.http_origin,
        string_at(&vector, "/operations/query/http/path"),
        filters,
    )
    .await;
    assert_eq!(query_result, json!([event_value.clone()]));
    let observed_ids: Vec<Value> = query_result
        .as_array()
        .expect("query result is an array")
        .iter()
        .map(|item| item["id"].clone())
        .collect();
    assert_eq!(&Value::Array(observed_ids), expected_ids);

    let websocket_event = websocket_history(
        &first_runtime.websocket_url,
        string_at(&vector, "/operations/query/websocket/message_type"),
        filters,
    )
    .await;
    assert_eq!(websocket_event, event_value);

    let duplicate_result = post_json(
        &client,
        &first_runtime.http_origin,
        string_at(&vector, "/operations/submit/http/path"),
        &event_value,
    )
    .await;
    assert_eq!(duplicate_result["accepted"], true);
    assert_eq!(duplicate_result["message"], "duplicate");

    let count_result = post_json(
        &client,
        &first_runtime.http_origin,
        string_at(&vector, "/operations/count/http/path"),
        filters,
    )
    .await;
    assert_eq!(count_result["count"], expected_count);
    first_runtime.stop().await;

    let second_runtime = RunningRelay::start(journal_path.clone()).await;
    let recovered = post_json(
        &client,
        &second_runtime.http_origin,
        string_at(&vector, "/operations/query/http/path"),
        filters,
    )
    .await;
    assert_eq!(recovered, json!([event_value]));
    let recovered_ids: Vec<Value> = recovered
        .as_array()
        .expect("recovered result is an array")
        .iter()
        .map(|item| item["id"].clone())
        .collect();
    assert_eq!(
        Value::Array(recovered_ids),
        vector["operations"]["restart"]["expected_event_ids"]
    );
    let recovered_count = post_json(
        &client,
        &second_runtime.http_origin,
        string_at(&vector, "/operations/count/http/path"),
        filters,
    )
    .await;
    assert_eq!(recovered_count["count"], expected_count);

    second_runtime.stop().await;
    std::fs::remove_file(journal_path).expect("test journal removes");
}
