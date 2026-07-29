//! Runs a local synchronization command whenever a trusted rendezvous node
//! witnesses a journal transition through a Beacon pulse (kind 20700).
//!
//! The watcher deliberately treats the pulse as a wake-up hint rather than
//! replication data. The configured command still performs the authorized,
//! cursor-safe drain; this process only replaces fixed-interval polling with
//! a signed transition signal.

use std::path::PathBuf;
use std::process::Stdio;
use std::time::Duration;

use anyhow::{bail, Context};
use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine;
use buzz_core::kind::KIND_BEACON_PULSE;
use buzz_core::verification::verify_event;
use buzz_ws_client::{NostrWsConnection, RelayMessage, WsClientError};
use nostr::hashes::sha256::Hash as Sha256Hash;
use nostr::hashes::Hash;
use nostr::nips::nip98::{HttpData, HttpMethod};
use nostr::types::Url as NostrUrl;
use nostr::{Event, EventBuilder, Keys, Kind, PublicKey, SecretKey, Tag};
use serde::Deserialize;
use serde_json::{json, Value};
use tokio::process::Command;
use tokio::time::{sleep, timeout};
use url::Url;
use uuid::Uuid;

const SUBSCRIPTION_ID: &str = "buzz-drain-pulse";
const DEFAULT_COMMAND_TIMEOUT_SECS: u64 = 240;
const IDLE_RECONNECT_SECS: u64 = 900;
const INITIAL_RECONNECT_SECS: u64 = 1;
const MAX_RECONNECT_SECS: u64 = 60;

#[derive(Debug)]
struct Config {
    relay: String,
    local: String,
    journal: PathBuf,
    key_file: PathBuf,
    witness: PublicKey,
    command: PathBuf,
    command_timeout: Duration,
}

impl Config {
    fn from_args() -> anyhow::Result<Self> {
        let mut relay = None;
        let mut local = None;
        let mut journal = None;
        let mut key_file = None;
        let mut witness = None;
        let mut command = None;
        let mut command_timeout = Duration::from_secs(DEFAULT_COMMAND_TIMEOUT_SECS);
        let mut args = std::env::args().skip(1);
        while let Some(arg) = args.next() {
            match arg.as_str() {
                "--relay" => relay = Some(next(&mut args, "--relay")?),
                "--local" => local = Some(next(&mut args, "--local")?),
                "--journal" => journal = Some(PathBuf::from(next(&mut args, "--journal")?)),
                "--key" => key_file = Some(PathBuf::from(next(&mut args, "--key")?)),
                "--witness" => {
                    let value = next(&mut args, "--witness")?;
                    witness = Some(
                        PublicKey::from_hex(&value)
                            .context("--witness requires a 32-byte hex public key")?,
                    )
                }
                "--command" => command = Some(PathBuf::from(next(&mut args, "--command")?)),
                "--command-timeout" => {
                    let seconds = next(&mut args, "--command-timeout")?
                        .parse::<u64>()
                        .context("--command-timeout requires a positive integer")?;
                    if seconds == 0 {
                        bail!("--command-timeout must be greater than zero");
                    }
                    command_timeout = Duration::from_secs(seconds);
                }
                "-h" | "--help" => {
                    print_help();
                    std::process::exit(0);
                }
                other => bail!("unknown argument: {other}"),
            }
        }
        Ok(Self {
            relay: websocket_url(&relay.context("--relay <URL> is required")?)?,
            local: local
                .context("--local <URL> is required")?
                .trim_end_matches('/')
                .to_string(),
            journal: journal.context("--journal <NDJSON path> is required")?,
            key_file: key_file.context("--key <nsec hex file> is required")?,
            witness: witness.context("--witness <hex pubkey> is required")?,
            command: command.context("--command <executable path> is required")?,
            command_timeout,
        })
    }
}

fn next(args: &mut impl Iterator<Item = String>, flag: &str) -> anyhow::Result<String> {
    args.next()
        .with_context(|| format!("{flag} requires a value"))
}

