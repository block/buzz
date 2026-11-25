use crate::{
    client::BuzzClient, error::CliError, GovCmd, GovernanceDocumentCmd, RepoGovernanceArgs,
};
use buzz_core::governance::{command_builder, Document, DocumentId, ExpectedHead};
use serde::Deserialize;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct HeadResponse {
    governance_head: u32,
    community_id: uuid::Uuid,
    document: String,
    #[serde(deserialize_with = "required_head")]
    head: Option<nostr::Event>,
}

fn required_head<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<nostr::Event>, D::Error> {
    Option::<nostr::Event>::deserialize(deserializer)
}

fn parse_head(raw: &str, id: &DocumentId) -> Result<HeadResponse, CliError> {
    let response: HeadResponse = serde_json::from_str(raw).map_err(|_| {
        CliError::Other("relay does not support authoritative governance reads".into())
    })?;
    if response.governance_head != 1 || response.document != id.to_string() {
        return Err(CliError::Other("invalid governance head response".into()));
    }
    if let Some(head) = &response.head {
        head.verify()
            .map_err(|_| CliError::Other("invalid signed governance head".into()))?;
        if head.kind.as_u16() as u32 != buzz_core::kind::KIND_GOVERNANCE_STATE {
            return Err(CliError::Other("wrong governance state kind".into()));
        }
        for (name, value) in [
            ("d", id.to_string()),
            ("community", response.community_id.to_string()),
        ] {
            let tags: Vec<_> = head
                .tags
                .iter()
                .filter(|t| t.as_slice().first().is_some_and(|s| s == name))
                .collect();
            if tags.len() != 1 || tags[0].as_slice() != [name.to_string(), value] {
                return Err(CliError::Other("governance head binding mismatch".into()));
            }
        }
        Document::parse(&head.content, id).map_err(CliError::Other)?;
    }
    Ok(response)
}

async fn document(
    id: DocumentId,
    command: GovernanceDocumentCmd,
    client: &BuzzClient,
) -> Result<(), CliError> {
    // Validate the write locally before any network request. Never substitute
    // a fetched head for the explicit user's precondition.
    let write = match command {
        GovernanceDocumentCmd::Get => None,
        GovernanceDocumentCmd::Set { file, expected } => {
            let expected: ExpectedHead = expected.parse().map_err(CliError::Usage)?;
            let content = std::fs::read_to_string(&file)
                .map_err(|e| CliError::Usage(format!("cannot read {}: {e}", file.display())))?;
            Document::parse(&content, &id).map_err(CliError::Usage)?;
            Some((expected, content))
        }
    };
    let raw = client
        .query(&serde_json::json!({"governance_head": 1, "document": id.to_string()}))
        .await?;
    let response = parse_head(&raw, &id)?;
    if let Some((expected, content)) = write {
        let builder = command_builder(response.community_id, &id, &expected, &content)
            .map_err(CliError::Usage)?;
        let event = client.sign_event(builder).await?;
        let result = client.submit_event(event).await?;
        println!(
            "{}",
            super::parse_write_response(
                &result,
                "governance head changed; review the current document before retrying"
            )?
        );
    } else {
        println!("{raw}");
    }
    Ok(())
}

/// Move a repository's binding: a governed bind, over the current head.
/// The relay requires an admin of both the bound channel and `channel`.
/// Returns the relay's write response.
pub async fn bind(repo: &str, channel: &str, client: &BuzzClient) -> Result<String, CliError> {
    let id: DocumentId = format!("bind:{repo}").parse().map_err(CliError::Usage)?;
    let raw = client
        .query(&serde_json::json!({"governance_head": 1, "document": id.to_string()}))
        .await?;
    let response = parse_head(&raw, &id)?;
    let expected = response
        .head
        .map_or(ExpectedHead::Absent, |h| ExpectedHead::Event(h.id.to_hex()));
    let content =
        serde_json::json!({"version": 1, "body": {"type": "bind", "channel": channel}}).to_string();
    let builder = command_builder(response.community_id, &id, &expected, &content)
        .map_err(CliError::Usage)?;
    let event = client.sign_event(builder).await?;
    let result = client.submit_event(event).await?;
    super::parse_write_response(&result, "the binding changed concurrently; retry")
}

