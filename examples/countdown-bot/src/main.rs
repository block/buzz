//! A tiny non-AI Buzz bot.
//!
//! The bot listens to one channel and replies to messages that contain commands:
//! - `!countdown 5` → `5 4 3 2 1 🚀`
//! - `!fib 8` → `13 8 5 3 2 1 1 0`
//! - `@Countdown Bot fib 8` → `13 8 5 3 2 1 1 0`
//!
//! It supports two relay-auth paths:
//! - `standalone`: authenticate as the bot key directly. This key must be an
//!   explicit relay member / allowlisted identity on closed relays.
//! - `owner-attested`: authenticate as the bot key with a NIP-OA `auth` tag
//!   signed by the owner/agent key. On relays that allow NIP-OA membership,
//!   the bot can connect because its owner is already a relay member.

use std::time::Duration;

use anyhow::{anyhow, bail, Context, Result};
use futures_util::{SinkExt, StreamExt};
use nostr::{
    Alphabet, Event, EventBuilder, Filter, JsonUtil, Keys, Kind, RelayUrl, SingleLetterTag, Tag,
};
use serde_json::{json, Value};
use tokio_tungstenite::{connect_async, tungstenite::Message};
use url::Url as WsUrl;

const DEFAULT_RELAY_URL: &str = "ws://localhost:3000";
const SUBSCRIPTION_ID: &str = "countdown-bot";
const BOT_NAME: &str = "countdown-bot";
const BOT_DISPLAY_NAME: &str = "Countdown Bot";
const BOT_ABOUT: &str =
    "A tiny non-AI Buzz reference bot that replies to !countdown and countdown-style !fib.";
const BOT_ICON_DATA_URL: &str = "data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 128 128'%3E%3Crect width='128' height='128' rx='28' fill='%23131622'/%3E%3Ccircle cx='64' cy='64' r='42' fill='none' stroke='%237dd3fc' stroke-width='10'/%3E%3Cpath d='M64 32v32l22 14' fill='none' stroke='%23facc15' stroke-width='10' stroke-linecap='round' stroke-linejoin='round'/%3E%3Cpath d='M42 96h44' stroke='%23a78bfa' stroke-width='8' stroke-linecap='round'/%3E%3C/svg%3E";

#[tokio::main]
async fn main() -> Result<()> {
    let config = Config::from_env()?;

    eprintln!(
        "countdown-bot pubkey: {}",
        config.bot_keys.public_key().to_hex()
    );
    eprintln!("connecting to {}", config.relay_url);

    let mut ws = connect_and_authenticate(&config).await?;
    publish_profile(&mut ws, &config).await?;
    announce_channel_membership(&mut ws, &config).await?;
    subscribe_to_channel(&mut ws, &config.channel_id).await?;

    let started_at = nostr::Timestamp::now();

    eprintln!(
        "listening in channel {} for !countdown, !fib, and @mention commands",
        config.channel_id
    );

    loop {
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {
                eprintln!("shutting down");
                return Ok(());
            }
            next = ws.next() => {
                let Some(message) = next else { bail!("relay closed the WebSocket"); };
                match message? {
                    Message::Text(text) => handle_relay_text(&mut ws, &config, started_at, &text).await?,
                    Message::Ping(bytes) => ws.send(Message::Pong(bytes)).await?,
                    Message::Close(frame) => bail!("relay closed connection: {frame:?}"),
                    _ => {}
                }
            }
        }
    }
}

struct Config {
    relay_url: String,
    channel_id: String,
    bot_keys: Keys,
    owner_auth_tag: Option<Tag>,
}

