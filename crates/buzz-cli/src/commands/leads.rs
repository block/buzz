//! Agent tools for leads: the prospect records Discovery finds and people
//! reference in chat.
//!
//! Reads come from the relay-signed prospect heads (kind 30644) in the business
//! channel the caller can see. Writes are ordinary member-signed prospect
//! actions (kind 47034) that name the exact head they replace, so the relay
//! applies the same rules as for the desktop: `won` is only reachable through
//! an accepted proposal, and nothing here creates, archives or deletes a lead.

use std::collections::HashMap;

use buzz_core::business_records::{
    ProspectAction, ProspectActivityInput, ProspectActivityKind, ProspectHead,
    ProspectQualification, ProspectStage, RecordAction, BUSINESS_RECORD_SCHEMA_VERSION,
};
use buzz_core::kind::KIND_PROSPECT_HEAD;
use serde::Serialize;
use serde_json::{json, Value};
use uuid::Uuid;

use crate::client::{normalize_write_response, BuzzClient};
use crate::error::CliError;
use crate::links::{lead_link, parse_lead_reference};
use crate::validate::{read_or_stdin, sdk_err};

const LEAD_QUERY_BOUND: u32 = 10_000;
const DEFAULT_LIST_LIMIT: usize = 50;
const MAX_LIST_LIMIT: usize = 500;
/// The relay bounds a note at 2000 bytes and a lost reason at 1000 bytes.
const MAX_NOTE_BYTES: usize = 2_000;
const MAX_LOST_REASON_BYTES: usize = 1_000;

pub async fn dispatch(cmd: crate::LeadsCmd, client: &BuzzClient) -> Result<(), CliError> {
    use crate::LeadsCmd;
    match cmd {
        LeadsCmd::List {
            limit,
            stage,
            query,
        } => cmd_list(client, limit, stage.as_deref(), query.as_deref()).await,
        LeadsCmd::Get { lead } => cmd_get(client, &lead).await,
        LeadsCmd::Note {
            lead,
            content,
            kind,
        } => {
            let content = read_or_stdin(&content)?;
            let kind = parse_activity_kind(kind.as_deref().unwrap_or("note"))?;
            update_lead(client, &lead, LeadChange::Note { content, kind }).await
        }
        LeadsCmd::Stage {
            lead,
            stage,
            reason,
        } => {
            let stage = parse_stage(&stage)?;
            update_lead(client, &lead, LeadChange::Stage { stage, reason }).await
        }
        LeadsCmd::Qualify {
            lead,
            qualification,
        } => {
            let qualification = parse_qualification(&qualification)?;
            update_lead(client, &lead, LeadChange::Qualification(qualification)).await
        }
        LeadsCmd::Save { lead, unsave } => {
            update_lead(client, &lead, LeadChange::Saved(!unsave)).await
        }
    }
}

/// One edit an agent can make to a lead.
#[derive(Debug, Clone, PartialEq, Eq)]
enum LeadChange {
    Note {
        content: String,
        kind: ProspectActivityKind,
    },
    Stage {
        stage: ProspectStage,
        reason: Option<String>,
    },
    Qualification(ProspectQualification),
    Saved(bool),
}

/// A verified current prospect head and the coordinates needed to replace it.
#[derive(Debug, Clone)]
pub(crate) struct CurrentLead {
    /// Id of the head event; the next action names it as the head it replaces.
    pub(crate) event_id: String,
    pub(crate) channel_id: Uuid,
    pub(crate) community_id: Uuid,
    pub(crate) created_at: u64,
    pub(crate) head: ProspectHead,
}

fn parse_stage(value: &str) -> Result<ProspectStage, CliError> {
    match value.trim().replace('-', "_").as_str() {
        "qualified" => Ok(ProspectStage::Qualified),
        "in_conversation" => Ok(ProspectStage::InConversation),
        "proposal" => Ok(ProspectStage::Proposal),
        "lost" => Ok(ProspectStage::Lost),
        "won" => Err(CliError::Usage(
            "a lead becomes won only when a client accepts its proposal".into(),
        )),
        _ => Err(CliError::Usage(
            "stage must be qualified, in_conversation, proposal, or lost".into(),
        )),
    }
}

fn parse_qualification(value: &str) -> Result<ProspectQualification, CliError> {
    match value.trim().replace('-', "_").as_str() {
        "qualified" => Ok(ProspectQualification::Qualified),
        "not_fit" => Ok(ProspectQualification::NotFit),
        "unreviewed" => Ok(ProspectQualification::Unreviewed),
        _ => Err(CliError::Usage(
            "qualification must be qualified, not_fit, or unreviewed".into(),
        )),
    }
}

