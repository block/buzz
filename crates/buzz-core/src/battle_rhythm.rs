//! Strict wire contracts for owner-authored Battle Rhythm records.
#![allow(missing_docs)]
use chrono::{DateTime, Datelike, Timelike, Utc};
use chrono_tz::Tz;
use serde::{Deserialize, Deserializer, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BattleRhythmSourceV1 {
    pub schema_version: u8,
    pub id: String,
    #[serde(rename = "type")]
    pub source_type: String,
    pub display_name: String,
    pub coverage_start: String,
    pub coverage_end: String,
    pub document_name: String,
    pub document_hash: String,
    pub revision_id: String,
    pub prior_revision_id: Option<String>,
    pub imported_at: String,
    pub status: String,
    pub source_reference: String,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "camelCase", deny_unknown_fields)]
pub enum EventOwnershipV1 {
    Manual,
    Source {
        source_id: String,
        revision_id: String,
        source_location: String,
    },
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BattleRhythmRecurrenceV1 {
    pub frequency: String,
    pub interval: u16,
    pub until: Option<String>,
    pub series_id: String,
}
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BattleRhythmEventV1 {
    pub schema_version: u8,
    pub id: String,
    pub ownership: EventOwnershipV1,
    pub title: String,
    pub description: Option<String>,
    #[serde(rename = "type")]
    pub event_type: String,
    pub start: String,
    pub end: String,
    pub all_day: bool,
    pub time_zone: String,
    pub status: String,
    pub location: Option<String>,
    pub responsible_owner: Option<String>,
    pub participants: Vec<String>,
    pub remarks: Option<String>,
    pub linked_plan_id: Option<String>,
    pub linked_task_id: Option<String>,
    pub linked_mission_requirement_id: Option<String>,
    pub parent_activity_id: Option<String>,
    pub recurrence: Option<BattleRhythmRecurrenceV1>,
    pub excluded_occurrence_starts: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct BattleRhythmEventWireV1 {
    schema_version: u8,
    id: String,
    ownership: EventOwnershipV1,
    title: String,
    description: Option<String>,
    #[serde(rename = "type")]
    event_type: String,
    start: String,
    end: String,
    all_day: bool,
    time_zone: String,
    status: String,
    location: Option<String>,
    responsible_owner: Option<String>,
    participants: Vec<String>,
    remarks: Option<String>,
    linked_plan_id: Option<String>,
    linked_task_id: Option<String>,
    linked_mission_requirement_id: Option<String>,
    parent_activity_id: Option<String>,
    recurrence: Option<BattleRhythmRecurrenceV1>,
    excluded_occurrence_starts: Vec<String>,
}

fn parsed_timestamp(value: &str) -> Result<DateTime<chrono::FixedOffset>, String> {
    DateTime::parse_from_rfc3339(value).map_err(|_| "timestamp must be RFC3339".to_owned())
}

fn is_occurrence(
    start: DateTime<Utc>,
    candidate: DateTime<Utc>,
    rule: &BattleRhythmRecurrenceV1,
    tz: Tz,
) -> bool {
    let first = start.with_timezone(&tz);
    let next = candidate.with_timezone(&tz);
    if (first.hour(), first.minute(), first.second()) != (next.hour(), next.minute(), next.second())
        || next.date_naive() < first.date_naive()
    {
        return false;
    }
    match rule.frequency.as_str() {
        "daily" => {
            (next.date_naive() - first.date_naive()).num_days() % i64::from(rule.interval) == 0
        }
        "weekly" => {
            let days = (next.date_naive() - first.date_naive()).num_days();
            days % (7 * i64::from(rule.interval)) == 0
        }
        "monthly" => {
            let months =
                (next.year() - first.year()) * 12 + next.month() as i32 - first.month() as i32;
            next.day() == first.day() && months >= 0 && months % i32::from(rule.interval) == 0
        }
        _ => false,
    }
}

impl<'de> Deserialize<'de> for BattleRhythmEventV1 {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let raw = BattleRhythmEventWireV1::deserialize(deserializer)?;
        if raw.schema_version != 1
            || raw.id.is_empty()
            || raw.id.len() > 256
            || raw.time_zone.parse::<Tz>().is_err()
        {
            return Err(serde::de::Error::custom(
                "invalid event identity or timezone",
            ));
        }
        let start = parsed_timestamp(&raw.start).map_err(serde::de::Error::custom)?;
        let end = parsed_timestamp(&raw.end).map_err(serde::de::Error::custom)?;
        if start >= end || raw.excluded_occurrence_starts.len() > 512 {
            return Err(serde::de::Error::custom(
                "invalid event timing or exclusions",
            ));
        }
        let tz = raw
            .time_zone
            .parse::<Tz>()
            .map_err(serde::de::Error::custom)?;
        if let Some(rule) = &raw.recurrence {
            if !matches!(rule.frequency.as_str(), "daily" | "weekly" | "monthly")
                || rule.interval == 0
                || rule.interval > 366
                || rule.series_id.is_empty()
                || rule.series_id.len() > 256
            {
                return Err(serde::de::Error::custom("invalid recurrence rule"));
            }
            let until = rule
                .until
                .as_deref()
                .map(parsed_timestamp)
                .transpose()
                .map_err(serde::de::Error::custom)?;
            if until.as_ref().is_some_and(|value| *value < start) {
                return Err(serde::de::Error::custom("recurrence until precedes start"));
            }
            let mut previous = None;
            for item in &raw.excluded_occurrence_starts {
                let occurrence = parsed_timestamp(item).map_err(serde::de::Error::custom)?;
                if previous.is_some_and(|value: DateTime<chrono::FixedOffset>| value >= occurrence)
                    || until.as_ref().is_some_and(|value| occurrence > *value)
                    || !is_occurrence(
                        start.with_timezone(&Utc),
                        occurrence.with_timezone(&Utc),
                        rule,
                        tz,
                    )
                {
                    return Err(serde::de::Error::custom("invalid excluded occurrence"));
                }
                previous = Some(occurrence);
            }
        } else if !raw.excluded_occurrence_starts.is_empty() {
            return Err(serde::de::Error::custom("exclusions require recurrence"));
        }
        Ok(Self {
            schema_version: raw.schema_version,
            id: raw.id,
            ownership: raw.ownership,
            title: raw.title,
            description: raw.description,
            event_type: raw.event_type,
            start: raw.start,
            end: raw.end,
            all_day: raw.all_day,
            time_zone: raw.time_zone,
            status: raw.status,
            location: raw.location,
            responsible_owner: raw.responsible_owner,
            participants: raw.participants,
            remarks: raw.remarks,
            linked_plan_id: raw.linked_plan_id,
            linked_task_id: raw.linked_task_id,
            linked_mission_requirement_id: raw.linked_mission_requirement_id,
            parent_activity_id: raw.parent_activity_id,
            recurrence: raw.recurrence,
            excluded_occurrence_starts: raw.excluded_occurrence_starts,
        })
    }
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BattleRhythmRevisionChunkV1 {
    pub schema_version: u8,
    pub revision_id: String,
    pub source_id: String,
    pub prior_revision_id: Option<String>,
    pub imported_at: String,
    pub chunk_index: u32,
    pub chunk_count: u32,
    pub manifest_hash: String,
    pub changes: Vec<serde_json::Value>,
}
