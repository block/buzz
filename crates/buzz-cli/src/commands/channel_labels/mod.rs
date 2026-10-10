//! Capability-gated NIP-CL commands and verified reads. No current-state receipt inference.
mod journal;
#[cfg(test)]
mod tests;

use std::path::Path;

use buzz_core::channel_labels::{verify_snapshot, CommandOutcome, LabelCommand, LabelSet};
use nostr::{Event, EventBuilder, EventId, Kind, PublicKey, Tag};
use serde_json::{json, Value};
use uuid::Uuid;

use crate::{client::BuzzClient, error::CliError, ChannelLabelsCmd};
use journal::{CommandRecord, Journal};

fn invalid(error: impl std::fmt::Display) -> CliError {
    CliError::Usage(error.to_string())
}

async fn capability(
    client: &BuzzClient,
    trusted: Option<PublicKey>,
) -> Result<PublicKey, CliError> {
    let url = url::Url::parse(client.relay_url()).map_err(invalid)?;
    if url.scheme() != "https" && trusted.is_none() {
        return Err(invalid(
            "NIP-CL over plain HTTP requires --trusted-relay <hex> from an out-of-band source",
        ));
    }
    let raw = client.nip11_for_channel_labels().await?;
    let info: Value = serde_json::from_str(&raw).map_err(invalid)?;
    if !info["supported_extensions"]
        .as_array()
        .is_some_and(|ext| ext.iter().any(|v| v == "nip-cl"))
    {
        return Err(invalid(
            "target relay does not advertise nip-cl; no label command was sent",
        ));
    }
    let relay = PublicKey::from_hex(
        info["self"]
            .as_str()
            .ok_or_else(|| invalid("NIP-11 missing self identity"))?,
    )
    .map_err(invalid)?;
    if trusted.is_some_and(|key| key != relay) {
        return Err(invalid(
            "target relay identity changed; recovery journal and snapshots cannot be retargeted",
        ));
    }
    Ok(relay)
}

fn trusted_key(value: Option<&str>) -> Result<Option<PublicKey>, CliError> {
    value.map(PublicKey::from_hex).transpose().map_err(invalid)
}

pub(super) async fn create(
    client: &BuzzClient,
    builder: EventBuilder,
    labels: Vec<String>,
    path: &Path,
    trusted: Option<&str>,
) -> Result<(), CliError> {
    // Validate raw tags through the same grammar, not only a deduplicated set.
    let tags = labels
        .iter()
        .map(|value| Tag::parse(["label", value]))
        .collect::<Result<Vec<_>, _>>()
        .map_err(invalid)?;
    prepare(
        client,
        client.sign_event(builder.tags(tags))?,
        path,
        trusted,
    )
    .await
}

async fn prepare(
    client: &BuzzClient,
    event: Event,
    path: &Path,
    trusted: Option<&str>,
) -> Result<(), CliError> {
    LabelCommand::parse(&event)
        .map_err(invalid)?
        .ok_or_else(|| invalid("not a NIP-CL command"))?;
    let relay = capability(client, trusted_key(trusted)?).await?;
    let mut journal = Journal::create(
        path,
        CommandRecord {
            version: 1,
            relay_url: client.relay_url().to_owned(),
            relay_key: relay,
            event,
        },
    )?;
    send(client, &mut journal, path).await
}

async fn send(client: &BuzzClient, journal: &mut Journal, path: &Path) -> Result<(), CliError> {
    let previous = journal.outcome;
    if previous != CommandOutcome::Committed {
        journal.persist(previous.before_send())?;
        // Exactly one attempt: transport/generic errors are Unknown, even on the
        // first attempt. Retrying uses this event and a new NIP-98 auth proof.
        let response = client
            .post_json_once_authed("/events", &json!(journal.record.event))
            .await;
        let outcome = response
            .ok()
            .and_then(|raw| serde_json::from_str::<Value>(&raw).ok())
            .and_then(|body| {
                Some(previous.acknowledge(
                    journal.record.event.id,
                    EventId::from_hex(body["event_id"].as_str()?).ok()?,
                    body["accepted"].as_bool()?,
                    body["message"].as_str()?,
                ))
            })
            .unwrap_or(CommandOutcome::Unknown);
        journal.persist(outcome).map_err(|e| {
            CliError::DeliveryUnknown(format!(
                "{e}; event {} is retained at {}",
                journal.record.event.id,
                path.display()
            ))
        })?;
    }
    let channel = LabelCommand::parse(&journal.record.event)
        .map_err(invalid)?
        .ok_or_else(|| invalid("not a NIP-CL command"))?
        .channel();
    println!(
        "{}",
        json!({"event_id": journal.record.event.id, "channel_id":channel,
        "outcome":journal.outcome, "command_file":path, "accepted":journal.outcome == CommandOutcome::Committed})
    );
    match journal.outcome {
        CommandOutcome::Committed => Ok(()),
        CommandOutcome::Rejected => Err(CliError::Other(format!("NIP-CL rejected new command {}; retained at {}", journal.record.event.id, path.display()))),
        _ => Err(CliError::DeliveryUnknown(format!("NIP-CL outcome unknown; retry only with channels labels retry --command-file {}; do not create a new event", path.display()))),
    }
}