fn parse_activity_kind(value: &str) -> Result<ProspectActivityKind, CliError> {
    match value.trim() {
        "note" => Ok(ProspectActivityKind::Note),
        "contact" => Ok(ProspectActivityKind::Contact),
        _ => Err(CliError::Usage("kind must be note or contact".into())),
    }
}

/// The snake_case label a prospect enum serializes to.
fn label<T: Serialize>(value: &T) -> String {
    serde_json::to_value(value)
        .ok()
        .and_then(|value| value.as_str().map(str::to_owned))
        .unwrap_or_default()
}

/// Split a prospect coordinate `business:<community>:prospect:<prospect>`.
fn parse_prospect_d_tag(d_tag: &str) -> Option<(Uuid, Uuid)> {
    let mut parts = d_tag.split(':');
    if parts.next()? != "business" {
        return None;
    }
    let community_id = Uuid::parse_str(parts.next()?).ok()?;
    if parts.next()? != "prospect" {
        return None;
    }
    let prospect_id = Uuid::parse_str(parts.next()?).ok()?;
    if parts.next().is_some() {
        return None;
    }
    Some((community_id, prospect_id))
}

/// Check that a prospect head is relay-signed, well formed and consistent with
/// its coordinate, then keep what an update needs.
fn parse_head_event(event: &Value, relay_self: &str) -> Result<CurrentLead, CliError> {
    let signed: nostr::Event = serde_json::from_value(event.clone())
        .map_err(|error| CliError::Other(format!("lead head event is malformed: {error}")))?;
    if signed.kind != nostr::Kind::Custom(KIND_PROSPECT_HEAD as u16) {
        return Err(CliError::Other(format!(
            "lead head event has wrong kind: {}",
            signed.kind.as_u16()
        )));
    }
    if signed.pubkey.to_hex() != relay_self {
        return Err(CliError::Other(
            "lead head author does not match the relay self key".into(),
        ));
    }
    signed
        .verify()
        .map_err(|error| CliError::Other(format!("lead head signature is invalid: {error}")))?;

    let tag_values = |name: &str| -> Vec<String> {
        signed
            .tags
            .iter()
            .filter(|tag| tag.kind().to_string() == name)
            .filter_map(|tag| tag.content().map(str::to_owned))
            .collect()
    };
    let (d_tags, h_tags) = (tag_values("d"), tag_values("h"));
    let ([d_tag], [h_tag]) = (d_tags.as_slice(), h_tags.as_slice()) else {
        return Err(CliError::Other(
            "lead head must have exactly one d tag and one h tag".into(),
        ));
    };
    let (community_id, prospect_id) = parse_prospect_d_tag(d_tag)
        .ok_or_else(|| CliError::Other("lead head d-tag is not a prospect coordinate".into()))?;
    let channel_id = Uuid::parse_str(h_tag)
        .map_err(|_| CliError::Other("lead head h-tag is not a channel UUID".into()))?;
    let head: ProspectHead = serde_json::from_str(&signed.content)
        .map_err(|error| CliError::Other(format!("lead head content is invalid: {error}")))?;
    if head.prospect_id != prospect_id {
        return Err(CliError::Other(
            "lead head d-tag does not match its content".into(),
        ));
    }
    Ok(CurrentLead {
        event_id: signed.id.to_hex(),
        channel_id,
        community_id,
        created_at: signed.created_at.as_secs(),
        head,
    })
}

/// Keep only the newest head of each lead, newest lead first.
fn latest_per_lead(leads: Vec<CurrentLead>) -> Vec<CurrentLead> {
    let mut latest: HashMap<Uuid, CurrentLead> = HashMap::new();
    for lead in leads {
        let prospect_id = lead.head.prospect_id;
        let newer = latest.get(&prospect_id).is_none_or(|existing| {
            (lead.created_at, &lead.event_id) > (existing.created_at, &existing.event_id)
        });
        if newer {
            latest.insert(prospect_id, lead);
        }
    }
    let mut leads: Vec<CurrentLead> = latest.into_values().collect();
    leads.sort_by(|left, right| {
        right
            .created_at
            .cmp(&left.created_at)
            .then_with(|| {
                left.head
                    .prospect
                    .party
                    .display_name
                    .cmp(&right.head.prospect.party.display_name)
            })
            .then_with(|| left.head.prospect_id.cmp(&right.head.prospect_id))
    });
    leads
}

fn matches_query(lead: &CurrentLead, query: &str) -> bool {
    let prospect = &lead.head.prospect;
    [
        Some(prospect.party.display_name.as_str()),
        Some(prospect.industry.as_str()),
        Some(prospect.vertical.as_str()),
        prospect.contact_name.as_deref(),
        prospect.location.as_deref(),
        prospect.email.as_deref(),
        prospect.website.as_deref(),
    ]
    .into_iter()
    .flatten()
    .any(|field| field.to_lowercase().contains(query))
}

