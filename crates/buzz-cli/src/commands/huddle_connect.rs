//! `buzz huddles connect` — join (or start) a channel's huddle and carry its
//! audio over standard input and output, so another program can be the
//! microphone and the speaker while this CLI keeps the identity.
//!
//! Getting in follows desktop: starting a huddle makes its private ephemeral
//! backing stream, then posts kind:48100 on the parent. The audio WebSocket
//! handshake (NIP-42 challenge, protocol v2, roster) is the shared one in
//! `buzz-audio-client`; this command only signs the auth event with its own
//! credentials and sends its usual upgrade headers. Media is passed through
//! untouched: the other program encodes and decodes Opus.
//!
//! Every line on stdout and stdin is one JSON object:
//!
//! ```text
//! out: {"type":"joined","huddle":<uuid>,"started":bool,"peer_index":n,"peers":[{"peer_index","pubkey","epoch"}]}
//! out: {"type":"audio","data":<base64 of peer_index ‖ v2 header ‖ opus>}
//! out: {"type":"control","data":<relay control message>}
//! out: {"type":"closed","reason":<text>,"ended":bool}
//! in:  {"type":"audio","data":<base64 of v2 header ‖ opus>}
//! in:  {"type":"control","data":<control message for the relay>}
//! ```
//!
//! Closing stdin leaves the huddle. A huddle this command started is ended and
//! its backing stream archived when it leaves as the last one present, as
//! desktop does; if the roster cannot be read for certain, it is left running.

use base64::Engine;
use futures_util::{SinkExt, StreamExt};
use nostr::{EventBuilder, JsonUtil, Kind, Tag};
use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio_tungstenite::tungstenite::{client::IntoClientRequest, Message};
use uuid::Uuid;

use buzz_core::kind::{KIND_AUTH, KIND_HUDDLE_ENDED, KIND_HUDDLE_STARTED};

use crate::client::BuzzClient;
use crate::error::CliError;
use crate::validate::parse_uuid;

/// The relay's default ephemeral lifetime for a huddle's backing stream.
const BACKING_TTL_SECONDS: i32 = 3600;
/// Largest media or control message accepted either way, as the relay allows.
const MAX_MESSAGE: usize = 65_536;

/// The relay's WebSocket base, as the NIP-42 `relay` tag names it.
fn ws_base(relay: &str) -> String {
    let base = relay.trim_end_matches('/');
    if let Some(rest) = base.strip_prefix("https://") {
        format!("wss://{rest}")
    } else if let Some(rest) = base.strip_prefix("http://") {
        format!("ws://{rest}")
    } else {
        base.to_string()
    }
}

fn ws_url(relay: &str, huddle: &str) -> String {
    format!("{}/huddle/{huddle}/audio", ws_base(relay))
}

/// A parent-channel huddle advisory (48100 started, 48103 ended), as desktop
/// writes it.
fn huddle_event(kind: u32, parent: &Uuid, huddle: &Uuid) -> Result<EventBuilder, CliError> {
    let kind = u16::try_from(kind).map_err(|_| CliError::Other("bad huddle kind".into()))?;
    let content = json!({ "ephemeral_channel_id": huddle.to_string() }).to_string();
    let h = Tag::parse(["h", &parent.to_string()])
        .map_err(|e| CliError::Other(format!("tag error: {e}")))?;
    Ok(EventBuilder::new(Kind::Custom(kind), content).tags([h]))
}

async fn publish(client: &BuzzClient, builder: EventBuilder, what: &str) -> Result<(), CliError> {
    let event = client.sign_event(builder).await?;
    super::parse_write_response(&client.submit_event(event).await?, what)?;
    Ok(())
}

/// Best-effort archive of a huddle's backing stream.
async fn archive(client: &BuzzClient, huddle: &Uuid) {
    if let Ok(builder) = buzz_sdk::build_archive(*huddle) {
        let _ = publish(client, builder, "the huddle channel could not be archived").await;
    }
}

/// Starts a huddle in `parent`: its private ephemeral backing stream first,
/// then the started event that points the parent's members to it. A backing
/// stream left without its started event is archived.
async fn start(client: &BuzzClient, parent: &Uuid) -> Result<Uuid, CliError> {
    let huddle = Uuid::new_v4();
    let name = format!("huddle-{}", &huddle.to_string()[..8]);
    let create = buzz_sdk::build_create_channel(
        huddle,
        &name,
        Some(buzz_sdk::Visibility::Private),
        Some(buzz_sdk::ChannelKind::Stream),
        None,
        Some(BACKING_TTL_SECONDS),
        false,
    )
    .map_err(|e| CliError::Other(format!("build_create_channel failed: {e}")))?;
    publish(client, create, "the huddle channel could not be made").await?;
    let started = async {
        let builder = huddle_event(KIND_HUDDLE_STARTED, parent, &huddle)?;
        publish(client, builder, "the huddle could not be started").await
    }
    .await;
    if let Err(e) = started {
        archive(client, &huddle).await;
        return Err(e);
    }
    Ok(huddle)
}

