use super::*;
use crate::error::exit_code;
use crate::{Cli, Cmd};
use axum::extract::State;
use axum::http::StatusCode;
use axum::routing::post;
use axum::{Json, Router};
use buzz_core::artifact::ArtifactOp as RelayOp;
use clap::Parser;
use nostr::Keys;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

const CH_A: &str = "9b353519-f4fe-4757-aef4-bec6cc0ae54c";
const CH_B: &str = "17cfe3ca-b3d0-4a76-a2a4-4eaf0d3a4109";

struct Head {
    event: Value,
    event_id: String,
    channel: String,
    artifact_type: String,
    deleted: bool,
}

/// A relay that applies NIP-AR's compare-and-set rules the way the real relay
/// does (`buzz-db` `accept_artifact`), so commands run against real semantics.
#[derive(Default)]
struct Relay {
    heads: HashMap<String, Head>,
    seen: Vec<String>,
    writes: Vec<Value>,
    queries: Vec<Value>,
    channels: HashMap<String, Value>,
    reject_writes: Option<(StatusCode, String)>,
    reply_override: Option<Value>,
    current_override: Option<Vec<Value>>,
    races_left: usize,
}

type Shared = Arc<Mutex<Relay>>;

impl Relay {
    fn apply(&mut self, raw: &Value) -> Result<(), (StatusCode, String)> {
        let invalid = |m: String| (StatusCode::BAD_REQUEST, format!("invalid: {m}"));
        let conflict = |m: &str| (StatusCode::CONFLICT, format!("conflict: {m}"));
        let event: nostr::Event =
            serde_json::from_value(raw.clone()).map_err(|e| invalid(e.to_string()))?;
        let env = artifact::validate(&event).map_err(|e| invalid(e.into()))?;
        let event_id = event.id.to_hex();
        if self.seen.contains(&event_id) {
            return Ok(());
        }
        let (d, home) = (env.id.to_string(), env.home.to_string());
        let prev = env.prev.as_ref().map(hex::encode);
        match (self.heads.get(&d), env.op) {
            (Some(_), RelayOp::Create) => return Err(conflict("artifact identity is taken")),
            (None, RelayOp::Create) => {}
            (None, _) => return Err(conflict("artifact head unavailable")),
            (Some(head), op) => {
                if op != RelayOp::Move && head.channel != home {
                    return Err(conflict("artifact head unavailable"));
                }
                if prev.as_deref() != Some(head.event_id.as_str()) {
                    return Err(conflict("artifact head changed"));
                }
                if env.artifact_type != head.artifact_type {
                    return Err(invalid("artifact type is immutable".into()));
                }
                if head.deleted != (op == RelayOp::Restore) {
                    return Err(invalid("deleted artifacts require restore".into()));
                }
                if (op == RelayOp::Move) != (head.channel != home) {
                    return Err(invalid("only move changes home".into()));
                }
            }
        }
        self.heads.insert(
            d,
            Head {
                event: raw.clone(),
                event_id: event_id.clone(),
                channel: home,
                artifact_type: env.artifact_type,
                deleted: env.op == RelayOp::Delete,
            },
        );
        self.seen.push(event_id);
        self.writes.push(raw.clone());
        Ok(())
    }

    fn compete(&mut self, d: &str) {
        let head = &self.heads[d];
        let revision = ArtifactRevision {
            artifact_id: Uuid::parse_str(d).unwrap(),
            channel_id: Uuid::parse_str(&head.channel).unwrap(),
            artifact_type: &head.artifact_type,
            op: ArtifactOp::Update,
            prev: Some(&head.event_id),
            root: None,
            title: Some("Competing edit"),
            tags: &[],
            content: "competing",
        };
        let event = build_artifact_revision(&revision)
            .unwrap()
            .sign_with_keys(&Keys::generate())
            .unwrap();
        let raw = serde_json::to_value(event).unwrap();
        self.apply(&raw).unwrap();
    }

