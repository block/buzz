//! `buzz agents cancel-turn` and `buzz agents set-model` — owner-only control
//! frames (kind:24200, `frame=control`) answered by a `control_result`
//! telemetry frame from the agent harness.
//!
//! The relay stores neither direction. The command subscribes for the result
//! first, sends exactly one control frame, then waits a bounded time for the
//! result that carries its own `requestId`.

use std::time::Duration;

use buzz_core::kind::KIND_AGENT_OBSERVER_FRAME;
use buzz_core::observer::OBSERVER_FRAME_CONTROL;
use buzz_core::observer::{decrypt_observer_payload, encrypt_observer_payload};
use nostr::{Event, Keys, PublicKey};
use serde_json::{json, Value};

use super::agent_activity::{is_agent_telemetry, unwrap_batch};
use super::agents::require_agent_owner;
use super::messages::resolve_author;
use crate::client::BuzzClient;
use crate::error::CliError;
use crate::validate::{parse_uuid, sdk_err};

/// Seconds waited for the harness result when `--wait` is not given.
pub const DEFAULT_WAIT_SECS: u64 = 8;
/// Largest accepted `--wait`.
pub const MAX_WAIT_SECS: u64 = 30;
const SINCE_SLACK_SECS: u64 = 300;
const SUBSCRIPTION_LIMIT: usize = 100;

/// The control action to send.
#[derive(Debug, Clone, PartialEq)]
pub enum Control {
    /// Stop the in-flight turn in a channel.
    CancelTurn,
    /// Switch the agent's model for a channel.
    SwitchModel {
        /// Model id as the agent's runtime names it.
        model_id: String,
    },
}

