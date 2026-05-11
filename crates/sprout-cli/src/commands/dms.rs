use nostr::{EventBuilder, Kind, Tag};

use crate::client::SproutClient;
use crate::error::CliError;
use crate::validate::{validate_hex64, validate_uuid};

/// List DM conversations by querying kind:41010 (DM open) events authored by us.
pub async fn cmd_list_dms(client: &SproutClient, limit: Option<u32>) -> Result<(), CliError> {
    let my_pk = client.keys().public_key().to_hex();
    let limit = limit.unwrap_or(50).min(200);
    let filter = serde_json::json!({
        "kinds": [41010],
        "authors": [my_pk],
        "limit": limit
    });
    let resp = client.query(&filter).await?;
    println!("{resp}");
    Ok(())
}

/// Open a DM with one or more users — sign and submit a kind:41010 event.
pub async fn cmd_open_dm(client: &SproutClient, pubkeys: &[String]) -> Result<(), CliError> {
    if pubkeys.is_empty() || pubkeys.len() > 8 {
        return Err(CliError::Usage("--pubkey: must provide 1–8 pubkeys".into()));
    }
    for pk in pubkeys {
        validate_hex64(pk)?;
    }

    let mut tags: Vec<Tag> = Vec::new();
    for pk in pubkeys {
        tags.push(Tag::parse(&["p", pk]).map_err(|e| CliError::Other(format!("tag error: {e}")))?);
    }

    let builder = EventBuilder::new(Kind::Custom(41010), "", tags);
    let event = client.sign_event(builder)?;

    let resp = client.submit_event(event).await?;
    println!("{resp}");
    Ok(())
}

/// Add a member to a DM group — sign and submit a kind:41011 event.
pub async fn cmd_add_dm_member(
    client: &SproutClient,
    channel_id: &str,
    pubkey: &str,
) -> Result<(), CliError> {
    validate_uuid(channel_id)?;
    validate_hex64(pubkey)?;

    let tags = vec![
        Tag::parse(&["h", channel_id]).map_err(|e| CliError::Other(format!("tag error: {e}")))?,
        Tag::parse(&["p", pubkey]).map_err(|e| CliError::Other(format!("tag error: {e}")))?,
    ];

    let builder = EventBuilder::new(Kind::Custom(41011), "", tags);
    let event = client.sign_event(builder)?;

    let resp = client.submit_event(event).await?;
    println!("{resp}");
    Ok(())
}