impl Config {
    fn from_env() -> Result<Self> {
        let relay_url =
            std::env::var("BUZZ_RELAY_URL").unwrap_or_else(|_| DEFAULT_RELAY_URL.to_string());
        let channel_id = required_env("BUZZ_CHANNEL_ID")?;
        let bot_keys = Keys::parse(&required_env("BUZZ_BOT_PRIVATE_KEY")?)
            .context("BUZZ_BOT_PRIVATE_KEY must be an nsec or hex private key")?;

        let auth_mode =
            std::env::var("BUZZ_BOT_AUTH_MODE").unwrap_or_else(|_| "standalone".to_string());
        let owner_auth_tag = match auth_mode.as_str() {
            "standalone" => None,
            "owner-attested" => {
                let tag_json = match std::env::var("BUZZ_AUTH_TAG") {
                    Ok(value) if !value.trim().is_empty() => value,
                    _ => {
                        let owner_keys = Keys::parse(&required_env("BUZZ_OWNER_PRIVATE_KEY")?)
                            .context("BUZZ_OWNER_PRIVATE_KEY must be an nsec or hex private key")?;
                        buzz_sdk::nip_oa::compute_auth_tag(&owner_keys, &bot_keys.public_key(), "")?
                    }
                };

                let owner = buzz_sdk::nip_oa::verify_auth_tag(&tag_json, &bot_keys.public_key())
                    .context("BUZZ_AUTH_TAG is not valid for BUZZ_BOT_PRIVATE_KEY")?;
                eprintln!("owner-attested auth tag verified; owner={}", owner.to_hex());
                Some(buzz_sdk::nip_oa::parse_auth_tag(&tag_json)?)
            }
            other => {
                bail!("BUZZ_BOT_AUTH_MODE must be 'standalone' or 'owner-attested', got {other:?}")
            }
        };

        Ok(Self {
            relay_url,
            channel_id,
            bot_keys,
            owner_auth_tag,
        })
    }
}

type Ws =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

async fn connect_and_authenticate(config: &Config) -> Result<Ws> {
    let parsed = WsUrl::parse(&config.relay_url)?;
    let (mut ws, _) = connect_async(parsed.as_str()).await?;

    let challenge = wait_for_auth_challenge(&mut ws).await?;
    let auth_event = build_auth_event(config, &challenge)?;
    let auth_event_id = auth_event.id.to_hex();

    send_json(&mut ws, json!(["AUTH", auth_event])).await?;
    wait_for_ok(&mut ws, &auth_event_id).await?;
    Ok(ws)
}

fn build_auth_event(config: &Config, challenge: &str) -> Result<Event> {
    let relay_url: RelayUrl = RelayUrl::parse(&config.relay_url)?;
    if let Some(auth_tag) = &config.owner_auth_tag {
        let tags = vec![
            Tag::parse(["relay", config.relay_url.as_str()])?,
            Tag::parse(["challenge", challenge])?,
            auth_tag.clone(),
        ];
        Ok(EventBuilder::new(Kind::Authentication, "")
            .tags(tags)
            .sign_with_keys(&config.bot_keys)?)
    } else {
        Ok(EventBuilder::auth(challenge, relay_url).sign_with_keys(&config.bot_keys)?)
    }
}

async fn publish_profile(ws: &mut Ws, config: &Config) -> Result<()> {
    let builder = buzz_sdk::builders::build_profile(
        Some(BOT_DISPLAY_NAME),
        Some(BOT_NAME),
        Some(BOT_ICON_DATA_URL),
        Some(BOT_ABOUT),
        None,
    )?;
    let profile_event = builder.sign_with_keys(&config.bot_keys)?;
    let profile_event_id = profile_event.id.to_hex();

    send_json(ws, json!(["EVENT", profile_event])).await?;
    wait_for_ok(ws, &profile_event_id).await?;
    eprintln!("published kind:0 profile for {BOT_DISPLAY_NAME}");
    Ok(())
}

async fn announce_channel_membership(ws: &mut Ws, config: &Config) -> Result<()> {
    let builder = EventBuilder::new(Kind::Custom(9000), "").tags([
        Tag::parse(["h", config.channel_id.as_str()])?,
        Tag::parse(["p", &config.bot_keys.public_key().to_hex()])?,
        Tag::parse(["role", "bot"])?,
    ]);
    let event = builder.sign_with_keys(&config.bot_keys)?;
    let event_id = event.id.to_hex();

    send_json(ws, json!(["EVENT", event])).await?;
    match wait_for_ok(ws, &event_id).await {
        Ok(()) => eprintln!("announced {BOT_DISPLAY_NAME} as a channel bot member"),
        Err(err) => eprintln!(
            "could not self-add {BOT_DISPLAY_NAME} as a channel bot member: {err}. For private channels, add the bot pubkey as a channel member/admin-invited bot."
        ),
    }
    Ok(())
}