fn print_help() {
    println!(
        "buzz-pulse-watch — wake a drain command on trusted rendezvous transitions

Usage:
  buzz-pulse-watch --relay URL --local URL --journal PATH --key PATH
                   --witness HEX --command PATH [--command-timeout SECS]

Options:
  --relay URL             Rendezvous WebSocket or HTTP(S) base URL
  --local URL             Local sovereign relay HTTP base URL
  --journal PATH          Local NDJSON journal used for ancestry distance
  --key PATH              Secret key used for NIP-42 authentication
  --witness HEX           Pinned rendezvous Beacon witness public key
  --command PATH          Drain executable to run after a new trusted pulse
  --command-timeout SECS  Per-drain timeout (default: {DEFAULT_COMMAND_TIMEOUT_SECS})"
    );
}

fn websocket_url(raw: &str) -> anyhow::Result<String> {
    let mut url = Url::parse(raw).context("--relay is not a valid URL")?;
    let websocket_scheme = match url.scheme() {
        "http" => "ws",
        "https" => "wss",
        "ws" => "ws",
        "wss" => "wss",
        scheme => bail!("unsupported relay URL scheme: {scheme}"),
    };
    url.set_scheme(websocket_scheme)
        .map_err(|_| anyhow::anyhow!("could not normalize relay URL scheme"))?;
    if url.path().is_empty() {
        url.set_path("/");
    }
    Ok(url.to_string())
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct PulseCursor {
    sequence: u64,
    head: Option<String>,
}

#[derive(Debug, Deserialize)]
struct PulseContent {
    journal: PulseJournal,
}

#[derive(Debug, Deserialize)]
struct PulseJournal {
    sequence: u64,
    head: Option<String>,
}

fn trusted_pulse(event: &Event, witness: PublicKey) -> Option<PulseCursor> {
    if event.kind.as_u16() as u32 != KIND_BEACON_PULSE || event.pubkey != witness {
        return None;
    }
    if verify_event(event).is_err() {
        return None;
    }
    let role_is_rendezvous = event.tags.iter().any(|tag| {
        let values = tag.as_slice();
        values.first().map(String::as_str) == Some("role")
            && values.get(1).map(String::as_str) == Some("rendezvous")
    });
    if !role_is_rendezvous {
        return None;
    }
    let content: PulseContent = serde_json::from_str(&event.content).ok()?;
    if let Some(head) = content.journal.head.as_ref() {
        if head.len() != 64 || !head.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return None;
        }
    } else if content.journal.sequence != 0 {
        return None;
    }
    Some(PulseCursor {
        sequence: content.journal.sequence,
        head: content.journal.head,
    })
}

#[derive(Debug)]
struct LocalView {
    journal: PulseJournal,
    holds_remote_head: bool,
    records_since_remote_head: Option<u64>,
}

fn nip98_header(keys: &Keys, url: &str, body: &[u8]) -> anyhow::Result<String> {
    let http_data =
        HttpData::new(NostrUrl::parse(url)?, HttpMethod::POST).payload(Sha256Hash::hash(body));
    let nonce = Uuid::new_v4().to_string();
    let event = EventBuilder::http_auth(http_data)
        .tag(Tag::parse(["nonce", nonce.as_str()]).context("nonce tag parses")?)
        .sign_with_keys(keys)?;
    Ok(format!(
        "Nostr {}",
        BASE64.encode(serde_json::to_vec(&event)?)
    ))
}

fn records_since(path: &PathBuf, event_id: &str) -> anyhow::Result<Option<u64>> {
    let journal = std::fs::read_to_string(path)
        .with_context(|| format!("failed to read local journal {}", path.display()))?;
    let mut found_at = None;
    let mut records = 0u64;
    for line in journal.lines().filter(|line| !line.trim().is_empty()) {
        let value: Value = serde_json::from_str(line)
            .with_context(|| format!("local journal {} is malformed", path.display()))?;
        if value.get("id").and_then(Value::as_str) == Some(event_id) {
            found_at = Some(records);
        }
        records = records.saturating_add(1);
    }
    Ok(found_at.map(|position| records.saturating_sub(position).saturating_sub(1)))
}

