//! Experimental signed interaction commands over the existing HTTP event bridge.

use std::{io::Read, time::Duration};

use buzz_core::interaction::{Prompt, EXTENSION, MAX_LIFETIME};
use buzz_core::kind::*;
use clap::{Args, Subcommand};
use nostr::{Event, EventBuilder, EventId, Kind, PublicKey, Tag, Timestamp};
use serde_json::{json, Value};

use crate::{
    client::{normalize_write_response, BuzzClient},
    error::CliError,
};

#[derive(Subcommand)]
/// Commands for the relay's experimental signed interaction capability.
pub enum InteractionsCmd {
    /// Post a signed buttons/form prompt (requires experimental relay capability)
    Ask(AskArgs),
    /// Post a poll (default close: expiry)
    Poll(AskArgs),
    /// Answer a prompt; repeat --choice or --value for multiple inputs
    Answer {
        #[arg(long)]
        prompt: String,
        #[arg(long = "choice")]
        choices: Vec<String>,
        #[arg(long = "value")]
        values: Vec<String>,
        #[arg(long, default_value = "")]
        comment: String,
    },
    /// Read canonical signed prompt and current relay state as a JSON array
    Get {
        #[arg(long)]
        prompt: String,
    },
    /// Wait for close; print the signed final state as a one-element JSON array
    Wait {
        #[arg(long)]
        prompt: String,
        #[arg(long, default_value = "24h")]
        timeout: String,
    },
    /// Close a prompt as its asker, preserving its event and decision history
    Close {
        #[arg(long)]
        prompt: String,
    },
}

#[derive(Args)]
/// Schema and delivery options for a prompt, or an unsigned JSON envelope.
pub struct AskArgs {
    #[arg(long, required_unless_present = "json")]
    channel: Option<String>,
    #[arg(long, required_unless_present = "json")]
    text: Option<String>,
    #[arg(long = "type", default_value = "buttons")]
    itype: String,
    /// id:label[:primary|danger|default]
    #[arg(long = "option")]
    options: Vec<String>,
    /// id:label:type:required|optional
    #[arg(long = "field")]
    fields: Vec<String>,
    /// field=value1,value2 (use --json for labels different from IDs)
    #[arg(long = "select")]
    selects: Vec<String>,
    #[arg(long, default_value = "members")]
    responders: String,
    #[arg(long = "responder")]
    listed: Vec<String>,
    #[arg(long)]
    min: Option<usize>,
    #[arg(long)]
    max: Option<usize>,
    #[arg(long)]
    closes: Option<String>,
    #[arg(long, default_value = "24h")]
    expires: String,
    #[arg(long)]
    reply_to: Option<String>,
    /// Raw unsigned {content,tags}; inline JSON, @path, or - for stdin
    #[arg(long, conflicts_with_all = ["channel", "text", "options", "fields", "selects", "listed", "reply_to", "min", "max", "closes"])]
    json: Option<String>,
}

fn usage(error: impl std::fmt::Display) -> CliError {
    CliError::Usage(error.to_string())
}
fn parse_tag(parts: Vec<String>) -> Result<Tag, CliError> {
    Tag::parse(parts).map_err(usage)
}
fn pair(name: &str, value: impl ToString) -> Result<Tag, CliError> {
    parse_tag(vec![name.into(), value.to_string()])
}
fn duration(value: &str) -> Result<Duration, CliError> {
    let (number, multiplier) = match value.chars().last() {
        Some('s') => (&value[..value.len() - 1], 1),
        Some('m') => (&value[..value.len() - 1], 60),
        Some('h') => (&value[..value.len() - 1], 3600),
        Some('d') => (&value[..value.len() - 1], 86400),
        _ => (value, 1),
    };
    let seconds = number
        .parse::<u64>()
        .ok()
        .and_then(|n| n.checked_mul(multiplier))
        .filter(|n| *n > 0 && *n <= MAX_LIFETIME)
        .ok_or_else(|| usage("duration must be 1s to 30d"))?;
    Ok(Duration::from_secs(seconds))
}

