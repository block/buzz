//! Manage the caller's encrypted NIP-ER reminders.

use buzz_sdk::reminders::{self, Content, Reminder, Status};
use clap::{Args, Subcommand};
use nostr::{Event, Timestamp};
use serde_json::{json, Value};

use crate::{client::BuzzClient, error::CliError, links::parse_message_link};

#[derive(Subcommand)]
pub enum RemindersCmd {
    /// Schedule a private follow-up. Preserve why, what to inspect, and context.
    Create {
        #[command(flatten)]
        time: DueTime,
        /// Reason and context for the follow-up; use '-' to read stdin.
        #[arg(long)]
        note: String,
        /// Optional original Buzz message link.
        #[arg(long)]
        link: Option<String>,
    },
    /// List current reminder heads, decrypted for your identity.
    List {
        /// Show terminal states as well as pending reminders.
        #[arg(long)]
        all: bool,
        /// Show only reminders that are due now.
        #[arg(long, conflicts_with = "all")]
        due: bool,
    },
    /// Inspect a reminder's current version before acting.
    Get { id: String },
    /// Reconsider later; optionally replace the reason or context.
    Snooze {
        id: String,
        #[command(flatten)]
        time: DueTime,
        #[arg(long)]
        note: Option<String>,
        /// Refuse the update if another client changed this event version.
        #[arg(long)]
        if_event: Option<String>,
    },
    /// Record that you have completed or acknowledged this follow-up.
    Complete {
        id: String,
        #[arg(long)]
        if_event: Option<String>,
    },
    /// Cancel a follow-up that is no longer useful.
    Cancel {
        id: String,
        #[arg(long)]
        if_event: Option<String>,
    },
}

#[derive(Args)]
pub struct DueTime {
    /// Absolute RFC3339 time with a timezone (e.g. 2026-09-14T14:00:00Z).
    #[arg(long, required_unless_present = "after", conflicts_with = "after")]
    at: Option<String>,
    /// Delay such as 30s, 20m, 4h, or 7d.
    #[arg(long, required_unless_present = "at", conflicts_with = "at")]
    after: Option<String>,
}

impl DueTime {
    fn resolve(&self, now: u64) -> Result<u64, CliError> {
        let time = if let Some(at) = &self.at {
            let timestamp = chrono::DateTime::parse_from_rfc3339(at)
                .map_err(|_| {
                    CliError::Usage("--at needs an RFC3339 timestamp with timezone".into())
                })?
                .timestamp();
            u64::try_from(timestamp)
                .map_err(|_| CliError::Usage("--at predates Unix time".into()))?
        } else {
            let raw = self.after.as_deref().unwrap_or("");
            let split = raw
                .len()
                .checked_sub(1)
                .filter(|&i| raw.is_char_boundary(i))
                .ok_or_else(|| {
                    CliError::Usage("--after needs a duration such as 20m or 7d".into())
                })?;
            let (number, unit) = raw.split_at(split);
            let multiplier = match unit {
                "s" => 1,
                "m" => 60,
                "h" => 3600,
                "d" => 86_400,
                _ => return Err(CliError::Usage("--after units are s, m, h, d".into())),
            };
            let seconds = reminders::parse_not_before(number)
                .map_err(input_error)?
                .checked_mul(multiplier)
                .ok_or_else(|| CliError::Usage("duration overflow".into()))?;
            now.checked_add(seconds)
                .ok_or_else(|| CliError::Usage("due time overflow".into()))?
        };
        if time <= now {
            return Err(CliError::Usage("choose a future reminder time".into()));
        }
        reminders::parse_not_before(&time.to_string()).map_err(input_error)
    }
}

fn input_error(error: buzz_sdk::SdkError) -> CliError {
    CliError::Usage(error.to_string())
}

fn read_note(note: String) -> Result<String, CliError> {
    if note != "-" {
        return Ok(note);
    }
    std::io::read_to_string(std::io::stdin())
        .map_err(|error| CliError::Other(format!("cannot read reminder note: {error}")))
}

async fn fetch(client: &BuzzClient, id: Option<&str>) -> Result<Vec<Reminder>, CliError> {
    let mut filter = json!({"kinds": [30300], "authors": [client.keys().public_key().to_hex()]});
    if let Some(id) = id {
        filter["#d"] = json!([id]);
    }
    let values = client.query_all(filter).await?;
    let events = values
        .into_iter()
        .filter_map(|v| serde_json::from_value::<Event>(v).ok())
        .collect();
    Ok(reminders::current_heads(events, client.keys()))
}

async fn head(client: &BuzzClient, id: &str) -> Result<Reminder, CliError> {
    fetch(client, Some(id))
        .await?
        .into_iter()
        .next()
        .ok_or_else(|| CliError::NotFound("no valid reminder for this identity and ID".into()))
}

