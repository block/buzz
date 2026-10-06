//! Bounded NIP-42 audio handshake. Signing and TLS policy belong to the host.

use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use std::{collections::BTreeSet, future::Future, time::Duration};
use tokio_tungstenite::{
    connect_async_tls_with_config,
    tungstenite::{protocol::WebSocketConfig, Message},
    Connector, MaybeTlsStream, WebSocketStream,
};

/// Authenticated audio socket; dropping it closes the connection.
pub type AudioSocket = WebSocketStream<MaybeTlsStream<tokio::net::TcpStream>>;

/// Relay roster entry. Epoch is optional on released v2 relays.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Peer {
    /// Relay-assigned media index.
    pub index: u8,
    /// Hex Nostr public key.
    pub pubkey: String,
    /// Control-plane occupancy epoch, zero on legacy relays.
    pub epoch: u8,
}

/// Successful audio admission, without granting message read/write authority.
pub struct Joined {
    /// Host-owned connection, ready for media and control messages.
    pub socket: AudioSocket,
    /// Index assigned to this publisher.
    pub index: u8,
    /// Initial room roster.
    pub peers: Vec<Peer>,
}

/// Parse a bounded roster without truncating out-of-range indices or epochs.
pub fn parse_peers(value: &Value) -> Result<Vec<Peer>, String> {
    let peers = value["peers"].as_array().ok_or("Invalid Huddle roster")?;
    if peers.len() > 256 {
        return Err("Huddle roster exceeds protocol capacity".into());
    }
    let mut indices = BTreeSet::new();
    peers
        .iter()
        .map(|p| {
            let index = p["peer_index"]
                .as_u64()
                .and_then(|i| u8::try_from(i).ok())
                .ok_or("Invalid Huddle peer index")?;
            let pubkey = p["pubkey"]
                .as_str()
                .filter(|s| s.len() == 64 && s.bytes().all(|c| c.is_ascii_hexdigit()))
                .ok_or("Invalid Huddle peer key")?;
            let epoch = match p.get("epoch") {
                None => 0,
                Some(e) => e
                    .as_u64()
                    .and_then(|i| u8::try_from(i).ok())
                    .ok_or("Invalid Huddle peer epoch")?,
            };
            if !indices.insert(index) {
                return Err("Duplicate Huddle peer".into());
            }
            Ok(Peer {
                index,
                pubkey: pubkey.into(),
                epoch,
            })
        })
        .collect()
}

async fn control(socket: &mut AudioSocket) -> Result<Value, String> {
    loop {
        match socket.next().await {
            Some(Ok(Message::Text(text))) => {
                let value: Value =
                    serde_json::from_str(&text).map_err(|_| "Invalid Huddle response")?;
                if value["type"] == "error" {
                    return Err(value["message"]
                        .as_str()
                        .unwrap_or("Huddle rejected by relay")
                        .chars()
                        .take(240)
                        .collect());
                }
                return Ok(value);
            }
            Some(Ok(Message::Ping(data))) => socket
                .send(Message::Pong(data))
                .await
                .map_err(|_| "Huddle disconnected")?,
            Some(Ok(Message::Pong(_))) => {}
            _ => return Err("Huddle disconnected during admission".into()),
        }
    }
}

/// Connect and authenticate using a host-provided asynchronous signer.
///
/// The host validates the destination before calling, and constructs the NIP-42
/// event (including its exact relay tag) from the challenge. No private key is
/// accepted. Dropping this future cancels connection/admission; the entire
/// operation is bounded to 15 seconds, including signing.
pub async fn connect<F, Fut>(
    audio_url: &str,
    parent: Option<&str>,
    connector: Option<Connector>,
    sign: F,
) -> Result<Joined, String>
where
    F: FnOnce(String) -> Fut,
    Fut: Future<Output = Result<Value, String>>,
{
    tokio::time::timeout(Duration::from_secs(15), async {
        let config = WebSocketConfig::default()
            .max_message_size(Some(65_536))
            .max_frame_size(Some(65_536));
        let (mut socket, _) =
            connect_async_tls_with_config(audio_url, Some(config), false, connector)
                .await
                .map_err(|e| format!("audio WS connect failed: {e}"))?;
        let challenge = tokio::time::timeout(Duration::from_secs(5), control(&mut socket))
            .await
            .map_err(|_| "Huddle challenge timed out")??;
        if challenge["type"] != "challenge" {
            return Err("Invalid Huddle handshake".into());
        }
        let challenge = challenge["challenge"]
            .as_str()
            .filter(|s| !s.is_empty() && s.len() <= 4096)
            .ok_or("Invalid Huddle challenge")?
            .to_owned();
        let event = sign(challenge).await?;
        socket
            .send(Message::Text(
                json!({
                    "type": "auth", "event": event, "parent_channel_id": parent,
                    "protocol_version": crate::wire::PROTOCOL_VERSION,
                })
                .to_string()
                .into(),
            ))
            .await
            .map_err(|_| "Huddle authentication failed")?;
        let joined = tokio::time::timeout(Duration::from_secs(5), control(&mut socket))
            .await
            .map_err(|_| "Huddle admission timed out")??;
        if joined["type"] != "joined" {
            return Err("Huddle admission was not confirmed".into());
        }
        let index = joined["peer_index"]
            .as_u64()
            .and_then(|i| u8::try_from(i).ok())
            .ok_or("Invalid Huddle publisher index")?;
        let peers = parse_peers(&joined)?;
        Ok(Joined {
            socket,
            index,
            peers,
        })
    })
    .await
    .map_err(|_| "Huddle connection timed out")?
}