    fn answer(&mut self, filter: &Value) -> Value {
        self.queries.push(filter.clone());
        if filter["kinds"] == json!([39000]) {
            let id = filter["#d"][0].as_str().unwrap_or_default();
            return json!(self.channels.get(id).into_iter().collect::<Vec<_>>());
        }
        let wanted = |name: &str, value: &str| {
            filter
                .get(name)
                .and_then(Value::as_array)
                .is_none_or(|values| values.iter().any(|v| v == value))
        };
        let mut heads: Vec<(&String, &Head)> = self.heads.iter().collect();
        heads.sort_by_key(|(d, _)| (*d).clone());
        match filter["artifact"].as_str() {
            Some("current") => {
                if let Some(events) = &self.current_override {
                    return json!(events);
                }
                let limit = filter["limit"].as_u64().unwrap() as usize;
                json!(heads
                    .into_iter()
                    .filter(|(_, h)| !h.deleted
                        && wanted("#h", &h.channel)
                        && wanted("#type", &h.artifact_type))
                    .take(limit)
                    .map(|(_, h)| h.event.clone())
                    .collect::<Vec<_>>())
            }
            Some("head") => {
                let d = filter["#d"][0].as_str().unwrap().to_string();
                let reply = json!(self
                    .heads
                    .get(&d)
                    .map(|h| h.event.clone())
                    .into_iter()
                    .collect::<Vec<_>>());
                if self.races_left > 0 && self.heads.contains_key(&d) {
                    self.races_left -= 1;
                    self.compete(&d);
                }
                reply
            }
            other => panic!("unexpected artifact view {other:?}"),
        }
    }
}

struct Harness {
    relay: Shared,
    client: BuzzClient,
    server: tokio::task::JoinHandle<()>,
}

impl Drop for Harness {
    fn drop(&mut self) {
        self.server.abort();
    }
}