impl Control {
    fn type_name(&self) -> &'static str {
        match self {
            Control::CancelTurn => "cancel_turn",
            Control::SwitchModel { .. } => "switch_model",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Verdict {
    Success,
    Refused,
}

/// Terminal verdict for a harness `status`, or `None` for a provisional or
/// unknown status. Unknown statuses never count as success.
fn verdict(control: &Control, status: &str) -> Option<Verdict> {
    match (control, status) {
        (Control::CancelTurn, "sent") | (Control::SwitchModel { .. }, "switched") => {
            Some(Verdict::Success)
        }
        (Control::CancelTurn, "no_active_turn" | "ambiguous_target") => Some(Verdict::Refused),
        (
            Control::SwitchModel { .. },
            "failure" | "unsupported_model" | "turn_ending" | "no_active_turn" | "ambiguous_target",
        ) => Some(Verdict::Refused),
        _ => None,
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Finish {
    Success,
    Refused,
    Pending,
    Unconfirmed,
}

/// What a control run learned. Printed as one JSON object.
#[derive(Debug)]
pub(crate) struct Outcome {
    request_id: String,
    agent: String,
    channel_id: String,
    model_id: Option<String>,
    status: String,
    confirmed: bool,
    finish: Finish,
}

impl Outcome {
    fn to_json(&self) -> Value {
        let mut out = json!({
            "request_id": self.request_id,
            "agent": self.agent,
            "channel_id": self.channel_id,
            "status": self.status,
            "confirmed": self.confirmed,
        });
        if let (Some(model), Some(map)) = (&self.model_id, out.as_object_mut()) {
            map.insert("model_id".into(), model.clone().into());
        }
        out
    }

    /// Exit-code mapping: success and `pending` are `Ok`; a refusal is exit 1;
    /// no answer is `DeliveryUnknown` (exit 2, never retryable).
    fn into_result(self) -> Result<(), CliError> {
        match self.finish {
            Finish::Success | Finish::Pending => Ok(()),
            Finish::Refused if self.status == "unsupported_model" => Err(CliError::Usage(
                "the agent does not offer that model".into(),
            )),
            Finish::Refused => Err(CliError::NotFound(format!(
                "the agent answered {}",
                self.status
            ))),
            Finish::Unconfirmed => Err(unknown_outcome(
                &self.agent,
                "the agent did not answer in time",
            )),
        }
    }
}

fn unknown_outcome(agent: &str, cause: &str) -> CliError {
    CliError::DeliveryUnknown(format!(
        "the control frame was sent but {cause}, so the outcome is unknown; do not blindly \
         retry. Run `buzz agents activity --agent {agent} --wait 10` to see the agent's state"
    ))
}

pub(crate) async fn run(
    client: &BuzzClient,
    agent: &str,
    channel: &str,
    control: Control,
    wait: u64,
) -> Result<(), CliError> {
    let outcome = execute(client, agent, channel, control, wait, unix_now()).await?;
    println!("{}", outcome.to_json());
    outcome.into_result()
}

pub(crate) async fn execute(
    client: &BuzzClient,
    agent: &str,
    channel: &str,
    control: Control,
    wait: u64,
    now: u64,
) -> Result<Outcome, CliError> {
    if !(1..=MAX_WAIT_SECS).contains(&wait) {
        return Err(CliError::Usage(format!("--wait must be 1-{MAX_WAIT_SECS}")));
    }
    let channel_id = parse_uuid(channel)?;
    let channel = channel_id.to_string();
    let request_id = uuid::Uuid::new_v4().to_string();
    let payload = match &control {
        Control::CancelTurn => buzz_sdk::build_cancel_turn_control_payload(channel_id, &request_id),
        Control::SwitchModel { model_id } => {
            buzz_sdk::build_switch_model_control_payload(channel_id, model_id, &request_id)
        }
    }
    .map_err(sdk_err)?;

    let agent_hex = resolve_author(client, agent).await?;
    require_agent_owner(client, &agent_hex).await?;
    let agent_key = PublicKey::from_hex(&agent_hex)
        .map_err(|e| CliError::Usage(format!("invalid agent pubkey: {e}")))?;
    let encrypted = encrypt_observer_payload(client.keys(), &agent_key, &payload)
        .map_err(|e| CliError::Other(format!("could not encrypt the control frame: {e}")))?;
    let event = client.sign_event_unchecked(
        buzz_sdk::build_agent_observer_frame(
            &agent_hex,
            &agent_hex,
            OBSERVER_FRAME_CONTROL,
            &encrypted,
        )
        .map_err(sdk_err)?,
    )?;

    let since = now.saturating_sub(SINCE_SLACK_SECS);
    let mut session = client
        .open_subscription(&json!({
            "kinds": [KIND_AGENT_OBSERVER_FRAME],
            "authors": [agent_hex],
            "#p": [client.keys().public_key().to_hex()],
            "since": since,
            "limit": SUBSCRIPTION_LIMIT,
        }))
        .await?;
    let request = Request {
        keys: client.keys(),
        agent: &agent_key,
        control: &control,
        channel: &channel,
        request_id: &request_id,
        since,
    };
    let waited = match session.publish(event).await {
        Ok(()) => await_result(&mut session, &request, Duration::from_secs(wait)).await,
        Err(e) => Err(e),
    };
    session.close().await;

    let (status, finish) = waited?;
    Ok(Outcome {
        request_id,
        agent: agent_hex,
        channel_id: channel,
        model_id: match control {
            Control::SwitchModel { model_id } => Some(model_id),
            Control::CancelTurn => None,
        },
        confirmed: matches!(finish, Finish::Success | Finish::Refused),
        status,
        finish,
    })
}

struct Request<'a> {
    keys: &'a Keys,
    agent: &'a PublicKey,
    control: &'a Control,
    channel: &'a str,
    request_id: &'a str,
    since: u64,
}

async fn await_result(
    session: &mut crate::client::EventSession,
    request: &Request<'_>,
    wait: Duration,
) -> Result<(String, Finish), CliError> {
    let deadline = tokio::time::Instant::now() + wait;
    let mut provisional = false;
    loop {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        let next = session.next_event(remaining).await.map_err(|e| match e {
            CliError::Auth(_) => e,
            other => unknown_outcome(
                &request.agent.to_hex(),
                &format!("the connection failed while waiting ({other})"),
            ),
        })?;
        let Some(event) = next else {
            break;
        };
        for status in result_statuses(request, &event) {
            match verdict(request.control, &status) {
                Some(Verdict::Success) => return Ok((status, Finish::Success)),
                Some(Verdict::Refused) => return Ok((status, Finish::Refused)),
                None if status == "sent" => provisional = true,
                None => {}
            }
        }
    }
    Ok(if provisional {
        ("pending".into(), Finish::Pending)
    } else {
        ("unconfirmed".into(), Finish::Unconfirmed)
    })
}

/// Statuses of `control_result` frames in `event` that answer this exact
/// request: signed by the agent, tagged as telemetry, no older than `since`,
/// decryptable by the owner key, and matching type, channel, and `requestId`.
fn result_statuses(request: &Request<'_>, event: &Event) -> Vec<String> {
    if !is_agent_telemetry(event, &request.agent.to_hex())
        || event.created_at.as_secs() < request.since
        || event.verify().is_err()
    {
        return Vec::new();
    }
    let Ok(frame) = decrypt_observer_payload::<Value>(request.keys, event) else {
        return Vec::new();
    };
    unwrap_batch(&frame)
        .into_iter()
        .filter(|inner| inner.get("kind").and_then(Value::as_str) == Some("control_result"))
        .filter(|inner| inner.get("channelId").and_then(Value::as_str) == Some(request.channel))
        .filter_map(|inner| inner.get("payload"))
        .filter(|p| p.get("type").and_then(Value::as_str) == Some(request.control.type_name()))
        .filter(|p| p.get("requestId").and_then(Value::as_str) == Some(request.request_id))
        .filter_map(|p| p.get("status").and_then(Value::as_str).map(str::to_owned))
        .collect()
}

fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
    use axum::{extract::State, response::IntoResponse, routing::get, routing::post, Json, Router};
    use buzz_core::observer::{OBSERVER_FRAME_TAG, OBSERVER_FRAME_TELEMETRY};
    use nostr::{EventBuilder, Kind, Timestamp};
    use std::sync::{Arc, Mutex};
    use tokio::net::TcpListener;

    const NOW: u64 = 1_800_000_000;
    const CHANNEL: &str = "7c07e659-3610-42f4-9a5e-1e9973c09da9";

    /// A `control_result` the fake relay forwards after it accepts the control
    /// frame. `None` fields echo the real request.
    #[derive(Clone)]
    struct Reply {
        status: &'static str,
        request_id: Option<String>,
        channel: Option<String>,
        control_type: Option<&'static str>,
        signer: Option<Keys>,
        created_at: u64,
        batch: bool,
    }

    fn reply(status: &'static str) -> Reply {
        Reply {
            status,
            request_id: None,
            channel: None,
            control_type: None,
            signer: None,
            created_at: NOW,
            batch: false,
        }
    }

    #[derive(Default, Clone)]
    enum AfterOk {
        #[default]
        Replies,
        Disconnect,
        Close(&'static str),
    }

    #[derive(Default)]
    struct Relay {
        replies: Vec<Reply>,
        log: Vec<&'static str>,
        reqs: Vec<Value>,
        published: Vec<Event>,
        profiles: Vec<Value>,
        refuse_req: Option<String>,
        withhold_eose: bool,
        after_ok: AfterOk,
        refuse_event: Option<String>,
        agent: Option<Keys>,
        owner: Option<Keys>,
    }
    type Shared = Arc<Mutex<Relay>>;

    fn result_event(
        relay: &Relay,
        reply: &Reply,
        request_id: &str,
        channel: &str,
        ctl: &str,
    ) -> Event {
        let (agent, owner) = (relay.agent.clone().unwrap(), relay.owner.clone().unwrap());
        let signer = reply.signer.clone().unwrap_or_else(|| agent.clone());
        let inner = json!({
            "seq": 1,
            "timestamp": "2026-10-30T00:00:00Z",
            "kind": "control_result",
            "agentIndex": 0,
            "channelId": reply.channel.clone().unwrap_or_else(|| channel.to_owned()),
            "sessionId": null,
            "turnId": null,
            "payload": {
                "type": reply.control_type.unwrap_or(match ctl {
                    "cancel_turn" => "cancel_turn",
                    _ => "switch_model",
                }),
                "status": reply.status,
                "requestId": reply.request_id.clone().unwrap_or_else(|| request_id.to_owned()),
            },
        });
        let frame = if reply.batch {
            json!({"seq": 9, "kind": "batch", "payload": {"events": [inner]}})
        } else {
            inner
        };
        let encrypted = encrypt_observer_payload(&signer, &owner.public_key(), &frame).unwrap();
        buzz_sdk::build_agent_observer_frame(
            &owner.public_key().to_hex(),
            &agent.public_key().to_hex(),
            OBSERVER_FRAME_TELEMETRY,
            &encrypted,
        )
        .unwrap()
        .custom_created_at(Timestamp::from(reply.created_at))
        .sign_with_keys(&signer)
        .unwrap()
    }

    async fn socket(State(relay): State<Shared>, ws: WebSocketUpgrade) -> impl IntoResponse {
        ws.on_upgrade(move |mut ws: WebSocket| async move {
            let send = |value: Value| Message::Text(value.to_string().into());
            let _ = ws.send(send(json!(["AUTH", "challenge"]))).await;
            let mut sub = Value::Null;
            while let Some(Ok(Message::Text(text))) = ws.recv().await {
                let msg: Vec<Value> = serde_json::from_str(&text).unwrap();
                match msg[0].as_str() {
                    Some("AUTH") => {
                        let _ = ws.send(send(json!(["OK", msg[1]["id"], true, ""]))).await;
                    }
                    Some("REQ") => {
                        sub = msg[1].clone();
                        let refusal = {
                            let mut relay = relay.lock().unwrap();
                            relay.log.push("REQ");
                            relay.reqs.push(msg[2].clone());
                            relay.refuse_req.clone()
                        };
                        let answer = match refusal {
                            Some(reason) => json!(["CLOSED", sub, reason]),
                            None => json!(["EOSE", sub]),
                        };
                        if !relay.lock().unwrap().withhold_eose {
                            let _ = ws.send(send(answer)).await;
                        }
                    }
                    Some("EVENT") => {
                        let event: Event = serde_json::from_value(msg[1].clone()).unwrap();
                        let (refusal, frames) = {
                            let mut relay = relay.lock().unwrap();
                            relay.log.push("EVENT");
                            relay.published.push(event.clone());
                            let refusal = relay.refuse_event.clone();
                            let payload: Value =
                                decrypt_observer_payload(relay.agent.as_ref().unwrap(), &event)
                                    .unwrap();
                            let frames: Vec<Event> = relay
                                .replies
                                .iter()
                                .map(|r| {
                                    result_event(
                                        &relay,
                                        r,
                                        payload["requestId"].as_str().unwrap(),
                                        payload["channelId"].as_str().unwrap(),
                                        payload["type"].as_str().unwrap(),
                                    )
                                })
                                .collect();
                            (refusal, frames)
                        };
                        let ok = match &refusal {
                            Some(reason) => json!(["OK", event.id.to_hex(), false, reason]),
                            None => json!(["OK", event.id.to_hex(), true, ""]),
                        };
                        let _ = ws.send(send(ok)).await;
                        let after_ok = relay.lock().unwrap().after_ok.clone();
                        match after_ok {
                            AfterOk::Disconnect => return,
                            AfterOk::Close(reason) => {
                                let _ = ws.send(send(json!(["CLOSED", sub, reason]))).await;
                                continue;
                            }
                            AfterOk::Replies => {}
                        }
                        if refusal.is_none() {
                            for frame in frames {
                                let _ = ws.send(send(json!(["EVENT", sub, frame]))).await;
                            }
                        }
                    }
                    _ => {}
                }
            }
        })
    }

    async fn query(State(relay): State<Shared>, Json(filters): Json<Vec<Value>>) -> Json<Value> {
        let relay = relay.lock().unwrap();
        let author = &filters[0]["authors"][0];
        let found: Vec<_> = relay
            .profiles
            .iter()
            .filter(|p| &p["pubkey"] == author)
            .cloned()
            .collect();
        Json(json!(found))
    }

    async fn setup(replies: Vec<Reply>) -> (BuzzClient, Shared, Keys, Keys) {
        let (owner, agent) = (Keys::generate(), Keys::generate());
        let sig = "a".repeat(128);
        let profile = EventBuilder::new(Kind::Metadata, "{\"name\":\"Scout\"}")
            .tags([nostr::Tag::parse(["auth", &owner.public_key().to_hex(), "", &sig]).unwrap()])
            .sign_with_keys(&agent)
            .unwrap();
        let relay = Shared::default();
        {
            let mut r = relay.lock().unwrap();
            r.replies = replies;
            r.profiles.push(serde_json::to_value(profile).unwrap());
            r.agent = Some(agent.clone());
            r.owner = Some(owner.clone());
        }
        let client = BuzzClient::new(serve(&relay).await, owner.clone(), None, None).unwrap();
        (client, relay, owner, agent)
    }

    async fn go(client: &BuzzClient, agent: &Keys, control: Control) -> Result<Outcome, CliError> {
        execute(
            client,
            &agent.public_key().to_hex(),
            CHANNEL,
            control,
            1,
            NOW,
        )
        .await
    }

    fn switch(model: &str) -> Control {
        Control::SwitchModel {
            model_id: model.into(),
        }
    }

    fn exit_of(outcome: Outcome) -> i32 {
        outcome
            .into_result()
            .err()
            .map_or(0, |e| crate::error::exit_code(&e))
    }

    #[tokio::test]
    async fn cancel_subscribes_first_sends_one_owner_signed_frame_and_reports_the_result() {
        let (client, relay, owner, agent) = setup(vec![reply("sent")]).await;
        let outcome = go(&client, &agent, Control::CancelTurn).await.unwrap();

        let relay = relay.lock().unwrap();
        assert_eq!(relay.log, ["REQ", "EVENT"]);
        let req = &relay.reqs[0];
        assert_eq!(req["kinds"], json!([24_200]));
        assert_eq!(req["#p"], json!([owner.public_key().to_hex()]));
        assert_eq!(req["authors"], json!([agent.public_key().to_hex()]));
        assert_eq!(req["since"], NOW - SINCE_SLACK_SECS);
        assert_eq!(req["limit"], SUBSCRIPTION_LIMIT);

        assert_eq!(relay.published.len(), 1);
        let event = &relay.published[0];
        assert_eq!(event.pubkey, owner.public_key());
        let tag = |name: &str| {
            event
                .tags
                .iter()
                .find(|t| t.kind().to_string() == name)
                .and_then(|t| t.content())
                .map(str::to_owned)
        };
        assert_eq!(tag("p"), Some(agent.public_key().to_hex()));
        assert_eq!(tag("agent"), Some(agent.public_key().to_hex()));
        assert_eq!(
            tag(OBSERVER_FRAME_TAG).as_deref(),
            Some(OBSERVER_FRAME_CONTROL)
        );
        let payload: Value = decrypt_observer_payload(&agent, event).unwrap();
        assert_eq!(
            payload,
            json!({"type": "cancel_turn", "channelId": CHANNEL, "requestId": outcome.request_id})
        );

        let printed = outcome.to_json();
        assert_eq!(printed["status"], "sent");
        assert_eq!(printed["confirmed"], true);
        assert_eq!(printed["channel_id"], CHANNEL);
        assert_eq!(printed["agent"], agent.public_key().to_hex());
        assert!(printed.get("model_id").is_none());
        assert_eq!(exit_of(outcome), 0);
    }

    #[tokio::test]
    async fn only_the_result_for_this_request_settles_it() {
        let other = Keys::generate();
        let decoys = vec![
            Reply {
                request_id: Some("older".into()),
                ..reply("sent")
            },
            Reply {
                channel: Some("11111111-1111-4111-8111-111111111111".into()),
                ..reply("sent")
            },
            Reply {
                control_type: Some("switch_model"),
                ..reply("sent")
            },
            Reply {
                signer: Some(other),
                ..reply("sent")
            },
            Reply {
                created_at: NOW - SINCE_SLACK_SECS - 1,
                ..reply("sent")
            },
            reply("something_new"),
            Reply {
                batch: true,
                ..reply("no_active_turn")
            },
        ];
        let (client, _, _, agent) = setup(decoys).await;
        let outcome = go(&client, &agent, Control::CancelTurn).await.unwrap();
        assert_eq!(outcome.status, "no_active_turn");
        assert!(outcome.confirmed);
        assert_eq!(exit_of(outcome), 1);
    }

    #[tokio::test]
    async fn every_terminal_status_maps_to_one_output_and_exit_code() {
        let cases: Vec<(Control, &str, bool, i32)> = vec![
            (Control::CancelTurn, "sent", true, 0),
            (Control::CancelTurn, "no_active_turn", true, 1),
            (Control::CancelTurn, "ambiguous_target", true, 1),
            (switch("m"), "switched", true, 0),
            (switch("m"), "failure", true, 1),
            (switch("m"), "unsupported_model", true, 1),
            (switch("m"), "turn_ending", true, 1),
            (switch("m"), "no_active_turn", true, 1),
            (switch("m"), "ambiguous_target", true, 1),
        ];
        for (control, status, confirmed, exit) in cases {
            let (client, _, _, agent) = setup(vec![reply(status)]).await;
            let outcome = go(&client, &agent, control).await.unwrap();
            assert_eq!(outcome.status, status);
            assert_eq!(outcome.confirmed, confirmed, "{status}");
            assert_eq!(exit_of(outcome), exit, "{status}");
        }
    }

    #[tokio::test]
    async fn switch_waits_past_the_provisional_ack_for_the_late_result() {
        let (client, relay, _, agent) = setup(vec![reply("sent"), reply("switched")]).await;
        let outcome = go(&client, &agent, switch("model-b")).await.unwrap();
        assert_eq!(outcome.status, "switched");
        assert_eq!(outcome.to_json()["model_id"], "model-b");
        let payload: Value =
            decrypt_observer_payload(&agent, &relay.lock().unwrap().published[0]).unwrap();
        assert_eq!(payload["modelId"], "model-b");
        assert_eq!(payload["type"], "switch_model");

        let (client, _, _, agent) = setup(vec![reply("sent"), reply("unsupported_model")]).await;
        let outcome = go(&client, &agent, switch("model-b")).await.unwrap();
        assert_eq!(outcome.status, "unsupported_model");
        let error = outcome.into_result().unwrap_err();
        assert!(matches!(error, CliError::Usage(_)), "{error:?}");
    }

    #[tokio::test]
    async fn an_ack_without_a_final_result_is_pending_never_success() {
        let (client, _, _, agent) = setup(vec![reply("sent")]).await;
        let outcome = go(&client, &agent, switch("model-b")).await.unwrap();
        assert_eq!(outcome.status, "pending");
        assert!(!outcome.confirmed);
        assert_eq!(outcome.to_json()["confirmed"], false);
        assert_eq!(exit_of(outcome), 0);
    }

    #[tokio::test]
    async fn no_answer_is_unconfirmed_and_never_retryable() {
        for (control, replies) in [
            (Control::CancelTurn, vec![]),
            (Control::CancelTurn, vec![reply("something_new")]),
            (switch("m"), vec![]),
        ] {
            let (client, relay, _, agent) = setup(replies).await;
            let outcome = go(&client, &agent, control).await.unwrap();
            assert_eq!(outcome.status, "unconfirmed");
            assert!(!outcome.confirmed);
            assert_eq!(relay.lock().unwrap().published.len(), 1);
            let error = outcome.into_result().unwrap_err();
            assert!(matches!(error, CliError::DeliveryUnknown(_)), "{error:?}");
            assert_eq!(crate::error::exit_code(&error), 2);
            assert!(!crate::error::is_retryable_error(&error));
            assert!(error.to_string().contains("do not blindly retry"));
        }
    }

    #[tokio::test]
    async fn a_non_owner_gets_exit_3_before_anything_is_sent() {
        let (_, relay, _, agent) = setup(vec![reply("sent")]).await;
        let other_owner = Keys::generate();
        let client = BuzzClient::new(serve(&relay).await, other_owner, None, None).unwrap();
        let error = go(&client, &agent, Control::CancelTurn).await.unwrap_err();
        assert!(matches!(error, CliError::Auth(_)), "{error:?}");
        assert_eq!(crate::error::exit_code(&error), 3);
        let relay = relay.lock().unwrap();
        assert!(relay.log.is_empty() && relay.published.is_empty());
    }

    async fn serve(relay: &Shared) -> String {
        let app = Router::new()
            .route("/", get(socket))
            .route("/query", post(query))
            .with_state(relay.clone());
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        format!("http://{addr}")
    }

    #[tokio::test]
    async fn relay_refusals_are_errors_and_a_refused_subscription_sends_nothing() {
        let (client, relay, _, agent) = setup(vec![reply("sent")]).await;
        relay.lock().unwrap().refuse_event = Some("restricted: not authorized".into());
        let error = go(&client, &agent, Control::CancelTurn).await.unwrap_err();
        assert_eq!(crate::error::exit_code(&error), 3, "{error:?}");

        relay.lock().unwrap().refuse_event = Some("rate-limited".into());
        let error = go(&client, &agent, Control::CancelTurn).await.unwrap_err();
        assert_eq!(crate::error::exit_code(&error), 2, "{error:?}");

        let (client, relay, _, agent) = setup(vec![reply("sent")]).await;
        relay.lock().unwrap().refuse_req = Some("restricted: no".into());
        let error = go(&client, &agent, Control::CancelTurn).await.unwrap_err();
        assert!(matches!(error, CliError::Auth(_)), "{error:?}");
        assert!(relay.lock().unwrap().published.is_empty());
    }

    #[tokio::test]
    async fn a_failure_after_the_frame_was_sent_is_never_retryable() {
        for (after_ok, exit) in [
            (AfterOk::Disconnect, 2),
            (AfterOk::Close("error: shutting down"), 2),
            (AfterOk::Close("restricted: revoked"), 3),
        ] {
            let (client, relay, _, agent) = setup(vec![]).await;
            relay.lock().unwrap().after_ok = after_ok;
            let error = go(&client, &agent, Control::CancelTurn).await.unwrap_err();
            assert_eq!(relay.lock().unwrap().published.len(), 1);
            assert_eq!(crate::error::exit_code(&error), exit, "{error:?}");
            assert!(!crate::error::is_retryable_error(&error), "{error:?}");
            if exit == 2 {
                assert!(matches!(error, CliError::DeliveryUnknown(_)), "{error:?}");
                assert!(error.to_string().contains("buzz agents activity"));
            }
        }
    }

    #[tokio::test]
    async fn nothing_is_sent_until_the_relay_confirms_the_subscription() {
        let (client, relay, _, agent) = setup(vec![reply("sent")]).await;
        relay.lock().unwrap().withhold_eose = true;
        let error = go(&client, &agent, Control::CancelTurn).await.unwrap_err();
        assert!(matches!(error, CliError::Relay { .. }), "{error:?}");
        assert_eq!(crate::error::exit_code(&error), 2);
        assert!(relay.lock().unwrap().published.is_empty());
    }

    #[tokio::test]
    async fn bad_input_is_rejected_before_any_network_use() {
        let (client, relay, _, agent) = setup(vec![]).await;
        let hex = agent.public_key().to_hex();
        let too_long = "m".repeat(201);
        let attempts = [
            (CHANNEL, Control::CancelTurn, 0),
            (CHANNEL, Control::CancelTurn, MAX_WAIT_SECS + 1),
            ("not-a-uuid", Control::CancelTurn, 1),
            (CHANNEL, switch(""), 1),
            (CHANNEL, switch(&too_long), 1),
        ];
        for (channel, control, wait) in attempts {
            let error = execute(&client, &hex, channel, control, wait, NOW)
                .await
                .unwrap_err();
            assert!(matches!(error, CliError::Usage(_)), "{error:?}");
        }
        assert!(relay.lock().unwrap().log.is_empty());
    }

    #[test]
    fn unknown_statuses_are_never_success() {
        for control in [Control::CancelTurn, switch("m")] {
            for status in ["", "ok", "success", "switchedd", "SENT"] {
                assert_eq!(verdict(&control, status), None, "{status}");
            }
        }
        assert_eq!(verdict(&switch("m"), "sent"), None);
    }
}
