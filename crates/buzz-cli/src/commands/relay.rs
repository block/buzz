//! `buzz relay` — NIP-43 community membership administration.
//!
//! Mutations are signed kinds 9030/9031 submitted through the generic event
//! bridge. The relay host identifies the community; there is no channel scope.

use buzz_core::kind::KIND_NIP43_MEMBERSHIP_LIST;
use nostr::PublicKey;

use crate::client::BuzzClient;
use crate::commands::parse_write_response;
use crate::error::CliError;
use crate::{OutputFormat, RelayCmd, RelayMemberRole};

fn resolve_pubkey(value: &str) -> Result<String, CliError> {
    let value = value.trim();
    PublicKey::parse(value)
        .map(|pubkey| pubkey.to_hex())
        .map_err(|error| {
            if value.to_ascii_lowercase().starts_with("npub1") {
                CliError::Usage(format!("invalid npub (check its Bech32 checksum): {error}"))
            } else {
                CliError::Usage(format!(
                    "invalid pubkey: pass a 64-character hex pubkey or a valid npub: {error}"
                ))
            }
        })
}

/// Return a relay-admin write response only when the relay accepted it.
///
/// NIP-43 commands are security-sensitive mutations; a `200` bridge response
/// with `accepted: false` must remain a non-zero CLI result.
fn accepted_write_response(response: &str) -> Result<String, CliError> {
    parse_write_response(response, "relay membership command was not accepted")
}

async fn cmd_add_member(
    client: &BuzzClient,
    pubkey: &str,
    role: RelayMemberRole,
) -> Result<(), CliError> {
    let pubkey = resolve_pubkey(pubkey)?;
    let builder = buzz_sdk::build_relay_add_member(&pubkey, role.as_wire())
        .map_err(crate::validate::sdk_err)?;
    let event = client.sign_event(builder)?;
    let response = client.submit_event(event).await?;
    println!("{}", accepted_write_response(&response)?);
    Ok(())
}

async fn cmd_remove_member(client: &BuzzClient, pubkey: &str) -> Result<(), CliError> {
    let pubkey = resolve_pubkey(pubkey)?;
    let builder = buzz_sdk::build_relay_remove_member(&pubkey).map_err(crate::validate::sdk_err)?;
    let event = client.sign_event(builder)?;
    let response = client.submit_event(event).await?;
    println!("{}", accepted_write_response(&response)?);
    Ok(())
}

async fn cmd_list_members(client: &BuzzClient, format: &OutputFormat) -> Result<(), CliError> {
    let filter = serde_json::json!({
        "kinds": [KIND_NIP43_MEMBERSHIP_LIST],
        "limit": 1,
    });
    let raw = client.query(&filter).await?;
    let events: Vec<serde_json::Value> = serde_json::from_str(&raw)
        .map_err(|error| CliError::Other(format!("invalid relay membership response: {error}")))?;
    let event = events.first().ok_or_else(|| {
        CliError::Other("relay has not published a membership snapshot yet".into())
    })?;
    let members = event
        .get("tags")
        .and_then(serde_json::Value::as_array)
        .map(|tags| {
            tags.iter()
                .filter_map(|tag| {
                    let tag = tag.as_array()?;
                    let [kind, pubkey, role, ..] = tag.as_slice() else {
                        return None;
                    };
                    (kind.as_str() == Some("member"))
                        .then(|| Some((pubkey.as_str()?, role.as_str()?)))
                        .flatten()
                        .map(|(pubkey, role)| serde_json::json!({"pubkey": pubkey, "role": role}))
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    match format {
        OutputFormat::Json => println!(
            "{}",
            serde_json::to_string(&members)
                .map_err(|error| CliError::Other(format!("serialize relay members: {error}")))?
        ),
        OutputFormat::Compact => {
            for member in members {
                let pubkey = member
                    .get("pubkey")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("");
                let role = member
                    .get("role")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("");
                println!("{pubkey}\t{role}");
            }
        }
    }
    Ok(())
}

pub async fn dispatch(
    command: RelayCmd,
    client: &BuzzClient,
    format: &OutputFormat,
) -> Result<(), CliError> {
    match command {
        RelayCmd::AddMember { pubkey, role } => cmd_add_member(client, &pubkey, role).await,
        RelayCmd::RemoveMember { pubkey } => cmd_remove_member(client, &pubkey).await,
        RelayCmd::ListMembers => cmd_list_members(client, format).await,
    }
}

#[cfg(test)]
mod tests {
    use super::{accepted_write_response, resolve_pubkey};
    use nostr::ToBech32;

    const PUBKEY: &str = "7f2a7d4223a977bbca6403fcf9d7c6393eb2730d03a2cc2b504143f1fb5b9670";

    #[test]
    fn resolves_hex_and_npub_to_lowercase_hex() {
        assert_eq!(resolve_pubkey(PUBKEY).unwrap(), PUBKEY);
        let npub = nostr::PublicKey::from_hex(PUBKEY)
            .unwrap()
            .to_bech32()
            .unwrap();
        assert_eq!(resolve_pubkey(&npub).unwrap(), PUBKEY);
    }

    #[test]
    fn rejects_a_corrupted_npub_with_checksum_guidance() {
        let error =
            resolve_pubkey("npub10u486s3r49mmhjnyq070n47xxyltyucdqw3v8x6sg9plr0m6jecr5yaqqqqqq")
                .unwrap_err()
                .to_string();
        assert!(error.contains("invalid npub"));
        assert!(error.contains("checksum"));
    }

    #[test]
    fn rejects_a_relay_admin_response_that_was_not_accepted() {
        let error =
            accepted_write_response(r#"{"accepted":false,"message":"actor not authorized"}"#)
                .unwrap_err()
                .to_string();
        assert!(error.contains("relay rejected event"));
        assert!(error.contains("actor not authorized"));
    }
}
