//! `buzz metrics publish` — encrypted NIP-AM agent snapshots.

use std::io::Read;

use buzz_core::agent_turn_metric::{encrypt_agent_turn_metric, AgentTurnMetricPayload};
use nostr::{EventBuilder, Kind, PublicKey, Tag};

use crate::client::BuzzClient;
use crate::error::CliError;
use crate::MetricsCmd;

fn read_payload_arg(value: &str) -> Result<String, CliError> {
    if value != "-" {
        return Ok(value.to_string());
    }
    let mut input = String::new();
    std::io::stdin()
        .read_to_string(&mut input)
        .map_err(|error| {
            CliError::Other(format!("failed to read metric payload from stdin: {error}"))
        })?;
    if input.trim().is_empty() {
        return Err(CliError::Usage("metric payload on stdin is empty".into()));
    }
    Ok(input)
}

pub async fn dispatch(cmd: MetricsCmd, client: &BuzzClient) -> Result<(), CliError> {
    match cmd {
        MetricsCmd::Publish { payload } => {
            let raw = read_payload_arg(&payload)?;
            let metric: AgentTurnMetricPayload = serde_json::from_str(&raw).map_err(|error| {
                CliError::Usage(format!("invalid NIP-AM payload JSON: {error}"))
            })?;
            metric
                .validate()
                .map_err(|error| CliError::Usage(format!("invalid NIP-AM payload: {error}")))?;

            let owner_hex = client.auth_tag_owner_hex().ok_or_else(|| {
                CliError::Auth(
                    "metrics publish requires BUZZ_AUTH_TAG so the owner encryption key is known"
                        .into(),
                )
            })?;
            let owner = PublicKey::from_hex(&owner_hex).map_err(|error| {
                CliError::Auth(format!("invalid owner pubkey in auth tag: {error}"))
            })?;
            let ciphertext = encrypt_agent_turn_metric(client.keys(), &owner, &metric)
                .map_err(|error| CliError::Other(format!("metric encryption failed: {error}")))?;
            let agent_hex = client.keys().public_key().to_hex();
            let event = client.sign_event(
                EventBuilder::new(
                    Kind::Custom(buzz_core::kind::KIND_AGENT_TURN_METRIC as u16),
                    ciphertext,
                )
                .tags([
                    Tag::parse(["p", &owner_hex])
                        .map_err(|error| CliError::Other(format!("invalid owner tag: {error}")))?,
                    Tag::parse(["agent", &agent_hex])
                        .map_err(|error| CliError::Other(format!("invalid agent tag: {error}")))?,
                ]),
            )?;
            let response = client.submit_event(event).await?;
            println!("{response}");
            Ok(())
        }
    }
}