async fn local_view(config: &Config, keys: &Keys, remote_head: &str) -> anyhow::Result<LocalView> {
    let query_url = format!("{}/query", config.local);
    let body = serde_json::to_vec(&json!([
        {"kinds": [KIND_BEACON_PULSE], "limit": 1},
        {"ids": [remote_head], "limit": 1}
    ]))?;
    let response = reqwest::Client::new()
        .post(&query_url)
        .header("authorization", nip98_header(keys, &query_url, &body)?)
        .header("content-type", "application/json")
        .body(body)
        .send()
        .await
        .context("local pulse query failed")?;
    if !response.status().is_success() {
        let status = response.status();
        let detail = response.text().await.unwrap_or_default();
        bail!("local relay denied pulse query: {status} {detail}");
    }
    let events: Vec<Event> = response
        .json()
        .await
        .context("local relay returned an invalid pulse query")?;
    let pulse = events
        .iter()
        .find(|event| event.kind.as_u16() as u32 == KIND_BEACON_PULSE)
        .context("local relay returned no Beacon pulse")?;
    verify_event(pulse).context("local relay pulse signature is invalid")?;
    let content: PulseContent =
        serde_json::from_str(&pulse.content).context("local relay pulse content is invalid")?;
    let holds_remote_head = events.iter().any(|event| event.id.to_hex() == remote_head);
    let records_since_remote_head = if holds_remote_head {
        records_since(&config.journal, remote_head)?
    } else {
        None
    };
    Ok(LocalView {
        journal: content.journal,
        holds_remote_head,
        records_since_remote_head,
    })
}

fn build_response(
    keys: &Keys,
    pulse: &Event,
    remote: &PulseCursor,
    local: &LocalView,
) -> anyhow::Result<Option<Event>> {
    let Some(remote_head) = remote.head.as_deref() else {
        return Ok(None);
    };
    let (stance, observed) = if local.journal.head.as_deref() == Some(remote_head) {
        ("recognize", json!({}))
    } else if local.holds_remote_head {
        let since = local
            .records_since_remote_head
            .context("local journal holds the rendezvous head but its position is unknown")?;
        ("advanced", json!({"since": since}))
    } else {
        (
            "diverged",
            json!({"measure": "head-unknown", "detail": "transport drain completed"}),
        )
    };
    let content = json!({
        "stance": stance,
        "head": remote_head,
        "mine": {
            "sequence": local.journal.sequence,
            "head": local.journal.head,
        },
        "observed": observed,
    });
    let response = EventBuilder::new(
        Kind::Custom(buzz_core::kind::KIND_BEACON_RESPONSE as u16),
        content.to_string(),
    )
    .tags(vec![
        Tag::parse(["e", pulse.id.to_hex().as_str()]).context("response e tag parses")?,
        Tag::parse(["p", pulse.pubkey.to_hex().as_str()]).context("response p tag parses")?,
        Tag::parse(["role", "participant"]).context("response role tag parses")?,
    ])
    .sign_with_keys(keys)?;
    Ok(Some(response))
}

async fn run_drain(config: &Config, pulse: &PulseCursor) -> anyhow::Result<()> {
    println!(
        "trusted rendezvous transition: sequence {}, head {}; draining",
        pulse.sequence,
        pulse.head.as_deref().unwrap_or("<empty>")
    );
    let mut child = Command::new(&config.command)
        .stdin(Stdio::null())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .kill_on_drop(true)
        .spawn()
        .with_context(|| format!("failed to start drain command {}", config.command.display()))?;
    let status = timeout(config.command_timeout, child.wait())
        .await
        .with_context(|| {
            format!(
                "drain command exceeded {} seconds",
                config.command_timeout.as_secs()
            )
        })??;
    if !status.success() {
        bail!("drain command exited with {status}");
    }
    Ok(())
}