fn lead_summary(lead: &CurrentLead) -> Value {
    let prospect = &lead.head.prospect;
    json!({
        "leadId": lead.head.prospect_id.to_string(),
        "reference": lead_link(lead.head.prospect_id),
        "name": prospect.party.display_name,
        "stage": label(&prospect.stage),
        "qualification": label(&prospect.qualification),
        "saved": prospect.saved,
        "fitScore": prospect.fit_score,
        "industry": prospect.industry,
        "vertical": prospect.vertical,
        "location": prospect.location,
        "website": prospect.website,
        "email": prospect.email,
        "phone": prospect.phone,
        "contactName": prospect.contact_name,
        "potentialMonthlyValueMinor": prospect.potential_monthly_value_minor,
    })
}

fn lead_detail(lead: &CurrentLead) -> Value {
    let prospect = &lead.head.prospect;
    let mut detail = lead_summary(lead);
    if let Some(object) = detail.as_object_mut() {
        object.insert("status".into(), json!(lead.head.status));
        object.insert("partyType".into(), json!(prospect.party.party_type));
        object.insert("lostReason".into(), json!(prospect.lost_reason));
        object.insert("lastVerifiedAt".into(), json!(prospect.last_verified_at));
        object.insert("evidence".into(), json!(prospect.evidence));
        object.insert("activities".into(), json!(lead.head.activities));
        object.insert("headEventId".into(), json!(lead.event_id));
        object.insert("channelId".into(), json!(lead.channel_id.to_string()));
        object.insert("updatedAt".into(), json!(lead.created_at));
    }
    detail
}

/// Work out the prospect action that applies `change` to `head`, copying every
/// other field from the current head.
fn build_update(
    head: &ProspectHead,
    head_event_id: &str,
    change: LeadChange,
    activity_id: Uuid,
) -> Result<ProspectAction, CliError> {
    if head.status != "active" {
        return Err(CliError::Usage(
            "this lead is archived; restore it in Colony before changing it".into(),
        ));
    }
    let mut prospect = head.prospect.clone();
    let mut activity = None;
    match change {
        LeadChange::Note { content, kind } => {
            let content = content.trim().to_owned();
            if content.is_empty() || content.len() > MAX_NOTE_BYTES {
                return Err(CliError::Usage(format!(
                    "a note must be 1 through {MAX_NOTE_BYTES} bytes"
                )));
            }
            activity = Some(ProspectActivityInput {
                activity_id,
                activity_kind: kind,
                content,
                proposal_id: None,
                proposal_version_event_id: None,
            });
        }
        LeadChange::Stage { stage, reason } => {
            let reason = reason
                .map(|reason| reason.trim().to_owned())
                .filter(|reason| !reason.is_empty());
            if stage == ProspectStage::Lost {
                let reason = reason.ok_or_else(|| {
                    CliError::Usage("a lost lead needs --reason saying why".into())
                })?;
                if reason.len() > MAX_LOST_REASON_BYTES {
                    return Err(CliError::Usage(format!(
                        "the lost reason must be at most {MAX_LOST_REASON_BYTES} bytes"
                    )));
                }
                prospect.lost_reason = Some(reason);
            } else {
                if reason.is_some() {
                    return Err(CliError::Usage(
                        "--reason only applies when the stage is lost".into(),
                    ));
                }
                prospect.lost_reason = None;
            }
            prospect.stage = stage;
        }
        LeadChange::Qualification(qualification) => prospect.qualification = qualification,
        LeadChange::Saved(saved) => prospect.saved = saved,
    }
    Ok(ProspectAction {
        schema_version: BUSINESS_RECORD_SCHEMA_VERSION,
        prospect_id: head.prospect_id,
        action: RecordAction::Update,
        expected_head_event_id: Some(head_event_id.to_owned()),
        prospect,
        activity,
    })
}

fn normalize_relay_self_hex(self_hex: &str) -> Result<String, CliError> {
    if self_hex.len() != 64 || !self_hex.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(CliError::Other(
            "relay 'self' field is not a valid 64-hex pubkey".into(),
        ));
    }
    Ok(self_hex.to_ascii_lowercase())
}