async fn subscribe_to_channel(ws: &mut Ws, channel_id: &str) -> Result<()> {
    let filter = Filter::new().kind(Kind::Custom(9)).custom_tag(
        SingleLetterTag::lowercase(Alphabet::H),
        channel_id.to_string(),
    );
    send_json(ws, json!(["REQ", SUBSCRIPTION_ID, filter])).await
}

async fn handle_relay_text(
    ws: &mut Ws,
    config: &Config,
    started_at: nostr::Timestamp,
    text: &str,
) -> Result<()> {
    let value: Value = serde_json::from_str(text)?;
    match value.get(0).and_then(Value::as_str) {
        Some("EVENT") => {
            let event_value = value
                .get(2)
                .ok_or_else(|| anyhow!("EVENT message missing event payload"))?;
            let event = Event::from_json(event_value.to_string())?;
            maybe_reply(ws, config, started_at, &event).await?;
        }
        Some("EOSE") => {}
        Some("NOTICE") | Some("CLOSED") => eprintln!("relay: {value}"),
        Some(other) => eprintln!("ignored relay message type: {other}"),
        None => eprintln!("ignored malformed relay message: {text}"),
    }
    Ok(())
}

async fn maybe_reply(
    ws: &mut Ws,
    config: &Config,
    started_at: nostr::Timestamp,
    event: &Event,
) -> Result<()> {
    if event.pubkey == config.bot_keys.public_key() || event.created_at < started_at {
        return Ok(());
    }

    let Some(reply) = event_reply(config, event) else {
        return Ok(());
    };

    let builder = buzz_sdk::builders::build_message(
        config.channel_id.parse()?,
        &reply,
        None,
        &[&event.pubkey.to_hex()],
        false,
        &[],
    )?;
    let reply_event = builder.sign_with_keys(&config.bot_keys)?;
    let reply_event_id = reply_event.id.to_hex();

    send_json(ws, json!(["EVENT", reply_event])).await?;
    eprintln!("replied to {} with {}", event.id.to_hex(), reply_event_id);
    Ok(())
}

fn event_reply(config: &Config, event: &Event) -> Option<String> {
    command_reply(&event.content).or_else(|| {
        event_mentions_bot(event, config).then(|| mention_command_reply(&event.content))?
    })
}

fn command_reply(content: &str) -> Option<String> {
    let mut parts = content.split_whitespace();
    let command = parts.next()?;
    let n = parts.next()?;

    match command {
        "!countdown" => Some(countdown_reply(n)),
        "!fib" => Some(fib_reply(n)),
        _ => None,
    }
}

fn mention_command_reply(content: &str) -> Option<String> {
    let tokens = content.split_whitespace().collect::<Vec<_>>();
    tokens.windows(2).find_map(|window| match window {
        ["countdown", n] => Some(countdown_reply(n)),
        ["fib", n] => Some(fib_reply(n)),
        _ => None,
    })
}

fn event_mentions_bot(event: &Event, config: &Config) -> bool {
    let bot_pubkey = config.bot_keys.public_key().to_hex();
    event.tags.iter().any(|tag| {
        let parts = tag.as_slice();
        parts.first().map(String::as_str) == Some("p")
            && parts.get(1).map(String::as_str) == Some(bot_pubkey.as_str())
    })
}

fn countdown_reply(n: &str) -> String {
    match parse_bounded(n, 1, 100) {
        Ok(n) => (1..=n)
            .rev()
            .map(|i| i.to_string())
            .chain(["🚀".to_string()])
            .collect::<Vec<_>>()
            .join(" "),
        Err(message) => message,
    }
}

fn fib_reply(n: &str) -> String {
    match parse_bounded(n, 1, 100) {
        Ok(n) => fibonacci_countdown(n)
            .into_iter()
            .map(|i| i.to_string())
            .collect::<Vec<_>>()
            .join(" "),
        Err(message) => message,
    }
}