impl Harness {
    async fn start() -> Self {
        let relay: Shared = Arc::default();
        let app = Router::new()
            .route(
                "/events",
                post(|State(r): State<Shared>, Json(event): Json<Value>| async move {
                    let mut relay = r.lock().unwrap();
                    if let Some((status, message)) = relay.reject_writes.clone() {
                        return (status, Json(json!({"error": message})));
                    }
                    match relay.apply(&event) {
                        Ok(()) => (
                            StatusCode::OK,
                            Json(relay.reply_override.clone().unwrap_or_else(
                                || json!({"event_id": event["id"], "accepted": true, "message": ""}),
                            )),
                        ),
                        Err((status, message)) => (status, Json(json!({"error": message}))),
                    }
                }),
            )
            .route(
                "/query",
                post(|State(r): State<Shared>, Json(filters): Json<Vec<Value>>| async move {
                    Json(r.lock().unwrap().answer(&filters[0]))
                }),
            )
            .with_state(relay.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        Self {
            relay,
            client: BuzzClient::new(url, Keys::generate(), None, None).unwrap(),
            server,
        }
    }

    async fn run(&self, args: &[&str]) -> Result<String, CliError> {
        let mut argv = vec!["buzz", "artifacts"];
        argv.extend_from_slice(args);
        let Cli {
            command: Cmd::Artifacts(cmd),
            format,
            ..
        } = Cli::try_parse_from(argv).expect("arguments parse")
        else {
            panic!("not an artifacts command");
        };
        super::run(cmd, &self.client, &format).await
    }

    async fn ok(&self, args: &[&str]) -> Value {
        serde_json::from_str(&self.run(args).await.expect("command succeeds")).unwrap()
    }

    async fn create(&self, channel: &str, artifact_type: &str, title: &str) -> String {
        let out = self
            .ok(&[
                "create",
                "--channel",
                channel,
                "--type",
                artifact_type,
                "--title",
                title,
            ])
            .await;
        out["artifact_id"].as_str().unwrap().to_string()
    }

    fn writes(&self) -> Vec<Value> {
        self.relay.lock().unwrap().writes.clone()
    }

    fn queries(&self) -> Vec<Value> {
        self.relay.lock().unwrap().queries.clone()
    }
}

fn tags(event: &Value) -> Vec<Vec<String>> {
    event["tags"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| {
            t.as_array()
                .unwrap()
                .iter()
                .map(|v| v.as_str().unwrap().to_string())
                .collect()
        })
        .collect()
}

fn tag(event: &Value, name: &str) -> Option<String> {
    tags(event)
        .into_iter()
        .find(|t| t[0] == name)
        .map(|t| t[1].clone())
}

fn names(event: &Value) -> Vec<String> {
    tags(event).into_iter().map(|t| t[0].clone()).collect()
}

fn s(parts: &[&str]) -> Vec<String> {
    parts.iter().map(|p| p.to_string()).collect()
}

#[tokio::test]
async fn every_write_publishes_the_tag_shape_the_nip_defines() {
    let h = Harness::start().await;
    let out = h
        .ok(&[
            "create",
            "--channel",
            CH_A,
            "--type",
            "buzz.task",
            "--title",
            "Fix timeout",
            "--content",
            r#"{"version":1}"#,
        ])
        .await;
    assert_eq!(out["accepted"], true);
    let id = out["artifact_id"].as_str().unwrap().to_string();

    let create = h.writes()[0].clone();
    assert_eq!(
        tags(&create),
        vec![
            s(&["ar", "1"]),
            s(&["d", &id]),
            s(&["h", CH_A]),
            s(&["type", "buzz.task"]),
            s(&["title", "Fix timeout"]),
            s(&["op", "create"]),
        ]
    );
    assert_eq!(create["content"], r#"{"version":1}"#);
    assert_eq!(create["kind"], 45010);
    assert_eq!(out["event_id"], create["id"]);

    h.ok(&[
        "update",
        "--channel",
        CH_A,
        "--id",
        &id,
        "--title",
        "Fix the timeout",
    ])
    .await;
    let update = h.writes()[1].clone();
    assert_eq!(
        tags(&update),
        vec![
            s(&["ar", "1"]),
            s(&["d", &id]),
            s(&["h", CH_A]),
            s(&["type", "buzz.task"]),
            s(&["title", "Fix the timeout"]),
            s(&["op", "update"]),
            s(&["prev", create["id"].as_str().unwrap()]),
        ]
    );
    assert_eq!(
        update["content"], r#"{"version":1}"#,
        "omitted content is kept"
    );

    h.ok(&[
        "move",
        "--channel",
        CH_A,
        "--id",
        &id,
        "--to",
        CH_B,
        "--yes",
    ])
    .await;
    let moved = h.writes()[2].clone();
    assert_eq!(
        tags(&moved),
        vec![
            s(&["ar", "1"]),
            s(&["d", &id]),
            s(&["h", CH_B]),
            s(&["type", "buzz.task"]),
            s(&["title", "Fix the timeout"]),
            s(&["op", "move"]),
            s(&["prev", update["id"].as_str().unwrap()]),
        ]
    );
    assert_eq!(moved["content"], r#"{"version":1}"#);

    h.ok(&["delete", "--channel", CH_B, "--id", &id]).await;
    let deleted = h.writes()[3].clone();
    assert_eq!(
        tags(&deleted),
        vec![
            s(&["ar", "1"]),
            s(&["d", &id]),
            s(&["h", CH_B]),
            s(&["type", "buzz.task"]),
            s(&["op", "delete"]),
            s(&["prev", moved["id"].as_str().unwrap()]),
        ]
    );
    assert_eq!(deleted["content"], "");

    h.ok(&[
        "restore",
        "--channel",
        CH_B,
        "--id",
        &id,
        "--title",
        "Back again",
        "--content",
        "{}",
    ])
    .await;
    let restored = h.writes()[4].clone();
    assert_eq!(
        tags(&restored),
        vec![
            s(&["ar", "1"]),
            s(&["d", &id]),
            s(&["h", CH_B]),
            s(&["type", "buzz.task"]),
            s(&["title", "Back again"]),
            s(&["op", "restore"]),
            s(&["prev", deleted["id"].as_str().unwrap()]),
        ]
    );
}

#[tokio::test]
async fn edits_keep_client_tags_and_root_but_delete_and_move_follow_the_nip() {
    let h = Harness::start().await;
    let (id, root) = (Uuid::new_v4().to_string(), "e".repeat(64));
    let extra = [s(&["project", "p1"]), s(&["assignee", "someone"])];
    let seeded = build_artifact_revision(&ArtifactRevision {
        artifact_id: Uuid::parse_str(&id).unwrap(),
        channel_id: Uuid::parse_str(CH_A).unwrap(),
        artifact_type: "buzz.task",
        op: ArtifactOp::Create,
        prev: None,
        root: Some(&root),
        title: Some("Seeded"),
        tags: &extra,
        content: "old",
    })
    .unwrap()
    .sign_with_keys(&Keys::generate())
    .unwrap();
    h.relay
        .lock()
        .unwrap()
        .apply(&serde_json::to_value(seeded).unwrap())
        .unwrap();

    h.ok(&["update", "--channel", CH_A, "--id", &id, "--content", "new"])
        .await;
    let update = h.writes()[1].clone();
    assert_eq!(tag(&update, "root"), Some(root.clone()));
    assert_eq!(tag(&update, "project"), Some("p1".into()));
    assert_eq!(tag(&update, "assignee"), Some("someone".into()));
    assert_eq!(tag(&update, "title"), Some("Seeded".into()));
    assert_eq!(update["content"], "new");

    h.ok(&[
        "move",
        "--channel",
        CH_A,
        "--id",
        &id,
        "--to",
        CH_B,
        "--yes",
    ])
    .await;
    let moved = h.writes()[2].clone();
    assert_eq!(
        tag(&moved, "root"),
        None,
        "the anchor belongs to the source"
    );
    assert_eq!(tag(&moved, "project"), Some("p1".into()));

    let id2 = Uuid::new_v4().to_string();
    let seeded = build_artifact_revision(&ArtifactRevision {
        artifact_id: Uuid::parse_str(&id2).unwrap(),
        channel_id: Uuid::parse_str(CH_A).unwrap(),
        artifact_type: "buzz.task",
        op: ArtifactOp::Create,
        prev: None,
        root: Some(&root),
        title: Some("Anchored"),
        tags: &extra,
        content: "x",
    })
    .unwrap()
    .sign_with_keys(&Keys::generate())
    .unwrap();
    h.relay
        .lock()
        .unwrap()
        .apply(&serde_json::to_value(seeded).unwrap())
        .unwrap();
    h.ok(&["delete", "--channel", CH_A, "--id", &id2]).await;
    let deleted = h.writes().last().unwrap().clone();
    assert_eq!(tag(&deleted, "root"), Some(root), "delete preserves root");
    assert_eq!(
        names(&deleted),
        ["ar", "d", "h", "type", "op", "root", "prev"],
        "delete drops client tags and title"
    );
}

#[tokio::test]
async fn list_returns_only_current_heads_after_an_edit_and_a_deletion() {
    let h = Harness::start().await;
    let a = h.create(CH_A, "buzz.task", "A").await;
    let b = h.create(CH_A, "buzz.task", "B").await;
    let c = h.create(CH_A, "buzz.note", "C").await;
    h.create(CH_B, "buzz.task", "elsewhere").await;
    h.ok(&["update", "--channel", CH_A, "--id", &a, "--title", "A2"])
        .await;
    h.ok(&["delete", "--channel", CH_A, "--id", &b]).await;

    let all = h.ok(&["list", "--channel", CH_A]).await;
    let mut titles: Vec<(String, String)> = all
        .as_array()
        .unwrap()
        .iter()
        .map(|e| (tag(e, "d").unwrap(), tag(e, "title").unwrap()))
        .collect();
    titles.sort();
    let mut expected = vec![(a.clone(), "A2".to_string()), (c.clone(), "C".to_string())];
    expected.sort();
    assert_eq!(titles, expected);
    assert_eq!(
        all.as_array()
            .unwrap()
            .iter()
            .map(|e| tag(e, "op").unwrap())
            .filter(|op| op == "delete")
            .count(),
        0
    );

    let tasks = h
        .ok(&[
            "list",
            "--channel",
            CH_A,
            "--type",
            "buzz.task",
            "--limit",
            "5",
        ])
        .await;
    assert_eq!(tasks.as_array().unwrap().len(), 1);
    assert_eq!(tag(&tasks[0], "d"), Some(a));

    let filter = h.queries().last().unwrap().clone();
    assert_eq!(
        filter,
        json!({"kinds": [45010], "artifact": "current", "#h": [CH_A], "#type": ["buzz.task"], "limit": 5})
    );
    assert_eq!(
        h.queries()
            .iter()
            .find(|q| q["artifact"] == "current")
            .unwrap()["limit"],
        100,
        "default page is bounded"
    );
}

#[tokio::test]
async fn list_refuses_a_reply_that_cannot_be_a_set_of_heads() {
    let h = Harness::start().await;
    let a = h.create(CH_A, "buzz.task", "A").await;
    let head = h.relay.lock().unwrap().heads[&a].event.clone();
    let mut foreign = head.clone();
    foreign["kind"] = json!(1);
    let mut other_channel = head.clone();
    other_channel["tags"] = json!(tags(&head)
        .into_iter()
        .map(|t| if t[0] == "h" { s(&["h", CH_B]) } else { t })
        .collect::<Vec<_>>());
    for reply in [
        vec![foreign],
        vec![head.clone(), head.clone()],
        vec![other_channel],
    ] {
        h.relay.lock().unwrap().current_override = Some(reply);
        let err = h.run(&["list", "--channel", CH_A]).await.unwrap_err();
        assert!(matches!(err, CliError::Other(_)), "{err}");
        assert_eq!(exit_code(&err), 4);
    }
}

#[tokio::test]
async fn list_drops_a_deletion_the_relay_should_not_have_sent() {
    let h = Harness::start().await;
    let a = h.create(CH_A, "buzz.task", "A").await;
    let b = h.create(CH_A, "buzz.task", "B").await;
    h.ok(&["delete", "--channel", CH_A, "--id", &b]).await;
    let (live, gone) = {
        let relay = h.relay.lock().unwrap();
        (relay.heads[&a].event.clone(), relay.heads[&b].event.clone())
    };
    assert_eq!(tag(&gone, "op"), Some("delete".into()));
    h.relay.lock().unwrap().current_override = Some(vec![live, gone]);
    let listed = h.ok(&["list", "--channel", CH_A]).await;
    let ids: Vec<_> = listed
        .as_array()
        .unwrap()
        .iter()
        .map(|e| tag(e, "d"))
        .collect();
    assert_eq!(ids, vec![Some(a)]);
}

#[tokio::test]
async fn get_reports_not_found_with_exit_1_for_every_unreadable_case() {
    let h = Harness::start().await;
    let id = h.create(CH_A, "buzz.task", "A").await;
    let ok = h.ok(&["get", "--channel", CH_A, "--id", &id]).await;
    assert_eq!(ok.as_array().unwrap().len(), 1);
    assert_eq!(tag(&ok[0], "title"), Some("A".into()));
    assert_eq!(
        h.queries()[0],
        json!({"kinds": [45010], "artifact": "head", "#d": [id.clone()]})
    );

    let unknown = Uuid::new_v4().to_string();
    let wrong = h
        .run(&["get", "--channel", CH_B, "--id", &id])
        .await
        .unwrap_err();
    assert_eq!(
        exit_code(&wrong),
        1,
        "a live artifact in another channel: {wrong}"
    );
    h.ok(&["delete", "--channel", CH_A, "--id", &id]).await;
    for (channel, target) in [
        (CH_A, unknown.as_str()),
        (CH_B, id.as_str()),
        (CH_A, id.as_str()),
    ] {
        let err = h
            .run(&["get", "--channel", channel, "--id", target])
            .await
            .unwrap_err();
        assert!(matches!(err, CliError::NotFound(_)), "{err}");
        assert_eq!(exit_code(&err), 1);
    }
    for args in [
        [
            "update",
            "--channel",
            CH_A,
            "--id",
            &unknown,
            "--title",
            "t",
        ],
        ["delete", "--channel", CH_A, "--id", &unknown, "", ""],
    ] {
        let args: Vec<&str> = args.iter().copied().filter(|a| !a.is_empty()).collect();
        let err = h.run(&args).await.unwrap_err();
        assert_eq!(exit_code(&err), 1, "{args:?}");
    }
    assert_eq!(h.writes().len(), 2, "not-found writes nothing");
}

#[tokio::test]
async fn relay_rejections_map_to_relay_and_auth_exit_codes() {
    let h = Harness::start().await;
    for (status, message, code) in [
        (StatusCode::BAD_REQUEST, "invalid: nope", 2),
        (StatusCode::FORBIDDEN, "restricted: not a member", 3),
        (StatusCode::SERVICE_UNAVAILABLE, "artifact storage busy", 2),
    ] {
        h.relay.lock().unwrap().reject_writes = Some((status, message.into()));
        let err = h
            .run(&[
                "create",
                "--channel",
                CH_A,
                "--type",
                "buzz.task",
                "--title",
                "t",
            ])
            .await
            .unwrap_err();
        assert!(
            matches!(err, CliError::Relay { .. } | CliError::DeliveryUnknown(_)),
            "{err}"
        );
        assert_eq!(exit_code(&err), code, "{status}");
    }
    assert!(h.writes().is_empty());
}

#[tokio::test]
async fn a_2xx_reply_that_does_not_say_accepted_is_a_failure() {
    let h = Harness::start().await;
    for reply in [
        json!({"event_id": "x", "accepted": false, "message": "blocked"}),
        json!({}),
    ] {
        h.relay.lock().unwrap().reply_override = Some(reply);
        let err = h
            .run(&[
                "create",
                "--channel",
                CH_A,
                "--type",
                "buzz.task",
                "--title",
                "t",
            ])
            .await
            .unwrap_err();
        assert!(matches!(err, CliError::Other(_)), "{err}");
        assert_eq!(exit_code(&err), 4);
    }
}

#[tokio::test]
async fn a_lost_race_exits_5_and_never_overwrites_the_competing_edit() {
    let h = Harness::start().await;
    let id = h.create(CH_A, "buzz.task", "Mine").await;
    h.relay.lock().unwrap().races_left = 1;
    let err = h
        .run(&["update", "--channel", CH_A, "--id", &id, "--title", "Loser"])
        .await
        .unwrap_err();
    assert!(matches!(err, CliError::Conflict(_)), "{err}");
    assert_eq!(exit_code(&err), 5);
    assert!(
        err.to_string()
            .starts_with("conflict: artifact head changed;"),
        "{err}"
    );
    let head = h.relay.lock().unwrap().heads[&id].event.clone();
    assert_eq!(tag(&head, "title"), Some("Competing edit".into()));

    h.ok(&[
        "update",
        "--channel",
        CH_A,
        "--id",
        &id,
        "--title",
        "Reapplied",
    ])
    .await;
    let head = h.relay.lock().unwrap().heads[&id].event.clone();
    assert_eq!(tag(&head, "title"), Some("Reapplied".into()));

    h.relay.lock().unwrap().races_left = 1;
    for args in [
        vec!["delete", "--channel", CH_A, "--id", &id],
        vec![
            "move",
            "--channel",
            CH_A,
            "--id",
            &id,
            "--to",
            CH_B,
            "--yes",
        ],
    ] {
        let err = h.run(&args).await.unwrap_err();
        assert_eq!(exit_code(&err), 5, "{args:?}");
        h.relay.lock().unwrap().races_left = 1;
    }
}

#[tokio::test]
async fn create_ensure_is_safe_to_retry_and_reconciles_in_every_state() {
    let h = Harness::start().await;
    let ensure = |content: &'static str| {
        [
            "create",
            "--channel",
            CH_A,
            "--type",
            "buzz.config",
            "--title",
            "Theme",
            "--ensure",
            "theme",
            "--content",
            content,
        ]
    };
    let derived = derive_artifact_id(Uuid::parse_str(CH_A).unwrap(), "buzz.config", "theme")
        .unwrap()
        .to_string();

    let first = h.ok(&ensure("dark")).await;
    assert_eq!(first["artifact_id"], derived.as_str());
    assert_eq!(tag(&h.writes()[0], "op"), Some("create".into()));

    let again = h.ok(&ensure("dark")).await;
    assert_ne!(again["message"], "conflict");
    assert_eq!(again["event_id"], first["event_id"]);
    assert_eq!(h.writes().len(), 1, "an identical retry writes nothing");
    let settled = super::ensure(
        &h.client,
        Uuid::parse_str(CH_A).unwrap(),
        Uuid::parse_str(&derived).unwrap(),
        "Theme",
        "buzz.config",
        "dark",
    )
    .await
    .unwrap();
    assert!(settled.contains("unchanged"), "{settled}");
    assert_eq!(h.writes().len(), 1, "a matching head needs no revision");

    let changed = h.ok(&ensure("light")).await;
    assert_eq!(changed["accepted"], true);
    let update = h.writes()[1].clone();
    assert_eq!(tag(&update, "op"), Some("update".into()));
    assert_eq!(
        tag(&update, "prev"),
        first["event_id"].as_str().map(String::from)
    );
    assert_eq!(update["content"], "light");

    h.ok(&["delete", "--channel", CH_A, "--id", &derived]).await;
    h.ok(&ensure("light")).await;
    let restore = h.writes().last().unwrap().clone();
    assert_eq!(tag(&restore, "op"), Some("restore".into()));
    assert_eq!(restore["content"], "light");

    let before = h.queries().len();
    h.relay.lock().unwrap().races_left = 100;
    let err = h.run(&ensure("contested")).await.unwrap_err();
    assert_eq!(exit_code(&err), 5, "{err}");
    let head_reads = h.queries()[before..]
        .iter()
        .filter(|q| q["artifact"] == "head")
        .count();
    assert_eq!(
        head_reads, ENSURE_MAX_ATTEMPTS,
        "ensure stops after its bound"
    );
}

#[tokio::test]
async fn ensure_does_not_adopt_an_identity_owned_by_another_channel_or_type() {
    let h = Harness::start().await;
    let derived =
        derive_artifact_id(Uuid::parse_str(CH_A).unwrap(), "buzz.config", "theme").unwrap();
    let seeded = build_artifact_revision(&ArtifactRevision {
        artifact_id: derived,
        channel_id: Uuid::parse_str(CH_B).unwrap(),
        artifact_type: "buzz.config",
        op: ArtifactOp::Create,
        prev: None,
        root: None,
        title: Some("Elsewhere"),
        tags: &[],
        content: "",
    })
    .unwrap()
    .sign_with_keys(&Keys::generate())
    .unwrap();
    h.relay
        .lock()
        .unwrap()
        .apply(&serde_json::to_value(seeded).unwrap())
        .unwrap();
    let err = h
        .run(&[
            "create",
            "--channel",
            CH_A,
            "--type",
            "buzz.config",
            "--title",
            "Theme",
            "--ensure",
            "theme",
        ])
        .await
        .unwrap_err();
    assert_eq!(exit_code(&err), 5, "{err}");
    assert!(
        err.to_string().contains("different channel or type"),
        "{err}"
    );
    assert_eq!(h.writes().len(), 1);
}

#[tokio::test]
async fn move_writes_nothing_until_the_destination_audience_is_confirmed() {
    let h = Harness::start().await;
    let id = h.create(CH_A, "buzz.task", "Secret plan").await;
    let err = h
        .run(&["move", "--channel", CH_A, "--id", &id, "--to", CH_B])
        .await
        .unwrap_err();
    assert!(
        matches!(err, CliError::NotFound(_)),
        "unknown destination: {err}"
    );

    h.relay.lock().unwrap().channels.insert(
        CH_B.into(),
        json!({"kind": 39000, "tags": [["d", CH_B], ["name", "all-hands"], ["public"]]}),
    );
    let err = h
        .run(&["move", "--channel", CH_A, "--id", &id, "--to", CH_B])
        .await
        .unwrap_err();
    assert_eq!(exit_code(&err), 1);
    let message = err.to_string();
    assert!(
        message.contains("all-hands") && message.contains("Secret plan"),
        "{message}"
    );
    assert!(message.contains("--yes"), "{message}");
    assert!(message.contains("open"), "{message}");
    assert_eq!(h.writes().len(), 1, "no move was published");
    assert_eq!(h.relay.lock().unwrap().heads[&id].channel, CH_A);

    h.relay.lock().unwrap().channels.insert(
        CH_B.into(),
        json!({"kind": 39000, "tags": [["d", CH_B], ["name", "leads"], ["private"], ["restricted"], ["closed"]]}),
    );
    let err = h
        .run(&["move", "--channel", CH_A, "--id", &id, "--to", CH_B])
        .await
        .unwrap_err();
    assert!(err.to_string().contains("private"), "{err}");
    assert!(!err.to_string().contains("open"), "{err}");

    h.ok(&[
        "move",
        "--channel",
        CH_A,
        "--id",
        &id,
        "--to",
        CH_B,
        "--yes",
    ])
    .await;
    assert_eq!(h.relay.lock().unwrap().heads[&id].channel, CH_B);

    let same = h
        .run(&[
            "move",
            "--channel",
            CH_B,
            "--id",
            &id,
            "--to",
            CH_B,
            "--yes",
        ])
        .await
        .unwrap_err();
    assert_eq!(exit_code(&same), 1);
}

#[tokio::test]
async fn bad_input_is_refused_before_any_relay_request() {
    let h = Harness::start().await;
    let big = "x".repeat(MAX_CONTENT_BYTES + 1);
    let dir = std::env::temp_dir().join(format!("buzz-artifacts-{}", Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    let big_file = dir.join("big.txt");
    std::fs::write(&big_file, &big).unwrap();
    let ok_file = dir.join("ok.json");
    std::fs::write(&ok_file, r#"{"version":1}"#).unwrap();
    let missing = dir.join("missing.json");
    let (big_file, ok_file, missing) = (
        big_file.to_str().unwrap(),
        ok_file.to_str().unwrap(),
        missing.to_str().unwrap(),
    );
    let id = Uuid::new_v4().to_string();
    let upper = CH_A.to_uppercase();
    let id_upper = id.to_uppercase();
    let cases: Vec<Vec<&str>> = vec![
        vec![
            "create",
            "--channel",
            "nope",
            "--type",
            "buzz.task",
            "--title",
            "t",
        ],
        vec![
            "create",
            "--channel",
            &upper,
            "--type",
            "buzz.task",
            "--title",
            "t",
        ],
        vec![
            "create",
            "--channel",
            CH_A,
            "--type",
            "task",
            "--title",
            "t",
        ],
        vec![
            "create",
            "--channel",
            CH_A,
            "--type",
            "Buzz.Task",
            "--title",
            "t",
        ],
        vec![
            "create",
            "--channel",
            CH_A,
            "--type",
            "buzz.task",
            "--title",
            "  ",
        ],
        vec![
            "create",
            "--channel",
            CH_A,
            "--type",
            "buzz.task",
            "--title",
            "t",
            "--content",
            &big,
        ],
        vec![
            "create",
            "--channel",
            CH_A,
            "--type",
            "buzz.task",
            "--title",
            "t",
            "--content-file",
            big_file,
        ],
        vec![
            "create",
            "--channel",
            CH_A,
            "--type",
            "buzz.task",
            "--title",
            "t",
            "--content-file",
            missing,
        ],
        vec![
            "create",
            "--channel",
            CH_A,
            "--type",
            "buzz.task",
            "--title",
            "t",
            "--ensure",
            "",
        ],
        vec!["update", "--channel", CH_A, "--id", &id],
        vec!["update", "--channel", CH_A, "--id", "nope", "--title", "t"],
        vec!["get", "--channel", CH_A, "--id", &id_upper],
        vec!["list", "--channel", CH_A, "--type", "task"],
    ];
    for args in cases {
        let err = h.run(&args).await.unwrap_err();
        assert!(matches!(err, CliError::Usage(_)), "{args:?}: {err}");
        assert_eq!(exit_code(&err), 1);
    }
    assert!(h.queries().is_empty() && h.writes().is_empty());

    let created = h
        .ok(&[
            "create",
            "--channel",
            CH_A,
            "--type",
            "buzz.task",
            "--title",
            "From file",
            "--content-file",
            ok_file,
        ])
        .await;
    assert_eq!(created["accepted"], true);
    assert_eq!(h.writes()[0]["content"], r#"{"version":1}"#);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn content_reads_stop_one_byte_past_the_cap() {
    let at_cap = vec![b'a'; MAX_CONTENT_BYTES];
    assert_eq!(
        read_limited(&at_cap[..], "t").unwrap().len(),
        MAX_CONTENT_BYTES
    );
    let over = vec![b'a'; MAX_CONTENT_BYTES + 1];
    assert!(matches!(
        read_limited(&over[..], "t"),
        Err(CliError::Usage(_))
    ));
    let endless = std::io::repeat(b'a');
    assert!(matches!(
        read_limited(endless, "t"),
        Err(CliError::Usage(_))
    ));
    assert!(matches!(
        read_limited(&[0xff, 0xfe][..], "t"),
        Err(CliError::Usage(_))
    ));
}

#[test]
fn flags_are_bounded_and_exclusive_at_parse_time() {
    let parse = |args: &[&str]| {
        let mut argv = vec!["buzz", "artifacts"];
        argv.extend_from_slice(args);
        Cli::try_parse_from(argv).is_ok()
    };
    assert!(parse(&["list", "--channel", CH_A, "--limit", "1000"]));
    assert!(!parse(&["list", "--channel", CH_A, "--limit", "0"]));
    assert!(!parse(&["list", "--channel", CH_A, "--limit", "1001"]));
    assert!(!parse(&[
        "create",
        "--channel",
        CH_A,
        "--type",
        "a.b",
        "--title",
        "t",
        "--content",
        "x",
        "--content-file",
        "f"
    ]));
    assert!(!parse(&[
        "update",
        "--channel",
        CH_A,
        "--id",
        CH_B,
        "--content",
        "x",
        "--content-file",
        "f"
    ]));
    assert!(!parse(&["move", "--channel", CH_A, "--id", CH_B]));
    assert!(!parse(&["create", "--channel", CH_A, "--type", "a.b"]));
}