/// Whether nobody but `me` is still in the huddle, by its lifecycle events.
/// Anything short of a complete roster counts as "others may remain".
fn only_me_left(participants: &Value, me: &str) -> bool {
    participants["state"] == "active"
        && participants["complete"] == true
        && participants["participants"]
            .as_array()
            .is_some_and(|all| all.iter().all(|p| p["pubkey"] == me))
}

/// Ends a huddle this command started when it is the last one present.
async fn end_if_last(client: &BuzzClient, parent: &Uuid, huddle: &Uuid) -> bool {
    let Ok(now) = super::huddles::now_seconds() else {
        return false;
    };
    let roster = super::huddles::run_participants(
        client,
        &parent.to_string(),
        &huddle.to_string(),
        super::huddles::MAX_LIFECYCLE_EVENTS,
        now,
    )
    .await;
    let me = client.public_key().to_hex();
    if !roster.is_ok_and(|r| only_me_left(&r, &me)) {
        return false;
    }
    let Ok(ended) = huddle_event(KIND_HUDDLE_ENDED, parent, huddle) else {
        return false;
    };
    if publish(client, ended, "the huddle could not be ended")
        .await
        .is_err()
    {
        return false;
    }
    archive(client, huddle).await;
    true
}

async fn emit(out: &mut tokio::io::Stdout, value: Value) -> Result<(), CliError> {
    let mut line = value.to_string();
    line.push('\n');
    out.write_all(line.as_bytes())
        .await
        .map_err(|e| CliError::Other(format!("stdout closed: {e}")))?;
    out.flush()
        .await
        .map_err(|e| CliError::Other(format!("stdout closed: {e}")))
}

pub async fn cmd_connect(
    client: &BuzzClient,
    channel: &str,
    huddle: Option<&str>,
) -> Result<(), CliError> {
    let parent = parse_uuid(channel)?;
    let (huddle, started) = match huddle {
        Some(id) => (parse_uuid(id)?, false),
        None => (start(client, &parent).await?, true),
    };

    let joined = join(client, &parent, &huddle).await;
    let joined = match joined {
        Ok(joined) => joined,
        Err(e) => {
            if started {
                let _ = end_if_last(client, &parent, &huddle).await;
            }
            return Err(e);
        }
    };

    let mut out = tokio::io::stdout();
    let peers: Vec<Value> = joined
        .peers
        .iter()
        .map(|p| json!({"peer_index": p.index, "pubkey": p.pubkey, "epoch": p.epoch}))
        .collect();
    emit(
        &mut out,
        json!({
            "type": "joined", "huddle": huddle.to_string(), "started": started,
            "peer_index": joined.index, "peers": peers,
        }),
    )
    .await?;

    let b64 = base64::engine::general_purpose::STANDARD;
    let (mut tx, mut rx) = joined.socket.split();
    let mut input = BufReader::new(tokio::io::stdin()).lines();
    let reason = loop {
        tokio::select! {
            line = input.next_line() => {
                let Ok(Some(line)) = line else { break "left".to_string() };
                if line.len() > MAX_MESSAGE * 2 { continue; }
                let Ok(msg) = serde_json::from_str::<Value>(&line) else { continue };
                let sent = match msg["type"].as_str() {
                    Some("audio") => match msg["data"].as_str().and_then(|d| b64.decode(d).ok()) {
                        Some(bytes) if bytes.len() <= MAX_MESSAGE => tx.send(Message::Binary(bytes.into())).await,
                        _ => continue,
                    },
                    Some("control") => tx.send(Message::Text(msg["data"].to_string().into())).await,
                    _ => continue,
                };
                if let Err(e) = sent { break format!("connection lost: {e}"); }
            }
            frame = rx.next() => {
                match frame {
                    None => break "closed by the relay".to_string(),
                    Some(Err(e)) => break format!("connection lost: {e}"),
                    Some(Ok(Message::Binary(bytes))) if bytes.len() <= MAX_MESSAGE => {
                        emit(&mut out, json!({"type": "audio", "data": b64.encode(&bytes)})).await?;
                    }
                    Some(Ok(Message::Text(text))) => {
                        if let Ok(data) = serde_json::from_str::<Value>(&text) {
                            emit(&mut out, json!({"type": "control", "data": data})).await?;
                        }
                    }
                    Some(Ok(Message::Ping(data))) => {
                        if let Err(e) = tx.send(Message::Pong(data)).await {
                            break format!("connection lost: {e}");
                        }
                    }
                    Some(Ok(Message::Close(_))) => break "closed by the relay".to_string(),
                    Some(Ok(_)) => {}
                }
            }
        }
    };
    // Check the roster while this connection still counts us, then leave.
    let ended = started && end_if_last(client, &parent, &huddle).await;
    let _ = tx.send(Message::Close(None)).await;
    let _ = emit(
        &mut out,
        json!({"type": "closed", "reason": reason, "ended": ended}),
    )
    .await;
    Ok(())
}

