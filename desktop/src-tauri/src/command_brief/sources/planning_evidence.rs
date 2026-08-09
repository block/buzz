use std::collections::{BTreeMap, BTreeSet};

use chrono::{DateTime, Duration, SecondsFormat, Utc};
use serde_json::{json, Map, Value};
use tauri::Manager;

use super::canonical::CandidateSource;
use super::SourceKind;
use crate::app_state::AppState;
use crate::relay::query_relay;
use buzz_core_pkg::kind::{
    KIND_BATTLE_RHYTHM_EVENT, KIND_MISSION_CONSTRAINT, KIND_PLANNING_PROJECT, KIND_PLANNING_TASK,
};

const MAX_CONTENT_BYTES: usize = 64 * 1024;
const MAX_TEXT_BYTES: usize = 4_096;
const MAX_PLANNING_SOURCES: usize = 48;
const PLANNING_HORIZON_DAYS: i64 = 120;

#[derive(Clone, Debug)]
struct RawPlanningEvent {
    event_id: String,
    author: String,
    kind: u32,
    d_tag: String,
    created_at: u64,
    content: String,
}

#[derive(Clone, Debug, Default)]
pub(crate) struct PlanningEvidenceBatch {
    pub(super) candidates: Vec<CandidateSource>,
    pub(super) limitations: Vec<String>,
}

impl PlanningEvidenceBatch {
    pub(crate) fn unavailable(limitation: &str) -> Self {
        Self {
            candidates: Vec::new(),
            limitations: vec![limitation.to_string()],
        }
    }
}

#[cfg(test)]
impl PlanningEvidenceBatch {
    pub(crate) fn for_test() -> Self {
        Self {
            candidates: vec![
                CandidateSource {
                    source_id: "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc"
                        .to_string(),
                    source_kind: SourceKind::BattleRhythm,
                    collection: "battle_rhythm".to_string(),
                    document_id: "sail-manila".to_string(),
                    chunk_id: "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc"
                        .to_string(),
                    timestamp: "2026-07-25T05:55:00+10:00".to_string(),
                    location: "signed Buzz planning event kind 30631".to_string(),
                    retrieved_at: "2026-07-25T06:00:00+10:00".to_string(),
                    observed_at: "2026-07-25T06:00:00+10:00".to_string(),
                    quote: r#"{"recordType":"battle_rhythm_event","title":"Sail Manila"}"#
                        .to_string(),
                },
                CandidateSource {
                    source_id: "dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd"
                        .to_string(),
                    source_kind: SourceKind::Plans,
                    collection: "command_plans".to_string(),
                    document_id: "repair-davit".to_string(),
                    chunk_id: "dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd"
                        .to_string(),
                    timestamp: "2026-07-25T05:56:00+10:00".to_string(),
                    location: "signed Buzz planning event kind 30633".to_string(),
                    retrieved_at: "2026-07-25T06:00:00+10:00".to_string(),
                    observed_at: "2026-07-25T06:00:00+10:00".to_string(),
                    quote: r#"{"recordType":"planning_task","title":"Repair davit"}"#.to_string(),
                },
            ],
            limitations: Vec::new(),
        }
    }
}

#[derive(Clone)]
struct PreparedCandidate {
    candidate: CandidateSource,
    sort_key: String,
}