pub async fn dispatch(command: GovCmd, client: &BuzzClient) -> Result<(), CliError> {
    match command {
        GovCmd::Floor(cmd) => document(DocumentId::Floor, cmd, client).await,
        GovCmd::Trusted(cmd) => document(DocumentId::Trusted, cmd, client).await,
    }
}

pub async fn repo(
    kind: &str,
    args: RepoGovernanceArgs,
    client: &BuzzClient,
) -> Result<(), CliError> {
    document(
        format!("{kind}:{}", args.repo)
            .parse()
            .map_err(CliError::Usage)?,
        args.command,
        client,
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use nostr::{EventBuilder, Keys, Kind, Tag};

    fn response(document: &str, community: uuid::Uuid) -> String {
        let head = EventBuilder::new(Kind::Custom(39010), r#"{"version":1,"body":{"type":"floor","rules":[],"labels":{},"max_grant_seconds":3600}}"#)
            .tags([Tag::parse(["d", document]).unwrap(), Tag::parse(["community", &community.to_string()]).unwrap()])
            .sign_with_keys(&Keys::generate()).unwrap();
        serde_json::json!({"governance_head":1,"community_id":community,"document":"floor","head":head}).to_string()
    }

    #[test]
    fn ordinary_query_results_cannot_claim_absence() {
        for raw in [
            "[]",
            "null",
            "{}",
            r#"{"governance_head":2,"community_id":"00000000-0000-0000-0000-000000000001","document":"floor","head":null}"#,
        ] {
            assert!(parse_head(raw, &DocumentId::Floor).is_err());
        }
        let raw=serde_json::json!({"governance_head":1,"community_id":uuid::Uuid::new_v4(),"document":"floor","head":null}).to_string();
        assert!(parse_head(&raw, &DocumentId::Floor).unwrap().head.is_none());
        let mut missing: serde_json::Value = serde_json::from_str(&raw).unwrap();
        missing.as_object_mut().unwrap().remove("head");
        assert!(parse_head(&missing.to_string(), &DocumentId::Floor).is_err());
    }

    #[test]
    fn signed_head_must_match_requested_document_and_community() {
        let community = uuid::Uuid::new_v4();
        let raw = response("floor", community);
        assert!(parse_head(&raw, &DocumentId::Floor).is_ok());
        assert!(parse_head(&response("trusted", community), &DocumentId::Floor).is_err());
        let mut changed: serde_json::Value = serde_json::from_str(&raw).unwrap();
        changed["community_id"] = uuid::Uuid::new_v4().to_string().into();
        assert!(parse_head(&changed.to_string(), &DocumentId::Floor).is_err());
        let mut changed: serde_json::Value = serde_json::from_str(&raw).unwrap();
        changed["head"]["content"] = "{}".into();
        assert!(parse_head(&changed.to_string(), &DocumentId::Floor).is_err());
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ReceiptsResponse {
    git_receipts: u32,
    community_id: uuid::Uuid,
    repo: String,
    receipts: Vec<serde_json::Value>,
    #[serde(deserialize_with = "required_cursor")]
    next_before: Option<i64>,
}
fn required_cursor<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<i64>, D::Error> {
    Option::<i64>::deserialize(deserializer)
}
fn parse_receipts(raw: &str, repo: &str) -> Result<ReceiptsResponse, CliError> {
    let response: ReceiptsResponse = serde_json::from_str(raw).map_err(|_| {
        CliError::Other("relay does not support authoritative Git receipt reads".into())
    })?;
    if response.git_receipts != 1
        || response.repo != repo
        || response.next_before.is_some_and(|n| n <= 0)
    {
        return Err(CliError::Other("invalid Git receipt response".into()));
    }
    let mut previous = i64::MAX;
    for row in &response.receipts {
        let seq = row
            .get("seq")
            .and_then(serde_json::Value::as_i64)
            .ok_or_else(|| CliError::Other("missing receipt sequence".into()))?;
        if seq <= 0
            || seq >= previous
            || !matches!(
                row.get("action").and_then(serde_json::Value::as_str),
                Some("git_landed" | "git_refused")
            )
            || !row.get("detail").is_some_and(serde_json::Value::is_object)
        {
            return Err(CliError::Other("invalid Git receipt row".into()));
        }
        previous = seq;
    }
    if response
        .next_before
        .is_some_and(|cursor| response.receipts.is_empty() || cursor != previous)
    {
        return Err(CliError::Other("invalid Git receipt cursor".into()));
    }
    Ok(response)
}

pub async fn receipts(
    repo: String,
    before: Option<i64>,
    limit: u32,
    client: &BuzzClient,
) -> Result<(), CliError> {
    let repo: buzz_core::governance::RepoCoordinate = repo.parse().map_err(CliError::Usage)?;
    if !(1..=200).contains(&limit) || before.is_some_and(|n| n <= 0) {
        return Err(CliError::Usage(
            "limit must be 1..200 and before must be positive".into(),
        ));
    }
    let raw=client.query(&serde_json::json!({"git_receipts":1,"repo":repo.to_string(),"before":before,"limit":limit})).await?;
    let response = parse_receipts(&raw, &repo.to_string())?;
    let _community = response.community_id;
    println!("{raw}");
    Ok(())
}

#[cfg(test)]
mod receipt_tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn receipt_envelope_cannot_confuse_unsupported_with_empty() {
        let repo = format!("30617:{}:repo", "a".repeat(64));
        let valid = json!({"git_receipts":1,"community_id":uuid::Uuid::new_v4(),"repo":repo,"receipts":[],"next_before":null});
        assert!(parse_receipts(&valid.to_string(), &repo).is_ok());
        for field in [
            "git_receipts",
            "community_id",
            "repo",
            "receipts",
            "next_before",
        ] {
            let mut malformed = valid.clone();
            malformed.as_object_mut().unwrap().remove(field);
            assert!(
                parse_receipts(&malformed.to_string(), &repo).is_err(),
                "{field}"
            );
        }
        assert!(parse_receipts("[]", &repo).is_err());
        assert!(parse_receipts(&valid.to_string(), "different-repo").is_err());
    }

    #[test]
    fn receipts_require_ordered_decisions_and_exact_cursor() {
        let repo = format!("30617:{}:repo", "a".repeat(64));
        let valid = json!({"git_receipts":1,"community_id":uuid::Uuid::new_v4(),"repo":repo,"receipts":[{"seq":10,"action":"git_refused","detail":{"stage":"hook"}},{"seq":8,"action":"git_landed","detail":{}}],"next_before":8});
        assert!(parse_receipts(&valid.to_string(), &repo).is_ok());
        for (field, value) in [
            ("seq", json!(10)),
            ("action", json!("event_created")),
            ("detail", json!(null)),
        ] {
            let mut malformed = valid.clone();
            malformed["receipts"][1][field] = value;
            assert!(
                parse_receipts(&malformed.to_string(), &repo).is_err(),
                "{field}"
            );
        }
        let mut malformed = valid;
        malformed["next_before"] = json!(7);
        assert!(parse_receipts(&malformed.to_string(), &repo).is_err());
    }
}

fn validate_effective(raw: &str, repo: &str) -> Result<(), CliError> {
    let value: serde_json::Value = serde_json::from_str(raw)
        .map_err(|_| CliError::Other("relay does not support effective Git policy reads".into()))?;
    let invalid = || CliError::Other("invalid or incomplete effective Git policy response".into());
    if value.get("git_effective").and_then(|v| v.as_u64()) != Some(1)
        || value.get("repo").and_then(|v| v.as_str()) != Some(repo)
        || value.get("evaluation").and_then(|v| v.as_str()) != Some("legacy_and_governance")
        || value.get("scope").and_then(|v| v.as_str()) != Some("policy_inputs_not_push_verdict")
    {
        return Err(invalid());
    }
    for key in ["community_id", "channel_id"] {
        value
            .get(key)
            .and_then(|v| v.as_str())
            .and_then(|s| s.parse::<uuid::Uuid>().ok())
            .ok_or_else(invalid)?;
    }
    for (key, field, array) in [
        ("floor", "rules", true),
        ("trusted", "checks", false),
        ("labels", "values", false),
        ("rules", "rules", true),
    ] {
        let part = value.get(key).ok_or_else(invalid)?;
        let head = part.get("head").ok_or_else(invalid)?;
        if !head.is_null()
            && head
                .as_str()
                .and_then(|s| nostr::EventId::from_hex(s).ok())
                .is_none()
        {
            return Err(invalid());
        }
        let content = part.get(field).ok_or_else(invalid)?;
        if if array {
            !content.is_array()
        } else {
            !content.is_object()
        } {
            return Err(invalid());
        }
    }
    if !value["floor"]["label_schema"].is_object()
        || !matches!(
            value["labels"]["source"].as_str(),
            Some("repo" | "floor_defaults")
        )
        || value["legacy"]["source"].as_str() != Some(repo)
        || !value["legacy"]["rules"].is_array()
        || value["legacy"]["default_min_roles"]
            .as_array()
            .is_none_or(|v| v.len() != 3)
    {
        return Err(invalid());
    }
    for (row, namespace) in value["legacy"]["default_min_roles"]
        .as_array()
        .unwrap()
        .iter()
        .zip(["branches", "tags", "other"])
    {
        if row["namespace"].as_str() != Some(namespace) {
            return Err(invalid());
        }
        for kind in ["create", "fast_forward", "non_fast_forward", "delete"] {
            if !matches!(row[kind].as_str(), Some("member" | "admin")) {
                return Err(invalid());
            }
        }
    }
    Ok(())
}

pub async fn effective(repo: String, client: &BuzzClient) -> Result<(), CliError> {
    let repo: buzz_core::governance::RepoCoordinate = repo.parse().map_err(CliError::Usage)?;
    let raw = client
        .query(&serde_json::json!({"git_effective":1,"repo":repo.to_string()}))
        .await?;
    validate_effective(&raw, &repo.to_string())?;
    println!("{raw}");
    Ok(())
}

#[cfg(test)]
mod effective_tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn effective_requires_both_policy_sources_and_explicit_heads() {
        let repo = format!("30617:{}:repo", "a".repeat(64));
        let value = json!({"git_effective":1,"repo":repo,"community_id":uuid::Uuid::new_v4(),"channel_id":uuid::Uuid::new_v4(),"evaluation":"legacy_and_governance","scope":"policy_inputs_not_push_verdict",
            "floor":{"head":null,"rules":[],"label_schema":{}},"trusted":{"head":null,"checks":{}},"labels":{"head":null,"source":"floor_defaults","values":{}},"rules":{"head":null,"rules":[]},"legacy":{"source":repo,"rules":[],"default_min_roles":[{"namespace":"branches","create":"member","fast_forward":"member","non_fast_forward":"admin","delete":"admin"},{"namespace":"tags","create":"member","fast_forward":"admin","non_fast_forward":"admin","delete":"admin"},{"namespace":"other","create":"admin","fast_forward":"admin","non_fast_forward":"admin","delete":"admin"}]}});
        assert!(validate_effective(&value.to_string(), &repo).is_ok());
        for key in [
            "floor",
            "trusted",
            "labels",
            "rules",
            "legacy",
            "evaluation",
            "scope",
            "channel_id",
        ] {
            let mut changed = value.clone();
            changed.as_object_mut().unwrap().remove(key);
            assert!(
                validate_effective(&changed.to_string(), &repo).is_err(),
                "{key}"
            );
        }
        let mut changed = value;
        changed["floor"].as_object_mut().unwrap().remove("head");
        assert!(validate_effective(&changed.to_string(), &repo).is_err());
        assert!(validate_effective("[]", &repo).is_err());
    }
}
