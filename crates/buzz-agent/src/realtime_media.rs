//! Opt-in ACP media boundary. Capture and control use separate bounded queues.

use base64::{engine::general_purpose::STANDARD, Engine};
use serde::Deserialize;
use serde_json::{json, Value};
use tokio::sync::mpsc;

use crate::{wire, App};

pub(crate) const VERSION: u64 = 1;
pub(crate) const FRAME_BYTES: usize = 4800;
pub(crate) const QUEUE_FRAMES: usize = 8;
pub(crate) const PREFIX: &str = "_buzz/unstable/realtime/";

pub(crate) struct Command {
    pub id: Value,
    pub action: Action,
}

pub(crate) enum Action {
    Append {
        sequence: u64,
        pcm: Vec<u8>,
        playback_pcm: Option<Vec<u8>>,
    },
    Playback(Playback),
    Interrupt(Option<Playback>),
    Close,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct Playback {
    pub response_id: String,
    pub item_id: String,
    pub content_index: u32,
    pub played_samples: u64,
    #[serde(default)]
    pub stopped: bool,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Params {
    session_id: String,
    stream_id: String,
    #[serde(default)]
    sequence: Option<u64>,
    #[serde(default)]
    data: Option<String>,
    #[serde(default)]
    playback_data: Option<String>,
    #[serde(default)]
    playback: Option<Playback>,
}

pub(crate) struct Ingress {
    pub stream_id: String,
    audio: mpsc::Sender<Command>,
    control: mpsc::Sender<Command>,
}

pub(crate) struct Receiver {
    pub audio: mpsc::Receiver<Command>,
    pub control: mpsc::Receiver<Command>,
}

pub(crate) fn channel(stream_id: String) -> (Ingress, Receiver) {
    let (audio, audio_rx) = mpsc::channel(QUEUE_FRAMES);
    let (control, control_rx) = mpsc::channel(QUEUE_FRAMES);
    (
        Ingress {
            stream_id,
            audio,
            control,
        },
        Receiver {
            audio: audio_rx,
            control: control_rx,
        },
    )
}

pub(crate) fn enabled(meta: &Value) -> bool {
    meta.get("buzz")
        .and_then(|v| v.get("realtimeAudio"))
        .and_then(Value::as_u64)
        == Some(VERSION)
}

fn action(operation: &str, params: &mut Params) -> Result<Action, &'static str> {
    if operation != "append" && params.playback_data.is_some() {
        return Err("playback audio is only valid with capture");
    }
    match operation {
        "append" if params.playback.is_none() => {
            let sequence = params.sequence.ok_or("missing sequence")?;
            let data = params.data.take().ok_or("missing PCM data")?;
            if data.len() > FRAME_BYTES.div_ceil(3) * 4 {
                return Err("PCM frame exceeds 100 ms");
            }
            let pcm = STANDARD.decode(data).map_err(|_| "invalid PCM base64")?;
            if pcm.is_empty() || pcm.len() > FRAME_BYTES || !pcm.len().is_multiple_of(2) {
                return Err("invalid PCM16 frame length");
            }
            let playback_pcm = params
                .playback_data
                .take()
                .map(|data| {
                    if data.len() > FRAME_BYTES.div_ceil(3) * 4 {
                        return Err("playback frame exceeds 100 ms");
                    }
                    let samples = STANDARD
                        .decode(data)
                        .map_err(|_| "invalid playback PCM base64")?;
                    if samples.len() != pcm.len() {
                        return Err("duplex capture lengths differ");
                    }
                    Ok(samples)
                })
                .transpose()?;
            Ok(Action::Append {
                sequence,
                pcm,
                playback_pcm,
            })
        }
        "playback" if params.sequence.is_none() && params.data.is_none() => Ok(Action::Playback(
            params.playback.take().ok_or("missing playback position")?,
        )),
        "interrupt" if params.sequence.is_none() && params.data.is_none() => {
            Ok(Action::Interrupt(params.playback.take()))
        }
        "close"
            if params.sequence.is_none() && params.data.is_none() && params.playback.is_none() =>
        {
            Ok(Action::Close)
        }
        _ => Err("unsupported media operation or fields"),
    }
}

pub(crate) async fn dispatch(
    app: &App,
    operation: &str,
    id: Value,
    value: Value,
    output: &wire::WireSender,
) {
    let result = async {
        let mut params: Params =
            serde_json::from_value(value).map_err(|_| "invalid media params")?;
        let action = action(operation, &mut params)?;
        let sessions = app.sessions.lock().await;
        let ingress = sessions
            .get(&params.session_id)
            .and_then(|s| s.media.as_ref())
            .ok_or("no active media session")?;
        if ingress.stream_id != params.stream_id {
            return Err("stale media stream");
        }
        let queue = if matches!(action, Action::Append { .. }) {
            &ingress.audio
        } else {
            &ingress.control
        };
        queue
            .try_send(Command {
                id: id.clone(),
                action,
            })
            .map_err(|_| "media queue full or closed")
    }
    .await;
    if let Err(message) = result {
        wire::send(output, wire::err(id, wire::INVALID_PARAMS, message)).await;
    }
}

pub(crate) async fn notify(
    output: &wire::WireSender,
    session_id: &str,
    stream_id: &str,
    update: Value,
) -> Result<(), crate::AgentError> {
    tokio::time::timeout(
        std::time::Duration::from_secs(2),
        wire::send_checked(
            output,
            json!({
                "jsonrpc":"2.0", "method":"_buzz/unstable/realtime/update",
                "params":{"sessionId":session_id,"streamId":stream_id,"update":update}
            }),
        ),
    )
    .await
    .map_err(|_| crate::AgentError::Llm("realtime: media consumer stalled".into()))?
    .map_err(|_| crate::AgentError::Cancelled)
}