fn select_planning_evidence(
    records: Vec<RawPlanningEvent>,
    owner_pubkey: &str,
    observed_at: &str,
) -> PlanningEvidenceBatch {
    let Ok(observed) = DateTime::parse_from_rfc3339(observed_at) else {
        return PlanningEvidenceBatch::unavailable(
            "Planning evidence was unavailable because the observation time was invalid.",
        );
    };
    let mut heads = BTreeMap::<(u32, String), RawPlanningEvent>::new();
    for record in records {
        let key = (record.kind, record.d_tag.clone());
        match heads.get(&key) {
            Some(prior)
                if (prior.created_at, prior.event_id.as_str())
                    >= (record.created_at, record.event_id.as_str()) => {}
            _ => {
                heads.insert(key, record);
            }
        }
    }
    let active_projects = heads
        .values()
        .filter(|record| record.kind == KIND_PLANNING_PROJECT)
        .filter_map(|record| {
            let value = parse_record_value(record, owner_pubkey)?;
            let id = required_string(&value, "id")?;
            (id == record.d_tag && required_string(&value, "status") == Some("active"))
                .then(|| id.to_string())
        })
        .collect::<BTreeSet<_>>();

    let mut malformed = 0_usize;
    let mut prepared = Vec::new();
    for record in heads.into_values() {
        match prepare_candidate(record, owner_pubkey, observed, &active_projects) {
            Ok(Some(candidate)) => prepared.push(candidate),
            Ok(None) => {}
            Err(()) => malformed += 1,
        }
    }
    prepared.sort_by(|left, right| {
        left.sort_key
            .cmp(&right.sort_key)
            .then_with(|| left.candidate.source_id.cmp(&right.candidate.source_id))
    });
    let omitted = prepared.len().saturating_sub(MAX_PLANNING_SOURCES);
    prepared.truncate(MAX_PLANNING_SOURCES);
    let mut limitations = Vec::new();
    if malformed > 0 {
        limitations.push(format!(
            "Planning evidence excluded {malformed} malformed or ineligible signed event{}.",
            if malformed == 1 { "" } else { "s" }
        ));
    }
    if omitted > 0 {
        limitations.push(format!(
            "Planning evidence omitted {omitted} lower-priority event{} from the bounded brief input.",
            if omitted == 1 { "" } else { "s" }
        ));
    }
    PlanningEvidenceBatch {
        candidates: prepared
            .into_iter()
            .map(|prepared| prepared.candidate)
            .collect(),
        limitations,
    }
}

fn prepare_candidate(
    record: RawPlanningEvent,
    owner_pubkey: &str,
    observed: DateTime<chrono::FixedOffset>,
    active_projects: &BTreeSet<String>,
) -> Result<Option<PreparedCandidate>, ()> {
    let value = parse_record_value(&record, owner_pubkey).ok_or(())?;
    let object = value.as_object().ok_or(())?;
    let id = required_string(&value, "id").ok_or(())?;
    if id != record.d_tag {
        return Err(());
    }
    let timestamp = DateTime::<Utc>::from_timestamp(record.created_at as i64, 0)
        .ok_or(())?
        .to_rfc3339_opts(SecondsFormat::Secs, true);
    let (source_kind, collection, quote, sort_key) = match record.kind {
        KIND_BATTLE_RHYTHM_EVENT => {
            if required_string(&value, "status") != Some("approved") {
                return Ok(None);
            }
            required_string(&value, "title").ok_or(())?;
            let start = parse_timestamp_field(&value, "start").ok_or(())?;
            let end = parse_timestamp_field(&value, "end").ok_or(())?;
            if start >= end
                || end < observed - Duration::days(1)
                || start > observed + Duration::days(PLANNING_HORIZON_DAYS)
            {
                return Ok(None);
            }
            (
                SourceKind::BattleRhythm,
                "battle_rhythm",
                bounded_projection(
                    "battle_rhythm_event",
                    object,
                    &[
                        "id",
                        "title",
                        "status",
                        "start",
                        "end",
                        "allDay",
                        "location",
                        "responsibleOwner",
                        "remarks",
                        "linkedPlanId",
                        "linkedTaskId",
                    ],
                )?,
                format!("0:{}", start.to_rfc3339()),
            )
        }
        KIND_PLANNING_PROJECT => {
            if required_string(&value, "status") != Some("active") {
                return Ok(None);
            }
            required_string(&value, "title").ok_or(())?;
            required_string(&value, "purpose").ok_or(())?;
            required_string(&value, "owner").ok_or(())?;
            let ready = parse_date_field(&value, "missionReadyDate").ok_or(())?;
            (
                SourceKind::Plans,
                "command_plans",
                bounded_projection(
                    "planning_project",
                    object,
                    &[
                        "id",
                        "title",
                        "purpose",
                        "missionReadyDate",
                        "status",
                        "progressPercent",
                        "owner",
                        "assumptions",
                    ],
                )?,
                format!("1:{ready}"),
            )
        }
        KIND_PLANNING_TASK => {
            let status = required_string(&value, "status").ok_or(())?;
            let project_id = required_string(&value, "projectId").ok_or(())?;
            required_string(&value, "wbs").ok_or(())?;
            required_string(&value, "title").ok_or(())?;
            required_string(&value, "owner").ok_or(())?;
            if matches!(status, "complete" | "cancelled") || !active_projects.contains(project_id) {
                return Ok(None);
            }
            let due = optional_date_field(&value, "dueDate")?;
            if due.as_ref().is_some_and(|due| {
                DateTime::parse_from_rfc3339(&format!("{due}T23:59:59Z"))
                    .is_ok_and(|due| due > observed + Duration::days(PLANNING_HORIZON_DAYS))
            }) {
                return Ok(None);
            }
            (
                SourceKind::Plans,
                "command_plans",
                bounded_projection(
                    "planning_task",
                    object,
                    &[
                        "id",
                        "projectId",
                        "wbs",
                        "title",
                        "owner",
                        "status",
                        "percentComplete",
                        "plannedStart",
                        "dueDate",
                        "durationWorkdays",
                        "dependencyIds",
                        "linkedCapabilityId",
                        "linkedMissionRequirementId",
                        "notes",
                    ],
                )?,
                format!("2:{}", due.unwrap_or_else(|| "9999-12-31".to_string())),
            )
        }
        KIND_MISSION_CONSTRAINT => {
            let status = required_string(&value, "status").ok_or(())?;
            let project_id = required_string(&value, "projectId").ok_or(())?;
            required_string(&value, "type").ok_or(())?;
            required_string(&value, "description").ok_or(())?;
            required_string(&value, "owner").ok_or(())?;
            if matches!(status, "resolved" | "missionChanged")
                || !active_projects.contains(project_id)
            {
                return Ok(None);
            }
            let severity = required_string(&value, "severity").ok_or(())?;
            let severity_order = match severity {
                "critical" => 0,
                "high" => 1,
                "medium" => 2,
                "low" => 3,
                _ => return Err(()),
            };
            (
                SourceKind::Plans,
                "command_plans",
                bounded_projection(
                    "mission_constraint",
                    object,
                    &[
                        "id",
                        "projectId",
                        "type",
                        "description",
                        "owner",
                        "severity",
                        "status",
                        "linkedTaskId",
                        "linkedMilestoneId",
                        "requiredDate",
                        "dispositionNote",
                    ],
                )?,
                format!("3:{severity_order}:{}", record.d_tag),
            )
        }
        _ => return Err(()),
    };
    Ok(Some(PreparedCandidate {
        candidate: CandidateSource {
            source_id: record.event_id.clone(),
            source_kind,
            collection: collection.to_string(),
            document_id: record.d_tag.clone(),
            chunk_id: record.event_id,
            timestamp,
            location: format!(
                "signed Buzz planning event kind {} d tag {} author {}",
                record.kind, record.d_tag, record.author
            ),
            retrieved_at: observed.to_rfc3339_opts(SecondsFormat::Millis, true),
            observed_at: observed.to_rfc3339_opts(SecondsFormat::Millis, true),
            quote,
        },
        sort_key,
    }))
}