fn parse_bounded(s: &str, min: usize, max: usize) -> Result<usize, String> {
    let Ok(n) = s.parse::<usize>() else {
        return Err(format!("Please use a number from {min} to {max}."));
    };
    if (min..=max).contains(&n) {
        Ok(n)
    } else {
        Err(format!("Please use a number from {min} to {max}."))
    }
}

fn fibonacci_countdown(count: usize) -> Vec<u128> {
    let mut values = Vec::with_capacity(count);
    let (mut a, mut b) = (0, 1);
    for _ in 0..count {
        values.push(a);
        (a, b) = (b, a + b);
    }
    values.reverse();
    values
}

async fn wait_for_ok(ws: &mut Ws, event_id: &str) -> Result<()> {
    loop {
        let text = next_text(ws, Duration::from_secs(5)).await?;
        let value: Value = serde_json::from_str(&text)?;
        if value.get(0).and_then(Value::as_str) != Some("OK") {
            continue;
        }
        if value.get(1).and_then(Value::as_str) != Some(event_id) {
            continue;
        }
        if value.get(2).and_then(Value::as_bool) == Some(true) {
            return Ok(());
        }
        let reason = value
            .get(3)
            .and_then(Value::as_str)
            .unwrap_or("unknown reason");
        bail!("relay rejected event {event_id}: {reason}");
    }
}

async fn wait_for_auth_challenge(ws: &mut Ws) -> Result<String> {
    loop {
        let text = next_text(ws, Duration::from_secs(5)).await?;
        let value: Value = serde_json::from_str(&text)?;
        if value.get(0).and_then(Value::as_str) == Some("AUTH") {
            return value
                .get(1)
                .and_then(Value::as_str)
                .map(str::to_string)
                .ok_or_else(|| anyhow!("AUTH message missing challenge"));
        }
    }
}

async fn next_text(ws: &mut Ws, timeout: Duration) -> Result<String> {
    loop {
        let message = tokio::time::timeout(timeout, ws.next())
            .await
            .context("timed out waiting for relay message")?
            .ok_or_else(|| anyhow!("relay closed the WebSocket"))??;
        match message {
            Message::Text(text) => return Ok(text.to_string()),
            Message::Ping(bytes) => ws.send(Message::Pong(bytes)).await?,
            Message::Close(frame) => bail!("relay closed connection: {frame:?}"),
            _ => {}
        }
    }
}

async fn send_json(ws: &mut Ws, value: Value) -> Result<()> {
    ws.send(Message::Text(value.to_string().into())).await?;
    Ok(())
}

fn required_env(name: &str) -> Result<String> {
    std::env::var(name).with_context(|| format!("{name} is required"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn countdown_command_is_algorithmic_and_bounded() {
        assert_eq!(
            command_reply("!countdown 5").as_deref(),
            Some("5 4 3 2 1 🚀")
        );
        assert_eq!(
            command_reply("!countdown 0").as_deref(),
            Some("Please use a number from 1 to 100.")
        );
        assert_eq!(
            command_reply("!countdown 101").as_deref(),
            Some("Please use a number from 1 to 100.")
        );
    }

    #[test]
    fn fibonacci_command_counts_down_and_is_bounded() {
        assert_eq!(command_reply("!fib 5").as_deref(), Some("3 2 1 1 0"));
        assert_eq!(command_reply("!fib 8").as_deref(), Some("13 8 5 3 2 1 1 0"));
        assert_eq!(
            command_reply("!fib 101").as_deref(),
            Some("Please use a number from 1 to 100.")
        );
    }

    #[test]
    fn mention_commands_are_algorithmic_and_bounded() {
        assert_eq!(
            mention_command_reply("@Countdown Bot countdown 5").as_deref(),
            Some("5 4 3 2 1 🚀")
        );
        assert_eq!(
            mention_command_reply("@Countdown Bot fib 5").as_deref(),
            Some("3 2 1 1 0")
        );
        assert_eq!(
            mention_command_reply("@Countdown Bot fib 101").as_deref(),
            Some("Please use a number from 1 to 100.")
        );
    }
}