async fn target(client: &BuzzClient, link: &str) -> Result<Value, CliError> {
    let link = parse_message_link(link)?;
    let raw = client
        .query(&json!({"ids": [link.message_id], "kinds": [9, 11, 1111, 40002, 40007]}))
        .await?;
    let events: Vec<Event> = serde_json::from_str(&raw)
        .map_err(|_| CliError::Other("invalid message response".into()))?;
    let event = events
        .into_iter()
        .find(|event| event.id.to_hex() == link.message_id)
        .ok_or_else(|| CliError::NotFound("reminder target message not found".into()))?;
    event
        .verify()
        .map_err(|_| CliError::Other("invalid target message signature".into()))?;
    if !event
        .tags
        .iter()
        .any(|tag| tag.kind().as_str() == "h" && tag.content() == Some(&link.channel_id))
    {
        return Err(CliError::Usage(
            "target message does not belong to the linked channel".into(),
        ));
    }
    Ok(
        json!({"id": event.id.to_hex(), "eventId": event.id.to_hex(), "channelId": link.channel_id,
        "preview": event.content.chars().take(500).collect::<String>(), "authorPubkey": event.pubkey.to_hex()}),
    )
}

async fn publish(
    client: &BuzzClient,
    id: &str,
    content: &Content,
    due: Option<u64>,
    created_at: u64,
) -> Result<(), CliError> {
    let builder =
        reminders::build(client.keys(), id, content, due, created_at).map_err(input_error)?;
    let event = client.sign_event(builder)?;
    let event_id = event.id.to_hex();
    let raw = client.submit_event(event).await?;
    super::parse_write_response(&raw, "reminder was superseded; inspect its current head")?;
    let current = head(client, id).await?;
    if current.event_id != event_id {
        return Err(CliError::Conflict(
            "reminder changed concurrently; inspect its current head".into(),
        ));
    }
    println!(
        "{}",
        json!({"accepted":true,"event_id":event_id,"reminder_id":id,"not_before":due,"status":content.status})
    );
    Ok(())
}

async fn update(
    client: &BuzzClient,
    id: String,
    status: Status,
    time: Option<DueTime>,
    note: Option<String>,
    expected: Option<String>,
) -> Result<(), CliError> {
    let mut reminder = head(client, &id).await?;
    if expected
        .as_deref()
        .is_some_and(|event| event != reminder.event_id)
    {
        return Err(CliError::Conflict(
            "reminder changed; inspect its current head".into(),
        ));
    }
    if reminder.content.status != Status::Pending {
        return Err(CliError::Usage(
            "reminder is already terminal; create a new reminder if needed".into(),
        ));
    }
    let now = Timestamp::now().as_secs();
    if status == Status::Done && reminder.not_before.is_some_and(|time| time > now) {
        return Err(CliError::Usage(
            "reminder is not due yet; cancel it if the reason no longer applies".into(),
        ));
    }
    let due = time.as_ref().map(|time| time.resolve(now)).transpose()?;
    reminder.content.status = status;
    if let Some(note) = note {
        reminder.content.note = Some(read_note(note)?);
    }
    publish(
        client,
        &id,
        &reminder.content,
        due,
        now.max(reminder.created_at.saturating_add(1)),
    )
    .await
}

pub async fn dispatch(command: RemindersCmd, client: &BuzzClient) -> Result<(), CliError> {
    if matches!(command, RemindersCmd::Create { .. }) {
        let info: Value = reqwest::Client::new()
            .get(client.relay_url())
            .header("Accept", "application/nostr+json")
            .timeout(std::time::Duration::from_secs(10))
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        if !reminders::relay_supports_private_reminders(&info) {
            return Err(CliError::Usage(
                "relay does not advertise author-private NIP-ER and NIP-42 support".into(),
            ));
        }
    }
    match command {
        RemindersCmd::Create { time, note, link } => {
            let now = Timestamp::now().as_secs();
            let due = time.resolve(now)?;
            let content = Content {
                status: Status::Pending,
                note: Some(read_note(note)?),
                target: match link {
                    Some(link) => Some(target(client, &link).await?),
                    None => None,
                },
                extra: Default::default(),
            };
            publish(client, &reminders::new_id(), &content, Some(due), now).await
        }
        RemindersCmd::List { all, due } => {
            let now = Timestamp::now().as_secs();
            let reminders: Vec<_> = fetch(client, None)
                .await?
                .into_iter()
                .filter(|r| (all || r.content.status == Status::Pending) && (!due || r.is_due(now)))
                .collect();
            println!("{}", json!(reminders));
            Ok(())
        }
        RemindersCmd::Get { id } => {
            println!("{}", json!(head(client, &id).await?));
            Ok(())
        }
        RemindersCmd::Snooze {
            id,
            time,
            note,
            if_event,
        } => update(client, id, Status::Pending, Some(time), note, if_event).await,
        RemindersCmd::Complete { id, if_event } => {
            update(client, id, Status::Done, None, None, if_event).await
        }
        RemindersCmd::Cancel { id, if_event } => {
            update(client, id, Status::Cancelled, None, None, if_event).await
        }
    }
}

#[cfg(test)]
#[path = "reminders_tests.rs"]
mod tests;