fn parse_record_value(record: &RawPlanningEvent, owner_pubkey: &str) -> Option<Value> {
    if record.author != owner_pubkey
        || !is_lower_hex_64(&record.author)
        || !is_lower_hex_64(&record.event_id)
        || record.d_tag.is_empty()
        || record.d_tag.len() > 256
        || record.d_tag.chars().any(char::is_control)
        || record.content.len() > MAX_CONTENT_BYTES
    {
        return None;
    }
    let value = serde_json::from_str::<Value>(&record.content).ok()?;
    (value.get("schemaVersion")?.as_u64() == Some(1)).then_some(value)
}

fn required_string<'a>(value: &'a Value, key: &str) -> Option<&'a str> {
    value
        .get(key)?
        .as_str()
        .filter(|text| !text.trim().is_empty() && text.len() <= MAX_TEXT_BYTES)
}

fn parse_timestamp_field(value: &Value, key: &str) -> Option<DateTime<chrono::FixedOffset>> {
    DateTime::parse_from_rfc3339(required_string(value, key)?).ok()
}

fn parse_date_field(value: &Value, key: &str) -> Option<String> {
    let date = required_string(value, key)?;
    if date.len() != 10 {
        return None;
    }
    DateTime::parse_from_rfc3339(&format!("{date}T00:00:00Z"))
        .ok()
        .map(|_| date.to_string())
}

fn optional_date_field(value: &Value, key: &str) -> Result<Option<String>, ()> {
    match value.get(key) {
        Some(Value::Null) | None => Ok(None),
        Some(_) => parse_date_field(value, key).map(Some).ok_or(()),
    }
}

fn bounded_projection(
    record_type: &str,
    source: &Map<String, Value>,
    fields: &[&str],
) -> Result<String, ()> {
    let mut projected = Map::new();
    projected.insert("recordType".to_string(), json!(record_type));
    for field in fields {
        if let Some(value) = source.get(*field) {
            projected.insert((*field).to_string(), bounded_value(value)?);
        }
    }
    let quote = serde_json::to_string(&Value::Object(projected)).map_err(|_| ())?;
    (quote.len() <= MAX_CONTENT_BYTES)
        .then_some(quote)
        .ok_or(())
}