pub(crate) async fn load_leads(client: &BuzzClient) -> Result<Vec<CurrentLead>, CliError> {
    let nip11_raw = client.get_public("/").await.map_err(|error| {
        CliError::Other(format!("failed to fetch relay info document: {error}"))
    })?;
    let nip11: Value = serde_json::from_str(&nip11_raw).map_err(|error| {
        CliError::Other(format!("relay info document is not valid JSON: {error}"))
    })?;
    let relay_self = nip11
        .get("self")
        .and_then(Value::as_str)
        .ok_or_else(|| CliError::Other("relay info document missing 'self' field".into()))?;
    let relay_self = normalize_relay_self_hex(relay_self)?;
    let events = client
        .query_all_bounded(
            json!({ "kinds": [KIND_PROSPECT_HEAD], "authors": [relay_self] }),
            LEAD_QUERY_BOUND,
        )
        .await?;
    let leads = events
        .iter()
        .map(|event| parse_head_event(event, &relay_self))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(latest_per_lead(leads))
}

fn find_lead(leads: Vec<CurrentLead>, lead_id: Uuid) -> Result<CurrentLead, CliError> {
    leads
        .into_iter()
        .find(|lead| lead.head.prospect_id == lead_id)
        .ok_or_else(|| CliError::NotFound(format!("lead {lead_id} not found")))
}

async fn cmd_list(
    client: &BuzzClient,
    limit: Option<u32>,
    stage: Option<&str>,
    query: Option<&str>,
) -> Result<(), CliError> {
    let stage = stage
        .map(|stage| {
            // `won` can be listed even though it cannot be set.
            if stage.trim() == "won" {
                Ok(ProspectStage::Won)
            } else {
                parse_stage(stage)
            }
        })
        .transpose()?;
    let query = query
        .map(|query| query.trim().to_lowercase())
        .filter(|query| !query.is_empty());
    let limit = limit.map_or(DEFAULT_LIST_LIMIT, |limit| limit as usize);
    if limit == 0 || limit > MAX_LIST_LIMIT {
        return Err(CliError::Usage(format!(
            "limit must be 1 through {MAX_LIST_LIMIT}"
        )));
    }
    let rows: Vec<Value> = load_leads(client)
        .await?
        .iter()
        .filter(|lead| lead.head.status == "active")
        .filter(|lead| stage.is_none_or(|stage| lead.head.prospect.stage == stage))
        .filter(|lead| {
            query
                .as_deref()
                .is_none_or(|query| matches_query(lead, query))
        })
        .take(limit)
        .map(lead_summary)
        .collect();
    println!("{}", Value::Array(rows));
    Ok(())
}

async fn cmd_get(client: &BuzzClient, lead: &str) -> Result<(), CliError> {
    let lead_id = parse_lead_reference(lead)?;
    let current = find_lead(load_leads(client).await?, lead_id)?;
    println!("{}", lead_detail(&current));
    Ok(())
}

async fn update_lead(client: &BuzzClient, lead: &str, change: LeadChange) -> Result<(), CliError> {
    let lead_id = parse_lead_reference(lead)?;
    let current = find_lead(load_leads(client).await?, lead_id)?;
    let action = build_update(&current.head, &current.event_id, change, Uuid::new_v4())?;
    let builder = buzz_sdk::business_records::build_prospect_action(
        current.community_id,
        current.channel_id,
        &action,
    )
    .map_err(sdk_err)?;
    let event = client.sign_event(builder)?;
    let response = client.submit_event(event).await?;
    println!("{}", write_output(&response, lead_id));
    Ok(())
}