async fn capability(client: &BuzzClient) -> Result<PublicKey, CliError> {
    let raw = client.get_public("/info").await?;
    let info: Value = serde_json::from_str(&raw).map_err(usage)?;
    relay_identity(&info)
}
fn relay_identity(info: &Value) -> Result<PublicKey, CliError> {
    if !info
        .get("supported_extensions")
        .and_then(Value::as_array)
        .is_some_and(|extensions| extensions.iter().any(|v| v == EXTENSION))
    {
        return Err(usage(format!(
            "relay has not enabled {EXTENSION} (BUZZ_EXPERIMENTAL_INTERACTIONS=true)"
        )));
    }
    let key = info
        .get("self")
        .and_then(Value::as_str)
        .ok_or_else(|| usage("relay does not advertise its state signing key"))?;
    PublicKey::from_hex(key).map_err(usage)
}

async fn read_prompt(client: &BuzzClient, id: &str) -> Result<(Event, Prompt), CliError> {
    let id = EventId::from_hex(id).map_err(usage)?;
    let raw = client
        .query(&json!({"kinds":[KIND_INTERACTION_PROMPT],"ids":[id.to_hex()],"limit":1}))
        .await?;
    let events: Vec<Event> = serde_json::from_str(&raw).map_err(usage)?;
    let event = events
        .into_iter()
        .find(|e| e.id == id && e.kind.as_u16() as u32 == KIND_INTERACTION_PROMPT)
        .ok_or_else(|| CliError::NotFound("prompt not found or inaccessible".into()))?;
    event.verify().map_err(usage)?;
    let schema = Prompt::parse(&event).map_err(usage)?;
    Ok((event, schema))
}
async fn read_state(
    client: &BuzzClient,
    prompt: &Event,
    schema: &Prompt,
    relay: PublicKey,
) -> Result<Option<Event>, CliError> {
    let raw = client.query(&json!({"kinds":[KIND_INTERACTION_STATE],"authors":[relay.to_hex()],"#d":[prompt.id.to_hex()],"#h":[schema.channel.to_string()],"limit":1})).await?;
    let events: Vec<Event> = serde_json::from_str(&raw).map_err(usage)?;
    let Some(event) = events.into_iter().next() else {
        return Ok(None);
    };
    event.verify().map_err(usage)?;
    if event.pubkey != relay
        || event.kind.as_u16() as u32 != KIND_INTERACTION_STATE
        || buzz_core::interaction::single_tag(&event, "d").map_err(usage)?
            != Some(prompt.id.to_hex().as_str())
        || buzz_core::interaction::channel(&event).map_err(usage)? != schema.channel
    {
        return Err(usage(
            "relay returned a state for the wrong signer or prompt",
        ));
    }
    Ok(Some(event))
}