async fn read(
    client: &BuzzClient,
    relay: PublicKey,
    filter: Value,
    expected: Option<Uuid>,
) -> Result<Value, CliError> {
    let raw = client.query(&filter).await?;
    // Parsing and Schnorr verification scale with the result page; do not run
    // that CPU work on the async executor.
    tokio::task::spawn_blocking(move || verified_snapshots(&raw, relay, expected))
        .await
        .map_err(|e| CliError::Other(format!("snapshot verification task failed: {e}")))?
}

fn verified_snapshots(
    raw: &str,
    relay: PublicKey,
    expected: Option<Uuid>,
) -> Result<Value, CliError> {
    let events: Vec<Event> = serde_json::from_str(raw).map_err(invalid)?;
    let mut current = std::collections::BTreeMap::new();
    for event in events {
        let channel = event
            .tags
            .iter()
            .find_map(|tag| {
                let parts = tag.as_slice();
                (parts[0] == "d").then(|| parts.get(1)).flatten()
            })
            .ok_or_else(|| invalid("metadata lacks channel identifier"))?;
        let channel = Uuid::parse_str(channel).map_err(invalid)?;
        if expected.is_some_and(|id| id != channel) {
            return Err(invalid("metadata identifies another channel"));
        }
        let labels = verify_snapshot(&event, relay, channel).map_err(invalid)?;
        let replace = current
            .get(&channel)
            .is_none_or(|(previous, _): &(Event, LabelSet)| {
                event.created_at > previous.created_at
                    || (event.created_at == previous.created_at && event.id < previous.id)
            });
        if replace {
            current.insert(channel, (event, labels));
        }
    }
    let mut current: Vec<_> = current.into_iter().collect();
    current.sort_by(|(_, (a, _)), (_, (b, _))| {
        b.created_at
            .cmp(&a.created_at)
            .then_with(|| a.id.cmp(&b.id))
    });
    Ok(Value::Array(
        current
            .into_iter()
            .map(|(channel, (event, labels))| {
                json!({
                    "channel_id":channel, "labels":labels.values(), "event":event
                })
            })
            .collect(),
    ))
}

pub(super) async fn dispatch(
    command: ChannelLabelsCmd,
    client: &BuzzClient,
) -> Result<(), CliError> {
    match command {
        ChannelLabelsCmd::Get {
            channel,
            trusted_relay,
        } => {
            let channel = crate::validate::parse_uuid(&channel)?;
            let relay = capability(client, trusted_key(trusted_relay.as_deref())?).await?;
            println!(
                "{}",
                read(
                    client,
                    relay,
                    json!({"kinds":[39000],"authors":[relay],"#d":[channel]}),
                    Some(channel)
                )
                .await?
            );
            Ok(())
        }
        ChannelLabelsCmd::Find {
            labels,
            channel_type,
            limit,
            trusted_relay,
        } => {
            let labels = LabelSet::new(labels).map_err(invalid)?;
            let relay = capability(client, trusted_key(trusted_relay.as_deref())?).await?;
            let mut filter = json!({"kinds":[39000],"authors":[relay],"#L":["nip-cl"],"#l":labels.values(),"limit":limit});
            if let Some(kind) = channel_type {
                filter["#t"] = json!([kind.to_string()]);
            }
            println!("{}", read(client, relay, filter, None).await?);
            Ok(())
        }
        ChannelLabelsCmd::Update {
            channel,
            add,
            remove,
            command_file,
            trusted_relay,
        } => {
            let channel = crate::validate::parse_uuid(&channel)?;
            let mut tags = vec![Tag::parse(["h", &channel.to_string()]).map_err(invalid)?];
            for (name, values) in [("add-label", add), ("remove-label", remove)] {
                for value in values {
                    tags.push(Tag::parse([name, &value]).map_err(invalid)?);
                }
            }
            let event = client.sign_event(EventBuilder::new(Kind::Custom(9002), "").tags(tags))?;
            prepare(client, event, &command_file, trusted_relay.as_deref()).await
        }
        ChannelLabelsCmd::Retry { command_file } => {
            let mut journal = Journal::open(&command_file).map_err(|e| {
                CliError::DeliveryUnknown(format!(
                    "{e}; preserve {}; do not generate a replacement command",
                    command_file.display()
                ))
            })?;
            if journal.record.relay_url != client.relay_url()
                || journal.record.event.pubkey != client.keys().public_key()
            {
                return Err(invalid(
                    "command recovery is bound to its original relay URL and author",
                ));
            }
            buzz_core::verify_event(&journal.record.event).map_err(invalid)?;
            LabelCommand::parse(&journal.record.event)
                .map_err(invalid)?
                .ok_or_else(|| invalid("journal is not a NIP-CL command"))?;
            if journal.outcome != CommandOutcome::Committed {
                capability(client, Some(journal.record.relay_key))
                    .await
                    .map_err(|e| {
                        if journal.outcome == CommandOutcome::Unknown {
                            CliError::DeliveryUnknown(format!(
                                "{e}; earlier application remains unknown"
                            ))
                        } else {
                            e
                        }
                    })?;
            }
            send(client, &mut journal, &command_file).await
        }
    }
}
