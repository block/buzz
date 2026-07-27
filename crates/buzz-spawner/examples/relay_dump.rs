//! Dev utility: dump spawner-related events (kinds 10180/30178/30179) from a relay.
//! Usage: relay_dump <relay_url> <secret>
use std::time::Duration;

use buzz_ws_client::{parse_relay_message, NostrWsConnection, RelayMessage};
use serde_json::json;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let url = std::env::args().nth(1).expect("relay url");
    let secret = std::env::args().nth(2).expect("secret");
    let keys = nostr::Keys::parse(&secret)?;
    eprintln!("me: {}", keys.public_key().to_hex());

    let mut conn = NostrWsConnection::connect_authenticated(&url, &keys, None).await?;
    let kinds: Vec<u32> = std::env::args()
        .nth(3)
        .unwrap_or_else(|| "10180,30178,30179".into())
        .split(',')
        .filter_map(|k| k.trim().parse().ok())
        .collect();
    conn.send_raw(&json!(["REQ", "dump", {"kinds": kinds, "limit": 200}]))
        .await?;
    loop {
        match conn.next_event(Duration::from_secs(5)).await {
            Ok(RelayMessage::Event { event, .. }) => {
                let d = event
                    .tags
                    .iter()
                    .find_map(|t| {
                        let s = t.clone().to_vec();
                        (s.first().map(String::as_str) == Some("d")).then(|| s.get(1).cloned())
                    })
                    .flatten()
                    .unwrap_or_default();
                println!(
                    "kind={} author={} d={} created_at={} content={}",
                    event.kind,
                    event.pubkey.to_hex(),
                    d,
                    event.created_at,
                    &event.content
                );
            }
            Ok(RelayMessage::Eose { .. }) => break,
            Ok(_) => {}
            Err(e) => {
                eprintln!("recv: {e}");
                break;
            }
        }
    }
    let _ = parse_relay_message; // keep import simple
    conn.disconnect().await.ok();
    Ok(())
}