async fn ask_builder(
    client: &BuzzClient,
    args: AskArgs,
    poll: bool,
) -> Result<EventBuilder, CliError> {
    if let Some(input) = args.json {
        let raw = if input == "-" {
            let mut raw = String::new();
            std::io::stdin()
                .take(65_537)
                .read_to_string(&mut raw)
                .map_err(usage)?;
            raw
        } else if let Some(path) = input.strip_prefix('@') {
            let file = std::fs::File::open(path).map_err(usage)?;
            let mut raw = String::new();
            file.take(65_537).read_to_string(&mut raw).map_err(usage)?;
            raw
        } else {
            input
        };
        if raw.len() > 65_536 {
            return Err(usage("prompt JSON exceeds 64 KiB"));
        }
        #[derive(serde::Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Input {
            content: String,
            tags: Vec<Vec<String>>,
        }
        let input: Input = serde_json::from_str(&raw).map_err(usage)?;
        let tags = input
            .tags
            .into_iter()
            .map(parse_tag)
            .collect::<Result<Vec<_>, _>>()?;
        return Ok(
            EventBuilder::new(Kind::Custom(KIND_INTERACTION_PROMPT as u16), input.content)
                .tags(tags),
        );
    }
    let channel = args.channel.ok_or_else(|| usage("--channel is required"))?;
    let channel = uuid::Uuid::parse_str(&channel).map_err(usage)?;
    let text = args.text.ok_or_else(|| usage("--text is required"))?;
    let itype = if poll { "poll" } else { &args.itype };
    let expires = Timestamp::now().as_secs() + duration(&args.expires)?.as_secs();
    let mut tags = vec![
        pair("h", channel)?,
        pair("itype", itype)?,
        pair("responders", args.responders)?,
        pair("visibility", "public")?,
        pair(
            "closes",
            args.closes
                .as_deref()
                .unwrap_or(if poll { "expiry" } else { "first" }),
        )?,
        pair("deadline", expires)?,
    ];
    for option in args.options {
        let parts: Vec<_> = option.splitn(3, ':').map(str::to_owned).collect();
        if parts.len() < 2 {
            return Err(usage("--option requires id:label[:style]"));
        }
        tags.push(parse_tag(
            std::iter::once("opt".into()).chain(parts).collect(),
        )?);
    }
    for field in args.fields {
        let parts: Vec<_> = field.split(':').map(str::to_owned).collect();
        if parts.len() != 4 {
            return Err(usage("--field requires id:label:type:required|optional"));
        }
        tags.push(parse_tag(
            std::iter::once("field".into()).chain(parts).collect(),
        )?);
    }
    for select in args.selects {
        let (field, options) = select
            .split_once('=')
            .ok_or_else(|| usage("--select requires field=value1,value2"))?;
        for option in options.split(',') {
            tags.push(parse_tag(vec![
                "optsel".into(),
                field.into(),
                option.into(),
            ])?);
        }
    }
    for pk in args.listed {
        tags.push(pair("p", PublicKey::parse(&pk).map_err(usage)?.to_hex())?);
    }
    if let Some(min) = args.min {
        tags.push(pair("min", min)?);
    }
    if let Some(max) = args.max {
        tags.push(pair("max", max)?);
    }
    if let Some(parent) = args.reply_to {
        let id = EventId::from_hex(&parent).map_err(usage)?;
        let raw = client.query(&json!({"kinds":[KIND_STREAM_MESSAGE,KIND_STREAM_MESSAGE_V2,KIND_FORUM_POST,KIND_FORUM_COMMENT,KIND_INTERACTION_PROMPT],"ids":[id.to_hex()],"#h":[channel.to_string()],"limit":1})).await?;
        let events: Vec<Event> = serde_json::from_str(&raw).map_err(usage)?;
        let parent = events
            .into_iter()
            .find(|e| e.id == id)
            .ok_or_else(|| CliError::NotFound("reply parent not found in channel".into()))?;
        let root = buzz_core::nip10::parse_thread_markers(&parent.tags)
            .resolve()
            .map(|(root, _)| root)
            .unwrap_or_else(|| id.to_hex());
        if root != id.to_hex() {
            tags.push(parse_tag(vec!["e".into(), root, "".into(), "root".into()])?);
        }
        tags.push(parse_tag(vec![
            "e".into(),
            id.to_hex(),
            "".into(),
            "reply".into(),
        ])?);
    }
    Ok(EventBuilder::new(Kind::Custom(KIND_INTERACTION_PROMPT as u16), text).tags(tags))
}