async fn watch_session(
    config: &Config,
    keys: &Keys,
    last_drained: &mut Option<PulseCursor>,
    last_responded_pulse: &mut Option<String>,
) -> anyhow::Result<()> {
    let mut connection = NostrWsConnection::connect_authenticated(&config.relay, keys, None)
        .await
        .with_context(|| format!("could not authenticate to {}", config.relay))?;
    connection
        .send_raw(&json!([
            "REQ",
            SUBSCRIPTION_ID,
            {"kinds": [KIND_BEACON_PULSE]}
        ]))
        .await
        .context("could not subscribe to Beacon pulses")?;

    loop {
        let message = connection
            .next_event(Duration::from_secs(IDLE_RECONNECT_SECS))
            .await?;
        match message {
            RelayMessage::Event {
                subscription_id,
                event,
            } if subscription_id == SUBSCRIPTION_ID => {
                let Some(pulse) = trusted_pulse(&event, config.witness) else {
                    continue;
                };
                if last_drained.as_ref() != Some(&pulse) {
                    run_drain(config, &pulse).await?;
                    *last_drained = Some(pulse.clone());
                }
                let pulse_id = event.id.to_hex();
                if last_responded_pulse.as_ref() == Some(&pulse_id) {
                    continue;
                }
                let Some(remote_head) = pulse.head.as_deref() else {
                    *last_responded_pulse = Some(pulse_id);
                    continue;
                };
                match local_view(config, keys, remote_head).await {
                    Ok(local) => match build_response(keys, &event, &pulse, &local) {
                        Ok(Some(response)) => match connection.send_event(response).await {
                            Ok(accepted) if accepted.accepted => {
                                println!("Beacon response accepted: {}", accepted.message);
                                *last_responded_pulse = Some(pulse_id);
                            }
                            Ok(rejected) => {
                                eprintln!("Beacon response rejected: {}", rejected.message);
                            }
                            Err(error) => {
                                eprintln!("Beacon response publication failed: {error}");
                            }
                        },
                        Ok(None) => {
                            *last_responded_pulse = Some(pulse_id);
                        }
                        Err(error) => {
                            eprintln!("could not build Beacon response: {error:#}");
                        }
                    },
                    Err(error) => {
                        eprintln!("could not evaluate local Beacon stance: {error:#}");
                    }
                }
            }
            RelayMessage::Closed {
                subscription_id,
                message,
            } if subscription_id == SUBSCRIPTION_ID => {
                bail!("Beacon subscription closed: {message}");
            }
            RelayMessage::Notice { message } => {
                eprintln!("rendezvous notice: {message}");
            }
            _ => {}
        }
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let config = Config::from_args()?;
    let secret = std::fs::read_to_string(&config.key_file)
        .with_context(|| format!("failed to read key file {}", config.key_file.display()))?;
    let keys = Keys::new(SecretKey::from_hex(secret.trim()).context("invalid secret key")?);
    let mut last_drained = None;
    let mut last_responded_pulse = None;
    let mut reconnect_delay = Duration::from_secs(INITIAL_RECONNECT_SECS);

    loop {
        match watch_session(&config, &keys, &mut last_drained, &mut last_responded_pulse).await {
            Ok(()) => return Ok(()),
            Err(error) => {
                if matches!(
                    error.downcast_ref::<WsClientError>(),
                    Some(WsClientError::ConnectionClosed)
                ) {
                    eprintln!("rendezvous connection closed; reconnecting");
                } else {
                    eprintln!("pulse watch interrupted: {error:#}");
                }
            }
        }
        sleep(reconnect_delay).await;
        reconnect_delay = (reconnect_delay * 2).min(Duration::from_secs(MAX_RECONNECT_SECS));
    }
}

#[cfg(test)]
mod tests {
    use nostr::{EventBuilder, Kind, Tag};

    use super::*;

    fn pulse(keys: &Keys, sequence: u64, head: Option<&str>, role: &str) -> nostr::Event {
        EventBuilder::new(
            Kind::Custom(KIND_BEACON_PULSE as u16),
            json!({"journal": {"sequence": sequence, "head": head}}).to_string(),
        )
        .tag(Tag::parse(["role", role]).expect("role tag parses"))
        .sign_with_keys(keys)
        .expect("pulse signs")
    }

    #[test]
    fn normalizes_http_relay_urls_for_websockets() {
        assert_eq!(
            websocket_url("https://relay.example").expect("URL normalizes"),
            "wss://relay.example/"
        );
        assert_eq!(
            websocket_url("ws://127.0.0.1:7777/").expect("URL stays WS"),
            "ws://127.0.0.1:7777/"
        );
    }

    #[test]
    fn accepts_only_the_pinned_rendezvous_witness() {
        let witness = Keys::generate();
        let foreign = Keys::generate();
        let head = "ab".repeat(32);
        let valid = pulse(&witness, 12, Some(&head), "rendezvous");

        assert_eq!(
            trusted_pulse(&valid, witness.public_key()),
            Some(PulseCursor {
                sequence: 12,
                head: Some(head.clone()),
            })
        );
        assert_eq!(trusted_pulse(&valid, foreign.public_key()), None);
        assert_eq!(
            trusted_pulse(
                &pulse(&witness, 12, Some(&head), "sovereign"),
                witness.public_key()
            ),
            None
        );
    }

    #[test]
    fn rejects_incoherent_journal_shapes() {
        let witness = Keys::generate();
        assert_eq!(
            trusted_pulse(
                &pulse(&witness, 3, None, "rendezvous"),
                witness.public_key()
            ),
            None
        );
        assert_eq!(
            trusted_pulse(
                &pulse(&witness, 3, Some("not-an-event-id"), "rendezvous"),
                witness.public_key()
            ),
            None
        );
    }

    #[test]
    fn builds_a_pulse_bound_recognition_response() {
        let witness = Keys::generate();
        let responder = Keys::generate();
        let head = "ab".repeat(32);
        let pulse_event = pulse(&witness, 12, Some(&head), "rendezvous");
        let remote = trusted_pulse(&pulse_event, witness.public_key()).expect("pulse is trusted");
        let local = LocalView {
            journal: PulseJournal {
                sequence: 20,
                head: Some(head.clone()),
            },
            holds_remote_head: true,
            records_since_remote_head: Some(0),
        };

        let response = build_response(&responder, &pulse_event, &remote, &local)
            .expect("response builds")
            .expect("non-empty head is answerable");
        let content: Value = serde_json::from_str(&response.content).expect("content is JSON");
        assert_eq!(content["stance"], "recognize");
        assert_eq!(content["head"], head);
        assert_eq!(content["mine"]["sequence"], 20);
        assert!(response.tags.iter().any(|tag| {
            let values = tag.as_slice();
            values.first().map(String::as_str) == Some("e")
                && values.get(1).map(String::as_str) == Some(pulse_event.id.to_hex().as_str())
        }));
    }

    #[test]
    fn reports_honest_advanced_and_diverged_stances() {
        let witness = Keys::generate();
        let responder = Keys::generate();
        let remote_head = "ab".repeat(32);
        let local_head = "cd".repeat(32);
        let pulse_event = pulse(&witness, 12, Some(&remote_head), "rendezvous");
        let remote = trusted_pulse(&pulse_event, witness.public_key()).expect("pulse is trusted");
        let advanced = LocalView {
            journal: PulseJournal {
                sequence: 20,
                head: Some(local_head.clone()),
            },
            holds_remote_head: true,
            records_since_remote_head: Some(4),
        };
        let response = build_response(&responder, &pulse_event, &remote, &advanced)
            .expect("advanced response builds")
            .expect("head is answerable");
        let content: Value = serde_json::from_str(&response.content).expect("content is JSON");
        assert_eq!(content["stance"], "advanced");
        assert_eq!(content["observed"]["since"], 4);

        let diverged = LocalView {
            journal: PulseJournal {
                sequence: 20,
                head: Some(local_head),
            },
            holds_remote_head: false,
            records_since_remote_head: None,
        };
        let response = build_response(&responder, &pulse_event, &remote, &diverged)
            .expect("diverged response builds")
            .expect("head is answerable");
        let content: Value = serde_json::from_str(&response.content).expect("content is JSON");
        assert_eq!(content["stance"], "diverged");
        assert_eq!(content["observed"]["measure"], "head-unknown");
    }
}
