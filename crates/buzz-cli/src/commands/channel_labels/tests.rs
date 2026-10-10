//! Real CLI dispatch over a loopback HTTP transport. The server controls receipts,
//! not application state; relay/Postgres tests establish the commit contract.
use super::*;
use axum::{
    body::Bytes,
    extract::State,
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
use base64::Engine;
use clap::Parser;
use nostr::Keys;
use std::{
    collections::VecDeque,
    path::PathBuf,
    sync::{Arc, Mutex},
};

#[derive(Clone)]
struct Reply {
    status: StatusCode,
    accepted: bool,
    message: &'static str,
    wrong_id: bool,
}

impl Reply {
    fn ok(accepted: bool, message: &'static str) -> Self {
        Self {
            status: StatusCode::OK,
            accepted,
            message,
            wrong_id: false,
        }
    }
}

#[derive(Default)]
struct ServerState {
    info: Value,
    replies: VecDeque<Reply>,
    sent: Vec<(Vec<u8>, Event)>,
    filters: Vec<Value>,
    snapshots: Vec<Event>,
    journal: Option<PathBuf>,
}

struct Fixture {
    client: BuzzClient,
    relay: Keys,
    state: Arc<Mutex<ServerState>>,
    task: tokio::task::JoinHandle<()>,
    temp: tempfile::TempDir,
}

async fn submit(
    State(state): State<Arc<Mutex<ServerState>>>,
    headers: HeaderMap,
    body: Bytes,
) -> impl IntoResponse {
    let event: Event = serde_json::from_slice(&body).unwrap();
    buzz_core::verify_event(&event).unwrap();
    let auth = headers["authorization"]
        .to_str()
        .unwrap()
        .strip_prefix("Nostr ")
        .unwrap();
    let auth: Event = serde_json::from_slice(
        &base64::engine::general_purpose::STANDARD
            .decode(auth)
            .unwrap(),
    )
    .unwrap();
    buzz_core::verify_event(&auth).unwrap();
    assert_eq!(auth.pubkey, event.pubkey);
    let mut state = state.lock().unwrap();
    let path = state.journal.as_ref().unwrap();
    let persisted = std::fs::read_to_string(path).unwrap();
    let mut lines = persisted.lines();
    let record: CommandRecord = serde_json::from_str(lines.next().unwrap()).unwrap();
    assert_eq!(
        record.event, event,
        "signed command must be on disk before transport"
    );
    assert_eq!(
        lines.last(),
        Some("\"unknown\""),
        "crash-safe state must precede transport"
    );
    assert!(
        Journal::open(path).is_err(),
        "another process must not send concurrently"
    );
    state.sent.push((body.to_vec(), auth));
    let reply = state
        .replies
        .pop_front()
        .expect("unexpected automatic send");
    (
        reply.status,
        Json(json!({
            "event_id": if reply.wrong_id { EventId::from_byte_array([0; 32]) } else { event.id },
            "accepted":reply.accepted, "message":reply.message,
        })),
    )
}

impl Fixture {
    async fn new(replies: Vec<Reply>) -> Self {
        let relay = Keys::generate();
        let state = Arc::new(Mutex::new(ServerState {
            info: json!({"supported_extensions":["nip-cl"],"self":relay.public_key()}),
            replies: replies.into(),
            ..Default::default()
        }));
        let app = Router::new()
            .route("/", get(|State(s): State<Arc<Mutex<ServerState>>>| async move {
                Json(s.lock().unwrap().info.clone())
            }))
            .route("/events", post(submit))
            .route("/query", post(|State(s): State<Arc<Mutex<ServerState>>>, Json(filter): Json<Value>| async move {
                let mut s = s.lock().unwrap();
                s.filters.push(filter);
                Json(s.snapshots.clone())
            }))
            .with_state(state.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let task = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        Self {
            client: BuzzClient::new(url, Keys::generate(), None, None).unwrap(),
            relay,
            state,
            task,
            temp: tempfile::tempdir().unwrap(),
        }
    }

    fn path(&self) -> PathBuf {
        self.temp.path().join("command.jsonl")
    }

    async fn update(&self) -> Result<(), CliError> {
        self.state.lock().unwrap().journal = Some(self.path());
        dispatch(
            ChannelLabelsCmd::Update {
                channel: Uuid::new_v4().to_string(),
                add: vec!["team:infra".into()],
                remove: vec![],
                command_file: self.path(),
                trusted_relay: Some(self.relay.public_key().to_hex()),
            },
            &self.client,
        )
        .await
    }

    async fn retry(&self) -> Result<(), CliError> {
        dispatch(
            ChannelLabelsCmd::Retry {
                command_file: self.path(),
            },
            &self.client,
        )
        .await
    }

    fn outcome(&self) -> CommandOutcome {
        Journal::open(&self.path()).unwrap().outcome
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        self.task.abort();
    }
}

#[tokio::test]
async fn lost_ack_then_rejection_stays_unknown_until_exact_positive_receipt() {
    let f = Fixture::new(vec![
        Reply {
            status: StatusCode::BAD_GATEWAY,
            ..Reply::ok(false, "proxy lost ack")
        },
        Reply::ok(
            false,
            "restricted: nip-cl-rejected access revoked or expired",
        ),
        Reply::ok(true, "duplicate: nip-cl-committed"),
    ])
    .await;
    assert!(matches!(
        f.update().await,
        Err(CliError::DeliveryUnknown(_))
    ));
    assert_eq!(f.outcome(), CommandOutcome::Unknown);
    let before = Journal::open(&f.path()).unwrap().record.event;
    assert!(matches!(f.retry().await, Err(CliError::DeliveryUnknown(_))));
    assert_eq!(f.outcome(), CommandOutcome::Unknown);
    f.retry().await.unwrap();
    assert_eq!(f.outcome(), CommandOutcome::Committed);
    // Known commit is terminal even after capability removal; no additional send.
    f.state.lock().unwrap().info = json!({});
    f.retry().await.unwrap();
    let state = f.state.lock().unwrap();
    assert_eq!(state.sent.len(), 3);
    for pair in state.sent.windows(2) {
        assert_eq!(
            pair[0].0, pair[1].0,
            "exact body across process-style reopen"
        );
        assert_ne!(
            pair[0].1.id, pair[1].1.id,
            "fresh NIP-98 proof, same command"
        );
    }
    assert_eq!(before, Journal::open(&f.path()).unwrap().record.event);
    assert!(
        state.filters.is_empty(),
        "current labels never establish a receipt"
    );
}

#[tokio::test]
async fn first_attempt_receipts_are_classified_without_automatic_transport_retries() {
    let mut cases = vec![
        (Reply::ok(true, ""), CommandOutcome::Committed),
        (
            Reply::ok(false, "error: nip-cl-unknown"),
            CommandOutcome::Unknown,
        ),
        (
            Reply::ok(false, "restricted: denied"),
            CommandOutcome::Unknown,
        ),
        (
            Reply::ok(false, "invalid: nip-cl-rejected-extra"),
            CommandOutcome::Unknown,
        ),
        (
            Reply {
                wrong_id: true,
                ..Reply::ok(true, "")
            },
            CommandOutcome::Unknown,
        ),
    ];
    for prefix in [
        "invalid",
        "auth-required",
        "restricted",
        "duplicate",
        "rate-limited",
        "error",
    ] {
        // The message is a static test fixture, bounded to six strings.
        let message = match prefix {
            "invalid" => "invalid: nip-cl-rejected",
            "auth-required" => "auth-required: nip-cl-rejected reason",
            "restricted" => "restricted: nip-cl-rejected reason",
            "duplicate" => "duplicate: nip-cl-rejected reason",
            "rate-limited" => "rate-limited: nip-cl-rejected reason",
            _ => "error: nip-cl-rejected reason",
        };
        cases.push((Reply::ok(false, message), CommandOutcome::Rejected));
    }
    for status in [
        StatusCode::TEMPORARY_REDIRECT,
        StatusCode::UNAUTHORIZED,
        StatusCode::TOO_MANY_REQUESTS,
        StatusCode::INTERNAL_SERVER_ERROR,
    ] {
        cases.push((
            Reply {
                status,
                ..Reply::ok(false, "generic transport error")
            },
            CommandOutcome::Unknown,
        ));
    }
    for (reply, expected) in cases {
        let f = Fixture::new(vec![reply]).await;
        let result = f.update().await;
        assert_eq!(f.outcome(), expected, "{result:?}");
        assert_eq!(result.is_ok(), expected == CommandOutcome::Committed);
        assert_eq!(
            matches!(result, Err(CliError::DeliveryUnknown(_))),
            expected == CommandOutcome::Unknown
        );
        assert_eq!(f.state.lock().unwrap().sent.len(), 1);
    }
}

#[tokio::test]
async fn capabilities_and_recovery_scope_fail_closed_before_send() {
    let f = Fixture::new(vec![Reply::ok(false, "error: nip-cl-unknown")]).await;
    assert!(
        capability(&f.client, None).await.is_err(),
        "plain HTTP needs explicit trust"
    );
    f.state.lock().unwrap().info["supported_extensions"] = json!([]);
    assert!(f.update().await.is_err());
    assert!(!f.path().exists());
    f.state.lock().unwrap().info["supported_extensions"] = json!(["nip-cl"]);
    f.update().await.unwrap_err();
    let other_author =
        BuzzClient::new(f.client.relay_url().into(), Keys::generate(), None, None).unwrap();
    let other_host = BuzzClient::new(
        f.client.relay_url().replace("127.0.0.1", "localhost"),
        f.client.keys().clone(),
        None,
        None,
    )
    .unwrap();
    for client in [&other_author, &other_host] {
        assert!(dispatch(
            ChannelLabelsCmd::Retry {
                command_file: f.path()
            },
            client
        )
        .await
        .is_err());
    }
    f.state.lock().unwrap().info["self"] = json!(Keys::generate().public_key());
    assert!(matches!(f.retry().await, Err(CliError::DeliveryUnknown(_))));
    assert_eq!(f.outcome(), CommandOutcome::Unknown);
    assert_eq!(f.state.lock().unwrap().sent.len(), 1);
}

#[tokio::test]
async fn labeled_and_unlabeled_create_use_the_persisted_cli_path() {
    for labels in [
        vec![],
        vec!["workflow/build".to_owned(), "team:infra".to_owned()],
    ] {
        let f = Fixture::new(vec![Reply::ok(true, "")]).await;
        f.state.lock().unwrap().journal = Some(f.path());
        super::super::channels::dispatch(
            crate::ChannelsCmd::Create {
                name: "builds".into(),
                channel_type: Some(crate::ChannelType::Forum),
                visibility: Some(crate::ChannelVisibility::Private),
                description: Some("CI".into()),
                ttl: None,
                template: None,
                templates_file: None,
                labels: labels.clone(),
                command_file: Some(f.path()),
                trusted_relay: Some(f.relay.public_key().to_hex()),
            },
            &f.client,
            &crate::OutputFormat::Json,
        )
        .await
        .unwrap();
        let record = Journal::open(&f.path()).unwrap();
        let Some(LabelCommand::Create { labels: actual, .. }) =
            LabelCommand::parse(&record.record.event).unwrap()
        else {
            panic!("not creation")
        };
        assert_eq!(actual, LabelSet::new(labels).unwrap());
        assert_eq!(record.outcome, CommandOutcome::Committed);
    }
}

#[test]
fn cli_requires_recovery_location_and_excludes_template_composition() {
    let channel = Uuid::new_v4().to_string();
    for args in [
        vec![
            "buzz",
            "channels",
            "labels",
            "update",
            "--channel",
            &channel,
            "--add-label",
            "a",
            "--command-file",
            "a.jsonl",
        ],
        vec![
            "buzz",
            "channels",
            "labels",
            "update",
            "--channel",
            &channel,
            "--remove-label",
            "a",
            "--command-file",
            "a.jsonl",
        ],
        vec![
            "buzz", "channels", "labels", "find", "--label", "a", "--label", "b", "--type",
            "stream",
        ],
        vec![
            "buzz",
            "channels",
            "labels",
            "retry",
            "--command-file",
            "a.jsonl",
        ],
    ] {
        assert!(crate::Cli::try_parse_from(args).is_ok());
    }
    for args in [
        vec![
            "buzz",
            "channels",
            "labels",
            "update",
            "--channel",
            &channel,
            "--add-label",
            "a",
        ],
        vec![
            "buzz",
            "channels",
            "labels",
            "update",
            "--channel",
            &channel,
            "--command-file",
            "a.jsonl",
        ],
        vec![
            "buzz",
            "channels",
            "create",
            "--name",
            "x",
            "--type",
            "stream",
            "--visibility",
            "open",
            "--label",
            "a",
        ],
        vec![
            "buzz",
            "channels",
            "create",
            "--name",
            "x",
            "--template",
            "test",
            "--command-file",
            "a.jsonl",
        ],
    ] {
        assert!(crate::Cli::try_parse_from(args).is_err());
    }
}

#[path = "journal_tests.rs"]
mod journal_tests;
#[path = "snapshot_tests.rs"]
mod snapshot_tests;