/// The standard write response plus the lead's id and reference link.
fn write_output(response: &str, lead_id: Uuid) -> String {
    let normalized = normalize_write_response(response);
    let mut output = match serde_json::from_str::<Value>(&normalized) {
        Ok(Value::Object(object)) => object,
        _ => return normalized,
    };
    output.insert("lead_id".into(), json!(lead_id.to_string()));
    output.insert("reference".into(), json!(lead_link(lead_id)));
    Value::Object(output).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use buzz_core::business_records::{
        PartyRecord, ProspectActivity, ProspectEvidence, ProspectRecordInput,
    };

    const COMMUNITY: u128 = 1;
    const CHANNEL: u128 = 2;
    const LEAD: u128 = 3;

    fn head(stage: ProspectStage) -> ProspectHead {
        let prospect_id = Uuid::from_u128(LEAD);
        ProspectHead {
            schema_version: BUSINESS_RECORD_SCHEMA_VERSION,
            prospect_id,
            status: "active".into(),
            prospect: ProspectRecordInput {
                prospect_id,
                party: PartyRecord {
                    party_id: prospect_id,
                    party_type: "organization".into(),
                    display_name: "Form & Field".into(),
                    external_ids: vec!["outscraper:ChIJ-test".into()],
                },
                industry: "Home & Living".into(),
                vertical: "Home Decor & Gift Shops".into(),
                fit_score: Some(91),
                origin: Some(buzz_core::business_records::ProspectOrigin::Discovery),
                discovery_tags: Vec::new(),
                potential_monthly_value_minor: Some(650_000),
                website: Some("https://form-field.example".into()),
                contact_name: Some("Jules".into()),
                location: Some("Woodstock, Cape Town".into()),
                email: Some("hello@form-field.example".into()),
                phone: None,
                evidence: vec![ProspectEvidence {
                    title: "Google Maps listing".into(),
                    url: "https://maps.example/form-field".into(),
                    excerpt: "Design-led home and living store".into(),
                    observed_at: 1_800_000_000,
                }],
                last_verified_at: Some(1_800_000_000),
                qualification: ProspectQualification::Unreviewed,
                saved: false,
                stage,
                lost_reason: None,
            },
            activities: vec![ProspectActivity {
                activity_id: Uuid::from_u128(9),
                activity_kind: ProspectActivityKind::Note,
                content: "Found by Discovery".into(),
                proposal_id: None,
                proposal_version_event_id: None,
                author_pubkey: "ab".repeat(32),
                created_at: 1_800_000_001,
            }],
            source_action_event_id: "0".repeat(64),
        }
    }

    fn d_tag() -> String {
        format!(
            "business:{}:prospect:{}",
            Uuid::from_u128(COMMUNITY),
            Uuid::from_u128(LEAD)
        )
    }

    fn signed_head(
        keys: &nostr::Keys,
        head: &ProspectHead,
        tags: Vec<[String; 2]>,
        created_at: u64,
    ) -> Value {
        let tags: Vec<nostr::Tag> = tags
            .iter()
            .map(|[name, value]| nostr::Tag::parse([name.as_str(), value.as_str()]).expect("tag"))
            .collect();
        let event = nostr::EventBuilder::new(
            nostr::Kind::Custom(KIND_PROSPECT_HEAD as u16),
            serde_json::to_string(head).expect("serialize head"),
        )
        .tags(tags)
        .custom_created_at(nostr::Timestamp::from_secs(created_at))
        .sign_with_keys(keys)
        .expect("sign head");
        serde_json::to_value(event).expect("event json")
    }

    fn standard_tags() -> Vec<[String; 2]> {
        vec![
            ["h".into(), Uuid::from_u128(CHANNEL).to_string()],
            ["d".into(), d_tag()],
        ]
    }

    #[test]
    fn stage_accepts_only_settable_stages() {
        assert_eq!(parse_stage("qualified").unwrap(), ProspectStage::Qualified);
        assert_eq!(
            parse_stage("in_conversation").unwrap(),
            ProspectStage::InConversation
        );
        assert_eq!(
            parse_stage("In-Conversation".to_lowercase().as_str()).unwrap(),
            ProspectStage::InConversation
        );
        assert_eq!(parse_stage("proposal").unwrap(), ProspectStage::Proposal);
        assert_eq!(parse_stage("lost").unwrap(), ProspectStage::Lost);
        let won = parse_stage("won").unwrap_err().to_string();
        assert!(won.contains("accepts its proposal"), "{won}");
        assert!(parse_stage("archived").is_err());
        assert!(parse_stage("").is_err());
    }

    #[test]
    fn qualification_and_activity_kinds_are_explicit() {
        assert_eq!(
            parse_qualification("not_fit").unwrap(),
            ProspectQualification::NotFit
        );
        assert_eq!(
            parse_qualification("unreviewed").unwrap(),
            ProspectQualification::Unreviewed
        );
        assert!(parse_qualification("maybe").is_err());
        assert_eq!(
            parse_activity_kind("contact").unwrap(),
            ProspectActivityKind::Contact
        );
        assert!(parse_activity_kind("call").is_err());
    }

    #[test]
    fn prospect_coordinates_are_split_strictly() {
        assert_eq!(
            parse_prospect_d_tag(&d_tag()),
            Some((Uuid::from_u128(COMMUNITY), Uuid::from_u128(LEAD)))
        );
        let with_extra = format!("{}:extra", d_tag());
        let other_record = d_tag().replace(":prospect:", ":party:");
        for bad in [
            "",
            "business",
            "business:not-a-uuid:prospect:00000000-0000-0000-0000-000000000003",
            "company:goal:00000000-0000-0000-0000-000000000003",
            with_extra.as_str(),
            other_record.as_str(),
        ] {
            assert_eq!(parse_prospect_d_tag(bad), None, "{bad}");
        }
    }

    #[test]
    fn head_verification_requires_relay_author_signature_and_coordinates() {
        let relay_keys = nostr::Keys::generate();
        let other_keys = nostr::Keys::generate();
        let relay_hex = relay_keys.public_key().to_hex();
        let head = head(ProspectStage::Qualified);

        let valid = signed_head(&relay_keys, &head, standard_tags(), 1_800_000_100);
        let lead = parse_head_event(&valid, &relay_hex).expect("relay-signed head is valid");
        assert_eq!(lead.channel_id, Uuid::from_u128(CHANNEL));
        assert_eq!(lead.community_id, Uuid::from_u128(COMMUNITY));
        assert_eq!(lead.created_at, 1_800_000_100);
        assert_eq!(lead.head, head);
        assert_eq!(lead.event_id, valid["id"].as_str().unwrap());

        // Not authored by the relay.
        let foreign = signed_head(&other_keys, &head, standard_tags(), 1_800_000_100);
        assert!(parse_head_event(&foreign, &relay_hex).is_err());

        // A tampered body no longer matches its signature.
        let mut tampered = valid.clone();
        tampered["content"] = json!("{}");
        assert!(parse_head_event(&tampered, &relay_hex).is_err());

        // The coordinate must name the lead in the content.
        let wrong_lead = [
            ["h".to_owned(), Uuid::from_u128(CHANNEL).to_string()],
            [
                "d".to_owned(),
                format!(
                    "business:{}:prospect:{}",
                    Uuid::from_u128(COMMUNITY),
                    Uuid::from_u128(99)
                ),
            ],
        ]
        .to_vec();
        let mismatched = signed_head(&relay_keys, &head, wrong_lead, 1_800_000_100);
        assert!(parse_head_event(&mismatched, &relay_hex).is_err());

        // Both an h tag and exactly one d tag are required.
        let without_channel = vec![["d".to_owned(), d_tag()]];
        let missing_h = signed_head(&relay_keys, &head, without_channel, 1_800_000_100);
        assert!(parse_head_event(&missing_h, &relay_hex).is_err());
        let mut two_d = standard_tags();
        two_d.push(["d".to_owned(), d_tag()]);
        let repeated_d = signed_head(&relay_keys, &head, two_d, 1_800_000_100);
        assert!(parse_head_event(&repeated_d, &relay_hex).is_err());
    }

    #[test]
    fn the_newest_head_of_each_lead_wins() {
        let relay_keys = nostr::Keys::generate();
        let relay_hex = relay_keys.public_key().to_hex();
        let older = head(ProspectStage::Qualified);
        let newer = head(ProspectStage::InConversation);
        let parse = |head: &ProspectHead, at: u64| {
            parse_head_event(
                &signed_head(&relay_keys, head, standard_tags(), at),
                &relay_hex,
            )
            .expect("valid head")
        };

        let leads = latest_per_lead(vec![
            parse(&newer, 1_800_000_200),
            parse(&older, 1_800_000_100),
        ]);
        assert_eq!(leads.len(), 1);
        assert_eq!(leads[0].head.prospect.stage, ProspectStage::InConversation);

        let reversed = latest_per_lead(vec![
            parse(&older, 1_800_000_100),
            parse(&newer, 1_800_000_200),
        ]);
        assert_eq!(reversed.len(), 1);
        assert_eq!(
            reversed[0].head.prospect.stage,
            ProspectStage::InConversation
        );
    }

    #[test]
    fn leads_are_listed_newest_first() {
        let relay_keys = nostr::Keys::generate();
        let relay_hex = relay_keys.public_key().to_hex();
        let mut second = head(ProspectStage::Qualified);
        second.prospect_id = Uuid::from_u128(4);
        second.prospect.prospect_id = Uuid::from_u128(4);
        second.prospect.party.party_id = Uuid::from_u128(4);
        second.prospect.party.display_name = "The Olive House".into();
        let second_tags = vec![
            ["h".to_owned(), Uuid::from_u128(CHANNEL).to_string()],
            [
                "d".to_owned(),
                format!(
                    "business:{}:prospect:{}",
                    Uuid::from_u128(COMMUNITY),
                    Uuid::from_u128(4)
                ),
            ],
        ];
        let first = parse_head_event(
            &signed_head(
                &relay_keys,
                &head(ProspectStage::Qualified),
                standard_tags(),
                1_800_000_100,
            ),
            &relay_hex,
        )
        .expect("first");
        let latest = parse_head_event(
            &signed_head(&relay_keys, &second, second_tags, 1_800_000_900),
            &relay_hex,
        )
        .expect("second");

        let ordered = latest_per_lead(vec![first, latest]);
        let names: Vec<&str> = ordered
            .iter()
            .map(|lead| lead.head.prospect.party.display_name.as_str())
            .collect();
        assert_eq!(names, vec!["The Olive House", "Form & Field"]);
    }

    #[test]
    fn query_matches_names_places_and_contacts_case_insensitively() {
        let lead = CurrentLead {
            event_id: "e".repeat(64),
            channel_id: Uuid::from_u128(CHANNEL),
            community_id: Uuid::from_u128(COMMUNITY),
            created_at: 1,
            head: head(ProspectStage::Qualified),
        };
        for hit in [
            "form &",
            "woodstock",
            "jules",
            "home decor",
            "form-field.example",
        ] {
            assert!(matches_query(&lead, hit), "{hit}");
        }
        assert!(!matches_query(&lead, "olive"));
    }

    #[test]
    fn summaries_and_details_expose_the_lead_and_its_reference() {
        let lead = CurrentLead {
            event_id: "e".repeat(64),
            channel_id: Uuid::from_u128(CHANNEL),
            community_id: Uuid::from_u128(COMMUNITY),
            created_at: 1_800_000_050,
            head: head(ProspectStage::InConversation),
        };
        let summary = lead_summary(&lead);
        assert_eq!(summary["leadId"], Uuid::from_u128(LEAD).to_string());
        assert_eq!(
            summary["reference"],
            format!("buzz://lead/{}", Uuid::from_u128(LEAD))
        );
        assert_eq!(summary["name"], "Form & Field");
        assert_eq!(summary["stage"], "in_conversation");
        assert_eq!(summary["qualification"], "unreviewed");
        assert_eq!(summary["saved"], false);
        assert_eq!(summary["fitScore"], 91);
        assert_eq!(summary["potentialMonthlyValueMinor"], 650_000);
        assert!(summary["phone"].is_null());

        let detail = lead_detail(&lead);
        assert_eq!(detail["stage"], "in_conversation");
        assert_eq!(detail["status"], "active");
        assert_eq!(detail["headEventId"], "e".repeat(64));
        assert_eq!(detail["evidence"][0]["title"], "Google Maps listing");
        assert_eq!(detail["activities"][0]["content"], "Found by Discovery");
        assert_eq!(detail["activities"][0]["activityKind"], "note");
        assert!(detail["lostReason"].is_null());
    }

    #[test]
    fn a_note_appends_one_activity_and_changes_nothing_else() {
        let current = head(ProspectStage::Qualified);
        let activity_id = Uuid::from_u128(77);
        let action = build_update(
            &current,
            &"a".repeat(64),
            LeadChange::Note {
                content: "  Spoke to Jules, wants a quote.  ".into(),
                kind: ProspectActivityKind::Contact,
            },
            activity_id,
        )
        .expect("note action");

        assert_eq!(action.action, RecordAction::Update);
        assert_eq!(action.prospect_id, current.prospect_id);
        assert_eq!(action.expected_head_event_id, Some("a".repeat(64)));
        assert_eq!(action.prospect, current.prospect);
        let activity = action.activity.expect("activity");
        assert_eq!(activity.activity_id, activity_id);
        assert_eq!(activity.activity_kind, ProspectActivityKind::Contact);
        assert_eq!(activity.content, "Spoke to Jules, wants a quote.");
        assert_eq!(activity.proposal_id, None);
    }

    #[test]
    fn notes_are_bounded_like_the_relay_bounds_them() {
        let current = head(ProspectStage::Qualified);
        let build = |content: String| {
            build_update(
                &current,
                &"a".repeat(64),
                LeadChange::Note {
                    content,
                    kind: ProspectActivityKind::Note,
                },
                Uuid::from_u128(1),
            )
        };
        assert!(build("   ".into()).is_err());
        assert!(build("x".repeat(2_001)).is_err());
        assert!(build("x".repeat(2_000)).is_ok());
    }

    #[test]
    fn stage_changes_copy_the_rest_and_manage_the_lost_reason() {
        let current = head(ProspectStage::Qualified);
        let stage_change = |stage, reason: Option<&str>| {
            build_update(
                &current,
                &"a".repeat(64),
                LeadChange::Stage {
                    stage,
                    reason: reason.map(str::to_owned),
                },
                Uuid::from_u128(1),
            )
        };

        let moved = stage_change(ProspectStage::InConversation, None).expect("move");
        assert_eq!(moved.prospect.stage, ProspectStage::InConversation);
        assert_eq!(moved.prospect.lost_reason, None);
        assert_eq!(moved.prospect.party, current.prospect.party);
        assert_eq!(moved.prospect.fit_score, current.prospect.fit_score);
        assert!(moved.activity.is_none());

        let lost = stage_change(ProspectStage::Lost, Some(" Budget ")).expect("lost");
        assert_eq!(lost.prospect.stage, ProspectStage::Lost);
        assert_eq!(lost.prospect.lost_reason.as_deref(), Some("Budget"));

        assert!(stage_change(ProspectStage::Lost, None).is_err());
        assert!(stage_change(ProspectStage::Lost, Some("   ")).is_err());
        assert!(stage_change(ProspectStage::Lost, Some(&"x".repeat(1_001))).is_err());
        assert!(stage_change(ProspectStage::Proposal, Some("because")).is_err());

        // Moving a lost lead back clears the reason.
        let mut lost_head = head(ProspectStage::Lost);
        lost_head.prospect.lost_reason = Some("Budget".into());
        let reopened = build_update(
            &lost_head,
            &"a".repeat(64),
            LeadChange::Stage {
                stage: ProspectStage::Qualified,
                reason: None,
            },
            Uuid::from_u128(1),
        )
        .expect("reopen");
        assert_eq!(reopened.prospect.stage, ProspectStage::Qualified);
        assert_eq!(reopened.prospect.lost_reason, None);
    }

    #[test]
    fn qualification_and_saved_flags_change_only_themselves() {
        let current = head(ProspectStage::Qualified);
        let qualified = build_update(
            &current,
            &"a".repeat(64),
            LeadChange::Qualification(ProspectQualification::Qualified),
            Uuid::from_u128(1),
        )
        .expect("qualify");
        assert_eq!(
            qualified.prospect.qualification,
            ProspectQualification::Qualified
        );
        assert_eq!(qualified.prospect.stage, current.prospect.stage);
        assert_eq!(qualified.prospect.saved, current.prospect.saved);

        let saved = build_update(
            &current,
            &"a".repeat(64),
            LeadChange::Saved(true),
            Uuid::from_u128(1),
        )
        .expect("save");
        assert!(saved.prospect.saved);
        assert_eq!(saved.prospect.qualification, current.prospect.qualification);
    }

    #[test]
    fn an_archived_lead_cannot_be_changed() {
        let mut archived = head(ProspectStage::Qualified);
        archived.status = "archived".into();
        let error = build_update(
            &archived,
            &"a".repeat(64),
            LeadChange::Saved(true),
            Uuid::from_u128(1),
        )
        .unwrap_err()
        .to_string();
        assert!(error.contains("archived"), "{error}");
    }

    #[test]
    fn built_updates_pass_the_shared_command_validation() {
        use buzz_core::business_records::{
            parse_business_command, validate_business_command_scope, BusinessCommand,
        };

        let current = head(ProspectStage::Qualified);
        let action = build_update(
            &current,
            &"a".repeat(64),
            LeadChange::Note {
                content: "Left a voicemail".into(),
                kind: ProspectActivityKind::Contact,
            },
            Uuid::from_u128(5),
        )
        .expect("note action");
        let builder = buzz_sdk::business_records::build_prospect_action(
            Uuid::from_u128(COMMUNITY),
            Uuid::from_u128(CHANNEL),
            &action,
        )
        .expect("builder");
        let event = builder
            .sign_with_keys(&nostr::Keys::generate())
            .expect("signed event");
        let command = parse_business_command(buzz_core::kind::KIND_PROSPECT_ACTION, &event.content)
            .expect("the relay can parse the action");
        assert!(matches!(command, BusinessCommand::ProspectAction(_)));
        assert_eq!(
            validate_business_command_scope(
                Uuid::from_u128(COMMUNITY),
                Uuid::from_u128(CHANNEL),
                &d_tag(),
                &command,
            ),
            Ok(())
        );
    }

    #[test]
    fn write_output_adds_the_lead_to_the_standard_response() {
        let output = write_output(
            r#"{"event_id":"abc","accepted":true,"message":"ok"}"#,
            Uuid::from_u128(LEAD),
        );
        let parsed: Value = serde_json::from_str(&output).expect("json");
        assert_eq!(parsed["event_id"], "abc");
        assert_eq!(parsed["accepted"], true);
        assert_eq!(parsed["lead_id"], Uuid::from_u128(LEAD).to_string());
        assert_eq!(
            parsed["reference"],
            format!("buzz://lead/{}", Uuid::from_u128(LEAD))
        );
        // A response that is not JSON passes through untouched.
        assert_eq!(
            write_output("plain text", Uuid::from_u128(LEAD)),
            "plain text"
        );
    }

    #[test]
    fn relay_self_must_be_a_hex_pubkey() {
        assert!(normalize_relay_self_hex(&"A".repeat(64)).is_ok());
        assert_eq!(
            normalize_relay_self_hex(&"A".repeat(64)).unwrap(),
            "a".repeat(64)
        );
        assert!(normalize_relay_self_hex("short").is_err());
        assert!(normalize_relay_self_hex(&"g".repeat(64)).is_err());
    }
}
