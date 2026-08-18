use buzz_sdk::kind::{
    KIND_BOOKMARK_LIST, KIND_BOOKMARK_SET, KIND_FOLLOW_SET, KIND_MUTE_LIST,
    KIND_NIP65_RELAY_LIST_METADATA, KIND_PIN_LIST,
};
use nostr::{EventBuilder, Kind, Tag};
use serde::Deserialize;

use crate::client::{normalize_write_response, BuzzClient};
use crate::error::CliError;
use crate::validate::{parse_event_id, validate_hex64};

/// A single contact entry (CLI-local, not from buzz-sdk).
#[derive(Debug, Deserialize)]
pub struct ContactEntry {
    pub pubkey: String,
    #[serde(default)]
    pub relay_url: Option<String>,
    #[serde(default)]
    pub petname: Option<String>,
}

pub async fn cmd_publish_note(
    client: &BuzzClient,
    content: &str,
    reply_to: Option<&str>,
) -> Result<(), CliError> {
    if let Some(r) = reply_to {
        validate_hex64(r)?;
    }

    let reply_id = reply_to.map(parse_event_id).transpose()?;

    let builder = buzz_sdk::build_note(content, reply_id)
        .map_err(|e| CliError::Other(format!("build error: {e}")))?;

    let event = client.sign_event(builder)?;

    let resp = client.submit_event(event).await?;
    println!("{}", normalize_write_response(&resp));
    Ok(())
}

pub async fn cmd_set_contact_list(
    client: &BuzzClient,
    contacts_json: &str,
) -> Result<(), CliError> {
    let entries: Vec<ContactEntry> = serde_json::from_str(contacts_json)
        .map_err(|e| CliError::Usage(format!("invalid contacts JSON: {e}")))?;

    let contacts: Vec<(&str, Option<&str>, Option<&str>)> = entries
        .iter()
        .map(|c| {
            (
                c.pubkey.as_str(),
                c.relay_url.as_deref(),
                c.petname.as_deref(),
            )
        })
        .collect();

    let builder = buzz_sdk::build_contact_list(&contacts)
        .map_err(|e| CliError::Other(format!("build error: {e}")))?;

    let event = client.sign_event(builder)?;

    let resp = client.submit_event(event).await?;
    println!("{}", normalize_write_response(&resp));
    Ok(())
}

/// Get a single event by ID via POST /query.
pub async fn cmd_get_event(client: &BuzzClient, event_id: &str) -> Result<(), CliError> {
    validate_hex64(event_id)?;
    let filter = serde_json::json!({
        "ids": [event_id]
    });
    let resp = client.query(&filter).await?;
    println!("{resp}");
    Ok(())
}

/// Get user notes (kind:1) by author pubkey.
pub async fn cmd_get_user_notes(
    client: &BuzzClient,
    pubkey: &str,
    limit: Option<u32>,
    before: Option<i64>,
    before_id: Option<&str>,
) -> Result<(), CliError> {
    validate_hex64(pubkey)?;
    if let Some(bid) = before_id {
        validate_hex64(bid)?;
    }
    let limit = limit.unwrap_or(50).min(100);

    let mut filter = serde_json::json!({
        "kinds": [1],
        "authors": [pubkey],
        "limit": limit
    });

    if let Some(b) = before {
        filter["until"] = serde_json::json!(b);
    }
    if let Some(bid) = before_id {
        filter["before_id"] = serde_json::json!(bid);
    }

    let resp = client.query(&filter).await?;
    println!("{resp}");
    Ok(())
}

/// Get a user's contact list (kind:3) by pubkey.
pub async fn cmd_get_contact_list(client: &BuzzClient, pubkey: &str) -> Result<(), CliError> {
    validate_hex64(pubkey)?;
    let filter = serde_json::json!({
        "kinds": [3],
        "authors": [pubkey],
        "limit": 1
    });
    let resp = client.query(&filter).await?;
    println!("{resp}");
    Ok(())
}

fn validate_social_list_kind(kind: u32) -> Result<(), CliError> {
    match kind {
        KIND_MUTE_LIST
        | KIND_PIN_LIST
        | KIND_NIP65_RELAY_LIST_METADATA
        | KIND_BOOKMARK_LIST
        | KIND_FOLLOW_SET
        | KIND_BOOKMARK_SET => Ok(()),
        _ => Err(CliError::Usage(format!(
            "unsupported social list kind {kind}; supported kinds: 10000, 10001, 10002, 10003, 30000, 30003"
        ))),
    }
}

fn is_parameterized_social_list_kind(kind: u32) -> bool {
    matches!(kind, KIND_FOLLOW_SET | KIND_BOOKMARK_SET)
}