/// Execute an interaction command using the shared authenticated HTTP bridge.
pub async fn dispatch(cmd: InteractionsCmd, client: &BuzzClient) -> Result<(), CliError> {
    let relay = capability(client).await?;
    let cmd = match cmd {
        InteractionsCmd::Poll(mut args) => {
            args.itype = "poll".into();
            InteractionsCmd::Ask(args)
        }
        other => other,
    };
    match cmd {
        InteractionsCmd::Ask(args) | InteractionsCmd::Poll(args) => {
            let poll = args.itype == "poll";
            let event = client.sign_event(ask_builder(client, args, poll).await?)?;
            let schema = Prompt::parse(&event).map_err(usage)?;
            if poll && schema.itype != "poll" {
                return Err(usage("poll JSON must declare itype=poll"));
            }
            schema
                .validate_lifetime(Timestamp::now().as_secs())
                .map_err(usage)?;
            let result = client.submit_event(event).await?;
            println!("{}", normalize_write_response(&result));
        }
        InteractionsCmd::Answer {
            prompt,
            choices,
            values,
            comment,
        } => {
            let (event, schema) = read_prompt(client, &prompt).await?;
            let mut tags = vec![
                pair("h", schema.channel)?,
                parse_tag(vec![
                    "e".into(),
                    event.id.to_hex(),
                    "".into(),
                    "prompt".into(),
                ])?,
            ];
            for choice in choices {
                tags.push(pair("choice", choice)?);
            }
            for value in values {
                let (id, value) = value
                    .split_once('=')
                    .ok_or_else(|| usage("--value requires field=value"))?;
                tags.push(parse_tag(vec!["value".into(), id.into(), value.into()])?);
            }
            let mut builder =
                EventBuilder::new(Kind::Custom(KIND_INTERACTION_RESPONSE as u16), comment)
                    .tags(tags);
            // Ensure a user's deliberate edit in the same second beats their last
            // accepted answer. Avoids gambling on the NIP-01 lower-ID tie break.
            if let Some(state) = read_state(client, &event, &schema, relay).await? {
                let summary: Value = serde_json::from_str(&state.content).map_err(usage)?;
                if summary["status"] == "closed" {
                    return Err(usage("prompt is closed"));
                }
                let me = client.keys().public_key().to_hex();
                let previous = summary["responders"]
                    .as_array()
                    .and_then(|responders| responders.iter().find(|r| r["pubkey"] == me))
                    .and_then(|r| r["created_at"].as_u64());
                if let Some(previous) = previous {
                    builder = builder.custom_created_at(Timestamp::from(
                        Timestamp::now().as_secs().max(previous.saturating_add(1)),
                    ));
                }
            }
            let response = client.sign_event(builder)?;
            schema.answer(&response).map_err(usage)?;
            println!(
                "{}",
                normalize_write_response(&client.submit_event(response).await?)
            );
        }
        InteractionsCmd::Get { prompt } => {
            let (event, schema) = read_prompt(client, &prompt).await?;
            let state = read_state(client, &event, &schema, relay).await?;
            let mut events = vec![event];
            if let Some(state) = state {
                events.push(state);
            }
            println!("{}", serde_json::to_string(&events).map_err(usage)?);
        }
        InteractionsCmd::Wait { prompt, timeout } => {
            let timeout = duration(&timeout)?;
            let (event, schema) = read_prompt(client, &prompt).await?;
            let wait = async {
                loop {
                    if let Some(state) = read_state(client, &event, &schema, relay).await? {
                        let summary: Value = serde_json::from_str(&state.content).map_err(usage)?;
                        if summary["status"] == "closed" {
                            println!("{}", serde_json::to_string(&[state]).map_err(usage)?);
                            return Ok::<(), CliError>(());
                        }
                    }
                    tokio::time::sleep(Duration::from_secs(1)).await;
                }
            };
            tokio::time::timeout(timeout, wait).await.map_err(|_| {
                CliError::Other(
                    "timed out waiting for interaction close; no decision inferred".into(),
                )
            })??;
        }
        InteractionsCmd::Close { prompt } => {
            let (event, schema) = read_prompt(client, &prompt).await?;
            let builder =
                EventBuilder::new(Kind::Custom(KIND_INTERACTION_CLOSE as u16), "").tags([
                    pair("h", schema.channel)?,
                    parse_tag(vec![
                        "e".into(),
                        event.id.to_hex(),
                        "".into(),
                        "prompt".into(),
                    ])?,
                ]);
            println!(
                "{}",
                normalize_write_response(&client.submit_event(client.sign_event(builder)?).await?)
            );
        }
    }
    Ok(())
}

#[cfg(test)]
#[path = "interactions_http_tests.rs"]
mod http_tests;

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn durations_are_bounded_and_do_not_overflow() {
        for value in ["0", "0s", "-1h", "31d", "18446744073709551615d", "x"] {
            assert!(duration(value).is_err(), "{value}");
        }
        assert_eq!(duration("24h").unwrap().as_secs(), 86400);
        assert_eq!(duration("30d").unwrap().as_secs(), MAX_LIFETIME);
    }
    #[test]
    fn capability_requires_both_the_extension_and_relay_identity() {
        let key = nostr::Keys::generate().public_key();
        assert!(relay_identity(&json!({"self":key.to_hex()})).is_err());
        assert!(relay_identity(&json!({"supported_extensions":[EXTENSION]})).is_err());
        assert_eq!(
            relay_identity(&json!({"supported_extensions":[EXTENSION],"self":key.to_hex()}))
                .unwrap(),
            key
        );
    }
}
