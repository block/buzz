use std::collections::{BTreeMap, BTreeSet};

use chrono::{DateTime, Duration, SecondsFormat, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tauri::Manager;
use uuid::Uuid;

use super::canonical::CandidateSource;
use super::{AdviserId, BriefSection, SourceKind};
use crate::app_state::AppState;
use crate::commands::engrams::{read_agent_memory_listing, EngramEntry};
use crate::managed_agents::load_managed_agents;

pub(super) const COMMAND_TEAM_COLLECTION: &str = "command_team_discussions";
const OUTCOME_SCHEMA: &str = "command-discussion-outcome-v1";
const OUTCOME_PREFIX: &str = "mem/command-brief";
const MAX_TEXT_BYTES: usize = 4_096;
const MAX_ARRAY_ITEMS: usize = 64;
const MAX_OUTCOME_JSON_BYTES: usize = 32_768;
const MAX_PER_ADVISER: usize = 6;
const MAX_TEAM_OUTCOMES: usize = 24;

#[derive(Clone)]
pub(super) struct DiscussionMemoryRecord {
    pub(super) persona_id: String,
    pub(super) agent_pubkey: String,
    pub(super) entry: EngramEntry,
}

#[derive(Clone, Debug, Default)]
pub(crate) struct CommandTeamDiscussionBatch {
    pub(super) candidates: Vec<CandidateSource>,
    pub(super) limitations: Vec<String>,
}

impl CommandTeamDiscussionBatch {
    pub(crate) fn unavailable(limitation: &str) -> Self {
        Self {
            candidates: Vec::new(),
            limitations: vec![limitation.to_string()],
        }
    }
}

#[cfg(test)]
impl CommandTeamDiscussionBatch {
    pub(crate) fn for_test(candidate_count: usize, limitations: Vec<String>) -> Self {
        let candidates = (0..candidate_count)
            .map(|index| {
                let source_id = format!("{index:064x}");
                CandidateSource {
                    source_id: source_id.clone(),
                    source_kind: SourceKind::Memory,
                    collection: COMMAND_TEAM_COLLECTION.to_string(),
                    document_id: format!("discussion-{index}"),
                    chunk_id: source_id,
                    timestamp: "2026-07-27T02:00:00Z".to_string(),
                    location: format!(
                        "command adviser persona builtin:command-operations Buzz channel test-{index}"
                    ),
                    retrieved_at: "2026-07-27T02:01:00Z".to_string(),
                    observed_at: "2026-07-27T02:01:00Z".to_string(),
                    quote: format!("Validated command-team outcome {index}."),
                }
            })
            .collect();
        Self {
            candidates,
            limitations,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct CommandDiscussionOutcome {
    schema: String,
    outcome_id: String,
    adviser: AdviserId,
    recorded_at: DateTime<Utc>,
    origin: OutcomeOrigin,
    status: OutcomeStatus,
    summary: String,
    decisions: Vec<String>,
    actions: Vec<OutcomeAction>,
    risks: Vec<String>,
    assumptions: Vec<String>,
    unresolved_questions: Vec<String>,
    brief_sections: Vec<BriefSection>,
    review_at: Option<DateTime<Utc>>,
    supersedes: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct OutcomeOrigin {
    channel_id: String,
    thread_root_event_id: Option<String>,
    last_event_id: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct OutcomeAction {
    description: String,
    owner: Option<String>,
    due_at: Option<DateTime<Utc>>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
enum OutcomeStatus {
    Active,
    Closed,
    Superseded,
}

#[derive(Clone)]
struct ParsedOutcome {
    outcome: CommandDiscussionOutcome,
    persona_id: String,
    agent_pubkey: String,
    engram_event_id: String,
    engram_created_at: u64,
}

impl ParsedOutcome {
    fn into_candidate(self, observed_at: &str) -> Result<CandidateSource, ()> {
        let quote = serde_jcs::to_vec(&self.outcome)
            .ok()
            .and_then(|bytes| String::from_utf8(bytes).ok())
            .ok_or(())?;
        let root = self
            .outcome
            .origin
            .thread_root_event_id
            .as_deref()
            .unwrap_or("none");
        Ok(CandidateSource {
            source_id: self.engram_event_id.clone(),
            source_kind: SourceKind::Memory,
            collection: COMMAND_TEAM_COLLECTION.to_string(),
            document_id: self.outcome.outcome_id.clone(),
            chunk_id: self.engram_event_id,
            timestamp: self.outcome.recorded_at.to_rfc3339(),
            location: format!(
                "command adviser persona {} agent {} Buzz channel {} thread root {} triggering event {} engram created {}",
                self.persona_id,
                self.agent_pubkey,
                self.outcome.origin.channel_id,
                root,
                self.outcome.origin.last_event_id,
                self.engram_created_at,
            ),
            retrieved_at: observed_at.to_string(),
            observed_at: observed_at.to_string(),
            quote,
        })
    }
}

fn parse_record(record: &DiscussionMemoryRecord) -> Result<ParsedOutcome, ()> {
    if record.entry.body.len() > MAX_OUTCOME_JSON_BYTES
        || !is_lower_hex_64(&record.entry.event_id)
        || !is_lower_hex_64(&record.agent_pubkey)
    {
        return Err(());
    }
    let outcome =
        serde_json::from_str::<CommandDiscussionOutcome>(&record.entry.body).map_err(|_| ())?;
    if outcome.schema != OUTCOME_SCHEMA {
        return Err(());
    }
    let authoritative_adviser = adviser_for_persona(&record.persona_id).ok_or(())?;
    if outcome.adviser != authoritative_adviser
        || !is_lower_hex_64(&outcome.outcome_id)
        || !is_lower_hex_64(&outcome.origin.last_event_id)
        || outcome
            .origin
            .thread_root_event_id
            .as_deref()
            .is_some_and(|value| !is_lower_hex_64(value))
    {
        return Err(());
    }
    let channel_id = Uuid::parse_str(&outcome.origin.channel_id).map_err(|_| ())?;
    if channel_id.to_string() != outcome.origin.channel_id {
        return Err(());
    }
    let expected_id = outcome_id(
        &record.persona_id,
        &outcome.origin.channel_id,
        &outcome.origin.last_event_id,
    );
    if outcome.outcome_id != expected_id {
        return Err(());
    }

    let parts = record.entry.slug.split('/').collect::<Vec<_>>();
    if parts.len() != 5
        || parts[0] != "mem"
        || parts[1] != "command-brief"
        || parts[2] != adviser_label(outcome.adviser)
        || parts[3] != outcome.recorded_at.date_naive().to_string()
        || parts[4] != outcome.outcome_id
        || !record.entry.slug.starts_with(OUTCOME_PREFIX)
    {
        return Err(());
    }
    if !is_bounded_text(&outcome.summary)
        || !is_bounded_text_array(&outcome.decisions)
        || !is_bounded_text_array(&outcome.risks)
        || !is_bounded_text_array(&outcome.assumptions)
        || !is_bounded_text_array(&outcome.unresolved_questions)
        || outcome.actions.len() > MAX_ARRAY_ITEMS
        || outcome.actions.iter().any(|action| {
            !is_bounded_text(&action.description)
                || action
                    .owner
                    .as_deref()
                    .is_some_and(|owner| !is_bounded_text(owner))
        })
        || outcome.brief_sections.is_empty()
        || outcome.brief_sections.len() > MAX_ARRAY_ITEMS
        || outcome.brief_sections.iter().collect::<BTreeSet<_>>().len()
            != outcome.brief_sections.len()
        || outcome.supersedes.len() > MAX_ARRAY_ITEMS
        || outcome
            .supersedes
            .iter()
            .any(|id| !is_lower_hex_64(id) || id == &outcome.outcome_id)
        || outcome.supersedes.iter().collect::<BTreeSet<_>>().len() != outcome.supersedes.len()
    {
        return Err(());
    }

    Ok(ParsedOutcome {
        outcome,
        persona_id: record.persona_id.clone(),
        agent_pubkey: record.agent_pubkey.clone(),
        engram_event_id: record.entry.event_id.clone(),
        engram_created_at: record.entry.created_at,
    })
}

pub(super) fn select_discussions(
    records: Vec<DiscussionMemoryRecord>,
    observed_at: DateTime<Utc>,
) -> CommandTeamDiscussionBatch {
    let mut malformed_count = 0_usize;
    let mut by_outcome = BTreeMap::<String, ParsedOutcome>::new();
    for record in records {
        let Ok(parsed) = parse_record(&record) else {
            malformed_count += 1;
            continue;
        };
        let id = parsed.outcome.outcome_id.clone();
        match by_outcome.get(&id) {
            Some(existing)
                if (
                    existing.engram_created_at,
                    existing.engram_event_id.as_str(),
                ) >= (parsed.engram_created_at, parsed.engram_event_id.as_str()) => {}
            _ => {
                by_outcome.insert(id, parsed);
            }
        }
    }

    let superseded_ids = by_outcome
        .values()
        .filter(|parsed| parsed.outcome.status != OutcomeStatus::Superseded)
        .flat_map(|parsed| parsed.outcome.supersedes.iter().cloned())
        .collect::<BTreeSet<_>>();
    let closed_cutoff = observed_at - Duration::days(90);
    let mut eligible = by_outcome
        .into_values()
        .filter(|parsed| {
            !superseded_ids.contains(&parsed.outcome.outcome_id)
                && match parsed.outcome.status {
                    OutcomeStatus::Active => true,
                    OutcomeStatus::Closed => parsed.outcome.recorded_at >= closed_cutoff,
                    OutcomeStatus::Superseded => false,
                }
        })
        .collect::<Vec<_>>();
    eligible.sort_by(compare_outcomes);

    let mut adviser_counts = BTreeMap::<AdviserId, usize>::new();
    let selected = eligible
        .into_iter()
        .filter(|parsed| {
            let count = adviser_counts.entry(parsed.outcome.adviser).or_default();
            if *count >= MAX_PER_ADVISER {
                return false;
            }
            *count += 1;
            true
        })
        .take(MAX_TEAM_OUTCOMES)
        .collect::<Vec<_>>();
    let observed_at = observed_at.to_rfc3339_opts(SecondsFormat::Millis, true);
    let mut candidates = Vec::with_capacity(selected.len());
    for parsed in selected {
        match parsed.into_candidate(&observed_at) {
            Ok(candidate) => candidates.push(candidate),
            Err(()) => malformed_count += 1,
        }
    }
    let limitations = if malformed_count == 0 {
        Vec::new()
    } else {
        vec![format!(
            "Command-team discussion memory excluded {malformed_count} malformed or ineligible entries."
        )]
    };
    CommandTeamDiscussionBatch {
        candidates,
        limitations,
    }
}

pub(crate) async fn load_command_team_discussions(
    app: &tauri::AppHandle,
    observed_at: DateTime<Utc>,
) -> Result<CommandTeamDiscussionBatch, String> {
    let agents = load_managed_agents(app)?;
    let state = app
        .try_state::<AppState>()
        .ok_or_else(|| "command-team memory state unavailable".to_string())?;
    let mut records = Vec::new();
    let mut read_failures = 0_usize;
    let mut truncated = 0_usize;
    for agent in agents {
        let Some(persona_id) = agent
            .persona_id
            .as_deref()
            .filter(|id| adviser_for_persona(id).is_some())
        else {
            continue;
        };
        match read_agent_memory_listing(&agent.pubkey, app, state.inner()).await {
            Ok(listing) => {
                if listing.truncated {
                    truncated += 1;
                }
                records.extend(
                    listing
                        .memories
                        .into_iter()
                        .map(|entry| DiscussionMemoryRecord {
                            persona_id: persona_id.to_string(),
                            agent_pubkey: agent.pubkey.clone(),
                            entry,
                        }),
                );
            }
            Err(_) => read_failures += 1,
        }
    }
    let mut batch = select_discussions(records, observed_at);
    if read_failures > 0 {
        batch.limitations.push(format!(
            "Command-team discussion memory was unavailable for {read_failures} adviser instance(s)."
        ));
    }
    if truncated > 0 {
        batch.limitations.push(format!(
            "Command-team discussion memory may be incomplete for {truncated} adviser instance(s)."
        ));
    }
    Ok(batch)
}

fn compare_outcomes(left: &ParsedOutcome, right: &ParsedOutcome) -> std::cmp::Ordering {
    status_priority(left.outcome.status)
        .cmp(&status_priority(right.outcome.status))
        .then_with(|| right.outcome.recorded_at.cmp(&left.outcome.recorded_at))
        .then_with(|| right.engram_created_at.cmp(&left.engram_created_at))
        .then_with(|| right.engram_event_id.cmp(&left.engram_event_id))
}

const fn status_priority(status: OutcomeStatus) -> u8 {
    match status {
        OutcomeStatus::Active => 0,
        OutcomeStatus::Closed => 1,
        OutcomeStatus::Superseded => 2,
    }
}

fn adviser_for_persona(persona_id: &str) -> Option<AdviserId> {
    match persona_id {
        "builtin:command-chief-of-staff" => Some(AdviserId::ChiefOfStaff),
        "builtin:command-operations" => Some(AdviserId::Operations),
        "builtin:command-intelligence" => Some(AdviserId::Intelligence),
        "builtin:command-logistics" => Some(AdviserId::Logistics),
        "builtin:command-navigation" => Some(AdviserId::Navigation),
        "builtin:command-daily-routine" => Some(AdviserId::DailyRoutine),
        "builtin:command-reporting" => Some(AdviserId::Reporting),
        "builtin:command-plans" => Some(AdviserId::Plans),
        _ => None,
    }
}

const fn adviser_label(adviser: AdviserId) -> &'static str {
    match adviser {
        AdviserId::ChiefOfStaff => "chief_of_staff",
        AdviserId::Operations => "operations",
        AdviserId::Intelligence => "intelligence",
        AdviserId::Logistics => "logistics",
        AdviserId::Navigation => "navigation",
        AdviserId::DailyRoutine => "daily_routine",
        AdviserId::Reporting => "reporting",
        AdviserId::Plans => "plans",
    }
}

fn outcome_id(persona_id: &str, channel_id: &str, event_id: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(format!("{persona_id}\n{channel_id}\n{event_id}").as_bytes());
    hex::encode(hasher.finalize())
}

fn is_lower_hex_64(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn is_bounded_text(value: &str) -> bool {
    !value.is_empty()
        && value.trim() == value
        && value.len() <= MAX_TEXT_BYTES
        && !value.chars().any(char::is_control)
}

fn is_bounded_text_array(values: &[String]) -> bool {
    values.len() <= MAX_ARRAY_ITEMS && values.iter().all(|value| is_bounded_text(value))
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use chrono::{DateTime, Utc};
    use serde_json::{json, Value};
    use sha2::{Digest, Sha256};

    use crate::commands::engrams::EngramEntry;

    use super::{
        parse_record, select_discussions, CommandTeamDiscussionBatch, DiscussionMemoryRecord,
        COMMAND_TEAM_COLLECTION,
    };

    const CHANNEL_ID: &str = "11111111-2222-3333-4444-555555555555";
    const OPERATIONS_PERSONA: &str = "builtin:command-operations";
    const OPERATIONS_EVENT: &str =
        "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    const OPERATIONS_OUTCOME: &str =
        "8f47f2665113bb5c554ba5cd9dbdbfdb3d32cef2bd96126015dcfe26b14eb6f0";

    fn observed_at() -> DateTime<Utc> {
        "2026-07-27T12:00:00Z".parse().unwrap()
    }

    fn digest(persona_id: &str, event_id: &str) -> String {
        let mut hasher = Sha256::new();
        hasher.update(format!("{persona_id}\n{CHANNEL_ID}\n{event_id}").as_bytes());
        hex::encode(hasher.finalize())
    }

    fn adviser_for_persona(persona_id: &str) -> &'static str {
        match persona_id {
            "builtin:command-chief-of-staff" => "chief_of_staff",
            "builtin:command-operations" => "operations",
            "builtin:command-intelligence" => "intelligence",
            "builtin:command-logistics" => "logistics",
            "builtin:command-navigation" => "navigation",
            "builtin:command-daily-routine" => "daily_routine",
            "builtin:command-reporting" => "reporting",
            "builtin:command-plans" => "plans",
            _ => panic!("unsupported test persona"),
        }
    }

    fn section_for_adviser(adviser: &str) -> &'static str {
        match adviser {
            "chief_of_staff" => "conflicts_and_gaps",
            "operations" => "operations",
            "intelligence" => "intelligence",
            "logistics" => "logistics",
            "navigation" => "navigation",
            "daily_routine" => "daily_routine",
            "reporting" => "reports",
            "plans" => "planning_30_60_90",
            _ => panic!("unsupported test adviser"),
        }
    }

    fn record(
        persona_id: &str,
        trigger_event_id: &str,
        engram_event_id: &str,
        recorded_at: &str,
        status: &str,
    ) -> DiscussionMemoryRecord {
        let adviser = adviser_for_persona(persona_id);
        let outcome_id = digest(persona_id, trigger_event_id);
        let value = json!({
            "schema": "command-discussion-outcome-v1",
            "outcome_id": outcome_id,
            "adviser": adviser,
            "recorded_at": recorded_at,
            "origin": {
                "channel_id": CHANNEL_ID,
                "thread_root_event_id": null,
                "last_event_id": trigger_event_id,
            },
            "status": status,
            "summary": format!("Controlled {adviser} outcome."),
            "decisions": [],
            "actions": [{
                "description": "Obtain the missing readiness input.",
                "owner": null,
                "due_at": null,
            }],
            "risks": [],
            "assumptions": [],
            "unresolved_questions": [],
            "brief_sections": [section_for_adviser(adviser)],
            "review_at": null,
            "supersedes": [],
        });
        DiscussionMemoryRecord {
            persona_id: persona_id.to_string(),
            agent_pubkey: "c".repeat(64),
            entry: EngramEntry {
                slug: format!(
                    "mem/command-brief/{adviser}/{}/{}",
                    &recorded_at[..10],
                    value["outcome_id"].as_str().unwrap()
                ),
                body: serde_json::to_string(&value).unwrap(),
                event_id: engram_event_id.to_string(),
                created_at: DateTime::parse_from_rfc3339(recorded_at)
                    .unwrap()
                    .timestamp()
                    .try_into()
                    .unwrap(),
                outgoing_refs: Vec::new(),
            },
        }
    }

    fn mutate_body(
        source: &DiscussionMemoryRecord,
        mutation: impl FnOnce(&mut serde_json::Map<String, Value>),
    ) -> DiscussionMemoryRecord {
        let mut changed = DiscussionMemoryRecord {
            persona_id: source.persona_id.clone(),
            agent_pubkey: source.agent_pubkey.clone(),
            entry: EngramEntry {
                slug: source.entry.slug.clone(),
                body: source.entry.body.clone(),
                event_id: source.entry.event_id.clone(),
                created_at: source.entry.created_at,
                outgoing_refs: Vec::new(),
            },
        };
        let mut value = serde_json::from_str::<Value>(&changed.entry.body).unwrap();
        mutation(value.as_object_mut().unwrap());
        changed.entry.body = serde_json::to_string(&value).unwrap();
        changed
    }

    #[test]
    fn valid_record_becomes_bounded_memory_evidence_with_origin_provenance() {
        let input = record(
            OPERATIONS_PERSONA,
            OPERATIONS_EVENT,
            &"d".repeat(64),
            "2026-07-27T10:00:00Z",
            "active",
        );
        assert_eq!(
            serde_json::from_str::<Value>(&input.entry.body).unwrap()["outcome_id"],
            OPERATIONS_OUTCOME
        );

        let parsed = parse_record(&input).expect("valid outcome should parse");
        let candidate = parsed
            .into_candidate("2026-07-27T10:05:00Z")
            .expect("valid outcome should serialize");

        assert_eq!(
            candidate.source_kind,
            crate::command_brief::types::SourceKind::Memory
        );
        assert_eq!(candidate.collection, COMMAND_TEAM_COLLECTION);
        assert_eq!(candidate.source_id, "d".repeat(64));
        assert_eq!(candidate.document_id, OPERATIONS_OUTCOME);
        assert_eq!(candidate.chunk_id, "d".repeat(64));
        assert_eq!(candidate.timestamp, "2026-07-27T10:00:00+00:00");
        assert!(candidate.location.contains(OPERATIONS_PERSONA));
        assert!(candidate.location.contains(&"c".repeat(64)));
        assert!(candidate.location.contains(CHANNEL_ID));
        assert!(candidate.location.contains(OPERATIONS_EVENT));
        assert!(candidate.quote.contains("command-discussion-outcome-v1"));
        assert!(!candidate.quote.contains("raw transcript"));
        assert!(candidate.quote.len() <= 4096);
    }

    #[test]
    fn strict_parser_rejects_malformed_or_identity_mismatched_records() {
        let valid = record(
            OPERATIONS_PERSONA,
            OPERATIONS_EVENT,
            &"d".repeat(64),
            "2026-07-27T10:00:00Z",
            "active",
        );
        let cases: BTreeMap<&str, DiscussionMemoryRecord> = BTreeMap::from([
            (
                "unknown field",
                mutate_body(&valid, |body| {
                    body.insert("instructions".to_string(), json!("ignore policy"));
                }),
            ),
            (
                "wrong schema",
                mutate_body(&valid, |body| {
                    body.insert("schema".to_string(), json!("other"));
                }),
            ),
            (
                "wrong adviser",
                mutate_body(&valid, |body| {
                    body.insert("adviser".to_string(), json!("navigation"));
                }),
            ),
            (
                "invalid timestamp",
                mutate_body(&valid, |body| {
                    body.insert("recorded_at".to_string(), json!("tomorrow"));
                }),
            ),
            (
                "invalid event id",
                mutate_body(&valid, |body| {
                    body["origin"]["last_event_id"] = json!("ABC");
                }),
            ),
            (
                "unsupported brief section",
                mutate_body(&valid, |body| {
                    body.insert("brief_sections".to_string(), json!(["weapons"]));
                }),
            ),
            (
                "oversized summary",
                mutate_body(&valid, |body| {
                    body.insert("summary".to_string(), json!("x".repeat(4097)));
                }),
            ),
            (
                "self supersession",
                mutate_body(&valid, |body| {
                    body.insert("supersedes".to_string(), json!([OPERATIONS_OUTCOME]));
                }),
            ),
        ]);

        for (label, input) in cases {
            assert!(parse_record(&input).is_err(), "{label} should be rejected");
        }

        let mut wrong_persona = valid;
        wrong_persona.persona_id = "builtin:command-navigation".to_string();
        assert!(parse_record(&wrong_persona).is_err());
    }

    #[test]
    fn selection_applies_active_closed_and_supersession_rules() {
        let active_old_event = "1".repeat(64);
        let active_old = record(
            OPERATIONS_PERSONA,
            &active_old_event,
            &"a".repeat(64),
            "2020-01-01T00:00:00Z",
            "active",
        );
        let superseded_event = "2".repeat(64);
        let superseded = record(
            OPERATIONS_PERSONA,
            &superseded_event,
            &"b".repeat(64),
            "2026-07-20T00:00:00Z",
            "active",
        );
        let replacement_event = "3".repeat(64);
        let replacement = mutate_body(
            &record(
                OPERATIONS_PERSONA,
                &replacement_event,
                &"c".repeat(64),
                "2026-07-25T00:00:00Z",
                "active",
            ),
            |body| {
                body.insert(
                    "supersedes".to_string(),
                    json!([digest(OPERATIONS_PERSONA, &superseded_event)]),
                );
            },
        );
        let recent_closed = record(
            "builtin:command-navigation",
            &"4".repeat(64),
            &"d".repeat(64),
            "2026-07-01T00:00:00Z",
            "closed",
        );
        let expired_closed = record(
            "builtin:command-navigation",
            &"5".repeat(64),
            &"e".repeat(64),
            "2026-01-01T00:00:00Z",
            "closed",
        );
        let status_superseded = record(
            "builtin:command-plans",
            &"6".repeat(64),
            &"f".repeat(64),
            "2026-07-26T00:00:00Z",
            "superseded",
        );

        let batch = select_discussions(
            vec![
                active_old,
                superseded,
                replacement,
                recent_closed,
                expired_closed,
                status_superseded,
            ],
            observed_at(),
        );
        let ids = batch
            .candidates
            .iter()
            .map(|candidate| candidate.document_id.as_str())
            .collect::<Vec<_>>();

        assert_eq!(
            ids,
            vec![
                digest(OPERATIONS_PERSONA, &replacement_event),
                digest(OPERATIONS_PERSONA, &active_old_event),
                digest("builtin:command-navigation", &"4".repeat(64)),
            ]
        );
    }

    #[test]
    fn selection_is_deterministic_and_enforces_per_adviser_and_team_caps() {
        let personas = [
            "builtin:command-chief-of-staff",
            "builtin:command-operations",
            "builtin:command-navigation",
            "builtin:command-daily-routine",
            "builtin:command-reporting",
            "builtin:command-plans",
        ];
        let mut records = Vec::new();
        for (persona_index, persona_id) in personas.iter().enumerate() {
            for outcome_index in 0..7 {
                let trigger = format!("{:064x}", outcome_index + persona_index * 16 + 1);
                let engram = format!("{:064x}", outcome_index + persona_index * 32 + 1);
                records.push(record(
                    persona_id,
                    &trigger,
                    &engram,
                    &format!("2026-07-{:02}T00:00:00Z", outcome_index + 1),
                    "active",
                ));
            }
        }

        let batch = select_discussions(records, observed_at());

        assert_eq!(batch.candidates.len(), 24);
        let mut by_adviser = BTreeMap::<String, usize>::new();
        for candidate in &batch.candidates {
            let quote = serde_json::from_str::<Value>(&candidate.quote).unwrap();
            *by_adviser
                .entry(quote["adviser"].as_str().unwrap().to_string())
                .or_default() += 1;
        }
        assert!(by_adviser.values().all(|count| *count <= 6));
        assert!(batch
            .candidates
            .windows(2)
            .all(|pair| pair[0].timestamp >= pair[1].timestamp));
    }

    #[test]
    fn duplicate_logical_outcome_uses_the_newest_engram_event() {
        let trigger = "7".repeat(64);
        let older = record(
            OPERATIONS_PERSONA,
            &trigger,
            &"1".repeat(64),
            "2026-07-27T10:00:00Z",
            "active",
        );
        let mut newer = record(
            OPERATIONS_PERSONA,
            &trigger,
            &"2".repeat(64),
            "2026-07-27T10:00:00Z",
            "active",
        );
        newer.agent_pubkey = "e".repeat(64);
        newer.entry.created_at += 60;

        let CommandTeamDiscussionBatch { candidates, .. } =
            select_discussions(vec![older, newer], observed_at());

        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].source_id, "2".repeat(64));
        assert!(candidates[0].location.contains(&"e".repeat(64)));
    }
}