fn parse_tags_json(tags_json: &str) -> Result<Vec<Tag>, CliError> {
    let raw_tags: Vec<Vec<String>> = serde_json::from_str(tags_json)
        .map_err(|e| CliError::Usage(format!("invalid tags JSON: {e}")))?;
    raw_tags
        .iter()
        .map(|parts| {
            Tag::parse(parts.iter().map(String::as_str))
                .map_err(|e| CliError::Usage(format!("invalid tag {parts:?}: {e}")))
        })
        .collect::<Result<_, _>>()
}

fn has_d_tag(tags: &[Tag]) -> bool {
    tags.iter()
        .any(|t| t.as_slice().first().map(|s| s.as_str()) == Some("d"))
}

pub async fn cmd_set_list(
    client: &BuzzClient,
    kind: u16,
    tags_json: &str,
    content: &str,
) -> Result<(), CliError> {
    let kind_u32 = u32::from(kind);
    validate_social_list_kind(kind_u32)?;
    let tags = parse_tags_json(tags_json)?;
    if is_parameterized_social_list_kind(kind_u32) && !has_d_tag(&tags) {
        return Err(CliError::Usage(format!(
            "kind {kind} is parameterized replaceable and requires a d tag"
        )));
    }

    let builder = EventBuilder::new(Kind::Custom(kind), content).tags(tags);
    let event = client.sign_event(builder)?;
    let resp = client.submit_event(event).await?;
    println!("{resp}");
    Ok(())
}

pub async fn cmd_get_list(
    client: &BuzzClient,
    pubkey: &str,
    kind: u32,
    d_tag: Option<&str>,
) -> Result<(), CliError> {
    validate_hex64(pubkey)?;
    validate_social_list_kind(kind)?;
    if !is_parameterized_social_list_kind(kind) && d_tag.is_some() {
        return Err(CliError::Usage(format!(
            "kind {kind} is not parameterized; omit --d-tag"
        )));
    }

    let mut filter = serde_json::json!({
        "kinds": [kind],
        "authors": [pubkey],
        "limit": 10
    });
    if let Some(d) = d_tag {
        filter["#d"] = serde_json::json!([d]);
    }
    let resp = client.query(&filter).await?;
    println!("{resp}");
    Ok(())
}

pub async fn dispatch(cmd: crate::SocialCmd, client: &BuzzClient) -> Result<(), CliError> {
    use crate::SocialCmd;
    match cmd {
        SocialCmd::PublishNote { content, reply_to } => {
            cmd_publish_note(client, &content, reply_to.as_deref()).await
        }
        SocialCmd::SetContactList { contacts } => cmd_set_contact_list(client, &contacts).await,
        SocialCmd::GetEvent { event } => cmd_get_event(client, &event).await,
        SocialCmd::GetUserNotes {
            pubkey,
            limit,
            before,
            before_id,
        } => cmd_get_user_notes(client, &pubkey, limit, before, before_id.as_deref()).await,
        SocialCmd::GetContactList { pubkey } => cmd_get_contact_list(client, &pubkey).await,
        SocialCmd::SetList {
            kind,
            tags,
            content,
        } => cmd_set_list(client, kind, &tags, &content).await,
        SocialCmd::GetList {
            pubkey,
            kind,
            d_tag,
        } => cmd_get_list(client, &pubkey, kind, d_tag.as_deref()).await,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn social_list_kind_validation_accepts_supported_kinds() {
        for kind in [
            KIND_MUTE_LIST,
            KIND_PIN_LIST,
            KIND_NIP65_RELAY_LIST_METADATA,
            KIND_BOOKMARK_LIST,
            KIND_FOLLOW_SET,
            KIND_BOOKMARK_SET,
        ] {
            assert!(validate_social_list_kind(kind).is_ok(), "kind {kind}");
        }
    }

    #[test]
    fn social_list_kind_validation_rejects_unsupported_kinds() {
        let err = validate_social_list_kind(30002).unwrap_err();
        assert!(
            matches!(err, CliError::Usage(msg) if msg.contains("unsupported social list kind 30002"))
        );
    }

    #[test]
    fn parses_tags_json_and_detects_d_tag() {
        let tags = parse_tags_json(r#"[["d","friends"],["p","aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"]]"#)
            .expect("tags parse");
        assert!(has_d_tag(&tags));
    }

    #[test]
    fn malformed_tags_json_is_usage_error() {
        let err = parse_tags_json("not json").unwrap_err();
        assert!(matches!(err, CliError::Usage(msg) if msg.contains("invalid tags JSON")));
    }

    #[test]
    fn parameterized_social_list_kind_detection() {
        assert!(is_parameterized_social_list_kind(KIND_FOLLOW_SET));
        assert!(is_parameterized_social_list_kind(KIND_BOOKMARK_SET));
        assert!(!is_parameterized_social_list_kind(KIND_MUTE_LIST));
    }
}
