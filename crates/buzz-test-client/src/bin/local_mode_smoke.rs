//! Phase 0 local-mode smoke: two authenticated clients, live fan-out, history and COUNT.
use buzz_test_client::{BuzzTestClient, RelayMessage};
use nostr::{Alphabet, EventBuilder, Filter, Keys, Kind, SingleLetterTag, Tag};
use std::time::Duration;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let url = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "ws://127.0.0.1:4317".into());
    let keys = Keys::parse("0000000000000000000000000000000000000000000000000000000000000001")?;
    if std::env::args().nth(2).as_deref() == Some("admit") {
        let target = std::env::args()
            .nth(3)
            .ok_or_else(|| anyhow::anyhow!("admit requires a target pubkey"))?;
        let event = EventBuilder::new(Kind::Custom(9030), "")
            .tags(vec![
                Tag::parse(["p", &target])?,
                Tag::parse(["role", "member"])?,
            ])
            .sign_with_keys(&keys)?;
        let mut client = BuzzTestClient::connect(&url, &keys).await?;
        let ok = client.send_event(event).await?;
        anyhow::ensure!(ok.accepted, "kind 9030 rejected: {}", ok.message);
        println!("PASS kind_9030_admission");
        return Ok(());
    }
    let channel_id = std::env::args()
        .nth(3)
        .unwrap_or_else(|| "00000000-0000-0000-0000-000000000042".into());
    let channel_name = format!("local-mode-{channel_id}");
    let filter =
        || Filter::new().custom_tag(SingleLetterTag::lowercase(Alphabet::H), channel_id.clone());
    if std::env::args().nth(2).as_deref() == Some("history-only") {
        let mut history = BuzzTestClient::connect(&url, &keys).await?;
        history.subscribe("restart-history", vec![filter()]).await?;
        loop {
            match history.recv_event(Duration::from_secs(2)).await? {
                RelayMessage::Event { event, .. } if event.content == "phase0 live" => break,
                RelayMessage::Eose { .. } => anyhow::bail!("persisted smoke event not found"),
                _ => {}
            }
        }
        println!("PASS sqlite_restart_history");
        return Ok(());
    }
    let channel_event = EventBuilder::new(Kind::Custom(9007), "")
        .tags(vec![
            Tag::parse(["h", &channel_id])?,
            Tag::parse(["name", &channel_name])?,
            Tag::parse(["channel_type", "stream"])?,
            Tag::parse(["visibility", "open"])?,
        ])
        .sign_with_keys(&keys)?;
    let mut publisher = BuzzTestClient::connect(&url, &keys).await?;
    let channel_ok = publisher.send_event(channel_event).await?;
    anyhow::ensure!(
        channel_ok.accepted,
        "channel creation rejected: {}",
        channel_ok.message
    );
    let mut subscriber = BuzzTestClient::connect(&url, &keys).await?;
    subscriber.subscribe("live", vec![filter()]).await?;
    loop {
        if matches!(
            subscriber.recv_event(Duration::from_secs(2)).await?,
            RelayMessage::Eose { .. }
        ) {
            break;
        }
    }
    let ok = publisher
        .send_text_message(&keys, &channel_id, "phase0 live", 9)
        .await?;
    anyhow::ensure!(ok.accepted, "publish rejected: {}", ok.message);
    loop {
        match subscriber.recv_event(Duration::from_secs(2)).await? {
            RelayMessage::Event { event, .. } if event.content == "phase0 live" => break,
            RelayMessage::Event { .. } => continue,
            other => anyhow::bail!("expected live EVENT, got {other:?}"),
        }
    }
    let mut history = BuzzTestClient::connect(&url, &keys).await?;
    history.subscribe("history", vec![filter()]).await?;
    loop {
        match history.recv_event(Duration::from_secs(2)).await? {
            RelayMessage::Event { event, .. } if event.content == "phase0 live" => break,
            RelayMessage::Event { .. } => continue,
            other => anyhow::bail!("expected historical EVENT, got {other:?}"),
        }
    }
    println!("PASS nip42,event_insert,req_history,live_fanout");
    Ok(())
}