/// Opens the huddle's audio WebSocket through the shared v2 handshake.
async fn join(
    client: &BuzzClient,
    parent: &Uuid,
    huddle: &Uuid,
) -> Result<buzz_audio_client::connection::Joined, CliError> {
    let relay = ws_base(client.relay_url());
    let mut request = ws_url(client.relay_url(), &huddle.to_string())
        .into_client_request()
        .map_err(|e| CliError::Other(format!("bad huddle address: {e}")))?;
    request
        .headers_mut()
        .extend(client.upgrade_headers().await?);
    let parent = parent.to_string();
    buzz_audio_client::connection::connect(request, Some(&parent), None, |challenge| async move {
        auth_event(client, &relay, &challenge)
            .await
            .map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| CliError::Other(format!("could not join the huddle: {e}")))
}

/// The NIP-42 event answering the audio socket's challenge.
async fn auth_event(client: &BuzzClient, relay: &str, challenge: &str) -> Result<Value, CliError> {
    let kind = u16::try_from(KIND_AUTH).map_err(|_| CliError::Other("bad auth kind".into()))?;
    let tags = [
        Tag::parse(["relay", relay]).map_err(|e| CliError::Other(format!("tag error: {e}")))?,
        Tag::parse(["challenge", challenge])
            .map_err(|e| CliError::Other(format!("tag error: {e}")))?,
    ];
    let event = client
        .sign_event(EventBuilder::new(Kind::Custom(kind), "").tags(tags))
        .await?;
    serde_json::from_str(&event.as_json()).map_err(|e| CliError::Other(format!("auth event: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn huddle_addresses_follow_the_relay_scheme() {
        let id = "188d5cb8-0068-4e72-a4cd-c640685e9ff7";
        assert_eq!(
            ws_url("https://relay.example.com/", id),
            format!("wss://relay.example.com/huddle/{id}/audio")
        );
        assert_eq!(
            ws_url("http://localhost:3000", id),
            format!("ws://localhost:3000/huddle/{id}/audio")
        );
        assert_eq!(
            ws_base("wss://relay.example.com"),
            "wss://relay.example.com"
        );
    }

    #[test]
    fn huddle_advisories_point_the_parent_at_the_backing_stream() {
        let parent = Uuid::new_v4();
        let huddle = Uuid::new_v4();
        let keys = nostr::Keys::generate();
        for kind in [KIND_HUDDLE_STARTED, KIND_HUDDLE_ENDED] {
            let event = huddle_event(kind, &parent, &huddle)
                .unwrap()
                .sign_with_keys(&keys)
                .unwrap();
            assert_eq!(u32::from(event.kind.as_u16()), kind);
            let content: Value = serde_json::from_str(&event.content).unwrap();
            assert_eq!(content["ephemeral_channel_id"], huddle.to_string());
            let h: Vec<_> = event
                .tags
                .iter()
                .filter(|t| t.as_slice().first().map(String::as_str) == Some("h"))
                .map(|t| t.as_slice()[1].clone())
                .collect();
            assert_eq!(h, vec![parent.to_string()]);
        }
    }

    #[test]
    fn a_huddle_is_ended_only_when_the_roster_is_certainly_just_me() {
        let me = "a".repeat(64);
        let other = "b".repeat(64);
        let roster = |state: &str, complete: bool, who: &[&str]| {
            json!({
                "state": state,
                "complete": complete,
                "participants": who.iter().map(|p| json!({"pubkey": p, "joined_at": 1})).collect::<Vec<_>>(),
            })
        };
        assert!(only_me_left(&roster("active", true, &[&me]), &me));
        assert!(only_me_left(&roster("active", true, &[]), &me));
        assert!(!only_me_left(&roster("active", true, &[&me, &other]), &me));
        assert!(!only_me_left(&roster("active", false, &[&me]), &me));
        assert!(!only_me_left(&roster("ended", true, &[&me]), &me));
        assert!(!only_me_left(&roster("unknown", false, &[]), &me));
        assert!(!only_me_left(&json!({}), &me));
    }
}