fn bounded_value(value: &Value) -> Result<Value, ()> {
    match value {
        Value::Null | Value::Bool(_) | Value::Number(_) => Ok(value.clone()),
        Value::String(text) if text.len() <= MAX_TEXT_BYTES => Ok(value.clone()),
        Value::Array(items) if items.len() <= 64 => items
            .iter()
            .map(|item| match item {
                Value::String(text) if text.len() <= MAX_TEXT_BYTES => Ok(item.clone()),
                _ => Err(()),
            })
            .collect::<Result<Vec<_>, _>>()
            .map(Value::Array),
        _ => Err(()),
    }
}

fn is_lower_hex_64(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

pub(crate) async fn load_planning_evidence(
    app: &tauri::AppHandle,
    observed_at: DateTime<Utc>,
) -> Result<PlanningEvidenceBatch, String> {
    let state = app
        .try_state::<AppState>()
        .ok_or_else(|| "planning evidence state unavailable".to_string())?;
    let owner_pubkey = state.signing_keys()?.public_key().to_hex();
    let filters = [
        json!({
            "kinds": [KIND_BATTLE_RHYTHM_EVENT],
            "authors": [owner_pubkey],
            "limit": 2000
        }),
        json!({
            "kinds": [KIND_PLANNING_PROJECT],
            "authors": [owner_pubkey],
            "limit": 500
        }),
        json!({
            "kinds": [KIND_PLANNING_TASK],
            "authors": [owner_pubkey],
            "limit": 5000
        }),
        json!({
            "kinds": [KIND_MISSION_CONSTRAINT],
            "authors": [owner_pubkey],
            "limit": 2000
        }),
    ];
    let events = query_relay(state.inner(), &filters).await?;
    let mut invalid_signatures = 0_usize;
    let records = events
        .into_iter()
        .filter_map(|event| {
            if event.verify().is_err() {
                invalid_signatures += 1;
                return None;
            }
            let d_tag = event.tags.iter().find_map(|tag| {
                let values = tag.as_slice();
                (values.len() == 2 && values[0].as_str() == "d")
                    .then(|| values[1].as_str().to_string())
            })?;
            Some(RawPlanningEvent {
                event_id: event.id.to_hex(),
                author: event.pubkey.to_hex(),
                kind: event.kind.as_u16() as u32,
                d_tag,
                created_at: event.created_at.as_secs(),
                content: event.content,
            })
        })
        .collect();
    let observed_at = observed_at.to_rfc3339_opts(SecondsFormat::Millis, true);
    let mut batch = select_planning_evidence(records, &owner_pubkey, &observed_at);
    if invalid_signatures > 0 {
        batch.limitations.push(format!(
            "Planning evidence excluded {invalid_signatures} event{} with an invalid signature.",
            if invalid_signatures == 1 { "" } else { "s" }
        ));
    }
    Ok(batch)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use buzz_core_pkg::kind::{
        KIND_BATTLE_RHYTHM_EVENT, KIND_MISSION_CONSTRAINT, KIND_PLANNING_PROJECT,
        KIND_PLANNING_TASK,
    };

    const OWNER: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    const OBSERVED_AT: &str = "2026-07-29T00:00:00Z";

    fn record(
        index: u32,
        kind: u32,
        d_tag: &str,
        created_at: u64,
        content: serde_json::Value,
    ) -> RawPlanningEvent {
        RawPlanningEvent {
            event_id: format!("{index:064x}"),
            author: OWNER.to_string(),
            kind,
            d_tag: d_tag.to_string(),
            created_at,
            content: serde_json::to_string(&content).expect("fixture JSON"),
        }
    }

    #[test]
    fn selects_current_schedule_active_tasks_and_unresolved_constraints() {
        let records = vec![
            record(
                1,
                KIND_BATTLE_RHYTHM_EVENT,
                "sail-manila",
                1_722_211_200,
                json!({
                    "schemaVersion": 1,
                    "id": "sail-manila",
                    "title": "Sail Manila",
                    "status": "approved",
                    "start": "2026-08-03T08:00:00+10:00",
                    "end": "2026-08-03T10:00:00+10:00",
                    "allDay": false,
                    "location": "Sydney",
                    "responsibleOwner": "Operations",
                    "remarks": null
                }),
            ),
            record(
                2,
                KIND_PLANNING_PROJECT,
                "deployment",
                1_722_211_201,
                json!({
                    "schemaVersion": 1,
                    "id": "deployment",
                    "title": "Regional deployment",
                    "purpose": "Sustain assigned maritime forces",
                    "missionReadyDate": "2026-10-20",
                    "status": "active",
                    "progressPercent": 25,
                    "owner": "Operations",
                    "assumptions": ["Port access remains available"]
                }),
            ),
            record(
                3,
                KIND_PLANNING_TASK,
                "repair-davit",
                1_722_211_202,
                json!({
                    "schemaVersion": 1,
                    "id": "repair-davit",
                    "projectId": "deployment",
                    "wbs": "1.2",
                    "title": "Repair seaboat davit",
                    "owner": "Marine Engineer",
                    "status": "blocked",
                    "percentComplete": 30,
                    "plannedStart": "2026-08-01",
                    "dueDate": "2026-08-20",
                    "durationWorkdays": 10,
                    "dependencyIds": ["survey"],
                    "notes": "Required for mission seaboat task"
                }),
            ),
            record(
                4,
                KIND_MISSION_CONSTRAINT,
                "davit-defect",
                1_722_211_203,
                json!({
                    "schemaVersion": 1,
                    "id": "davit-defect",
                    "projectId": "deployment",
                    "type": "defect",
                    "description": "Starboard seaboat davit unavailable",
                    "owner": "Marine Engineer",
                    "severity": "critical",
                    "status": "oplimCandidate",
                    "linkedTaskId": "repair-davit",
                    "requiredDate": "2026-08-20",
                    "dispositionNote": null
                }),
            ),
            record(
                5,
                KIND_PLANNING_TASK,
                "completed-task",
                1_722_211_204,
                json!({
                    "schemaVersion": 1,
                    "id": "completed-task",
                    "projectId": "deployment",
                    "wbs": "1.1",
                    "title": "Completed task",
                    "owner": "Operations",
                    "status": "complete",
                    "percentComplete": 100,
                    "plannedStart": "2026-07-01",
                    "dueDate": "2026-07-02",
                    "durationWorkdays": 2,
                    "dependencyIds": [],
                    "notes": null
                }),
            ),
        ];

        let batch = select_planning_evidence(records, OWNER, OBSERVED_AT);

        assert_eq!(batch.candidates.len(), 4);
        assert!(batch.limitations.is_empty());
        assert!(batch
            .candidates
            .iter()
            .any(|candidate| candidate.collection == "battle_rhythm"
                && candidate.quote.contains("Sail Manila")));
        assert!(batch
            .candidates
            .iter()
            .any(|candidate| candidate.collection == "command_plans"
                && candidate.quote.contains("Repair seaboat davit")));
        assert!(!batch
            .candidates
            .iter()
            .any(|candidate| candidate.quote.contains("Completed task")));
    }

    #[test]
    fn newest_parameterized_event_wins_and_malformed_content_is_reported() {
        let records = vec![
            record(
                10,
                KIND_BATTLE_RHYTHM_EVENT,
                "sailing",
                100,
                json!({
                    "schemaVersion": 1,
                    "id": "sailing",
                    "title": "Old sailing",
                    "status": "approved",
                    "start": "2026-08-03T08:00:00+10:00",
                    "end": "2026-08-03T10:00:00+10:00",
                    "allDay": false,
                    "location": null,
                    "responsibleOwner": null,
                    "remarks": null
                }),
            ),
            record(
                11,
                KIND_BATTLE_RHYTHM_EVENT,
                "sailing",
                101,
                json!({
                    "schemaVersion": 1,
                    "id": "sailing",
                    "title": "Current sailing",
                    "status": "approved",
                    "start": "2026-08-04T08:00:00+10:00",
                    "end": "2026-08-04T10:00:00+10:00",
                    "allDay": false,
                    "location": null,
                    "responsibleOwner": null,
                    "remarks": null
                }),
            ),
            RawPlanningEvent {
                event_id: format!("{:064x}", 12),
                author: OWNER.to_string(),
                kind: KIND_PLANNING_TASK,
                d_tag: "malformed".to_string(),
                created_at: 102,
                content: "{not-json".to_string(),
            },
        ];

        let batch = select_planning_evidence(records, OWNER, OBSERVED_AT);

        assert_eq!(batch.candidates.len(), 1);
        assert!(batch.candidates[0].quote.contains("Current sailing"));
        assert!(!batch.candidates[0].quote.contains("Old sailing"));
        assert_eq!(
            batch.limitations,
            ["Planning evidence excluded 1 malformed or ineligible signed event."]
        );
    }
}
