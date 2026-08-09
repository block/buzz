use chrono::{DateTime, Duration, TimeZone, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WorldMonitorTool {
    CountryRisk,
    ConflictEvents,
    MilitaryPosture,
    NewsIntelligence,
    MaritimeActivity,
    ChokepointStatus,
    SupplyChainData,
}

impl WorldMonitorTool {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::CountryRisk => "get_country_risk",
            Self::ConflictEvents => "get_conflict_events",
            Self::MilitaryPosture => "get_military_posture",
            Self::NewsIntelligence => "get_news_intelligence",
            Self::MaritimeActivity => "get_maritime_activity",
            Self::ChokepointStatus => "get_chokepoint_status",
            Self::SupplyChainData => "get_supply_chain_data",
        }
    }

    const fn freshness_window(self) -> Duration {
        match self {
            Self::ConflictEvents
            | Self::MilitaryPosture
            | Self::NewsIntelligence
            | Self::MaritimeActivity => Duration::hours(24),
            Self::CountryRisk | Self::ChokepointStatus | Self::SupplyChainData => Duration::days(7),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct WorldMonitorRequest {
    pub tool: WorldMonitorTool,
    pub arguments: Value,
}

impl WorldMonitorRequest {
    pub fn new(tool: WorldMonitorTool, arguments: Value) -> Result<Self, WorldMonitorError> {
        validate_arguments(tool, &arguments)?;
        Ok(Self { tool, arguments })
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WorldMonitorFreshness {
    Fresh,
    Stale,
    Unknown,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct NormalizedWorldMonitorEvidence {
    pub tool: WorldMonitorTool,
    pub arguments: Value,
    pub payload: Value,
    pub retrieved_at: DateTime<Utc>,
    pub source_time: Option<DateTime<Utc>>,
    pub freshness: WorldMonitorFreshness,
}

impl NormalizedWorldMonitorEvidence {
    pub fn new(request: WorldMonitorRequest, payload: Value, retrieved_at: DateTime<Utc>) -> Self {
        let source_time = extract_source_time(&payload);
        let freshness = freshness(request.tool, source_time, retrieved_at);
        Self {
            tool: request.tool,
            arguments: request.arguments,
            payload,
            retrieved_at,
            source_time,
            freshness,
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum WorldMonitorError {
    #[error("invalid World Monitor arguments")]
    InvalidArguments,
}

fn validate_arguments(tool: WorldMonitorTool, arguments: &Value) -> Result<(), WorldMonitorError> {
    let object = arguments
        .as_object()
        .ok_or(WorldMonitorError::InvalidArguments)?;
    if object.contains_key("jmespath") {
        return Err(WorldMonitorError::InvalidArguments);
    }
    for key in object.keys() {
        if !allowed_keys(tool).contains(&key.as_str()) {
            return Err(WorldMonitorError::InvalidArguments);
        }
    }
    if let Some(country) = object.get("country_code") {
        let country = country
            .as_str()
            .filter(|value| value.len() == 2 && value.bytes().all(|byte| byte.is_ascii_uppercase()))
            .ok_or(WorldMonitorError::InvalidArguments)?;
        if country.is_empty() {
            return Err(WorldMonitorError::InvalidArguments);
        }
    }
    if let Some(limit) = object.get("limit") {
        if !matches!(limit.as_u64(), Some(1..=30)) {
            return Err(WorldMonitorError::InvalidArguments);
        }
    }
    if let Some(topic) = object.get("topic") {
        if !matches!(
            topic.as_str(),
            Some("conflict" | "economy" | "cyber" | "nuclear" | "intelligence" | "maritime")
        ) {
            return Err(WorldMonitorError::InvalidArguments);
        }
    }
    Ok(())
}

const fn allowed_keys(tool: WorldMonitorTool) -> &'static [&'static str] {
    match tool {
        WorldMonitorTool::CountryRisk => &["country_code"],
        WorldMonitorTool::ConflictEvents => {
            &["country_code", "limit", "days", "start_date", "end_date"]
        }
        WorldMonitorTool::MilitaryPosture => &["country_code", "region", "limit"],
        WorldMonitorTool::NewsIntelligence => &["country_code", "topic", "limit", "days"],
        WorldMonitorTool::MaritimeActivity => &["country_code", "region", "limit"],
        WorldMonitorTool::ChokepointStatus => &["chokepoint", "region", "limit"],
        WorldMonitorTool::SupplyChainData => &["country_code", "region", "limit"],
    }
}

fn freshness(
    tool: WorldMonitorTool,
    source_time: Option<DateTime<Utc>>,
    retrieved_at: DateTime<Utc>,
) -> WorldMonitorFreshness {
    let Some(source_time) = source_time else {
        return WorldMonitorFreshness::Unknown;
    };
    if source_time.timestamp() <= 0 || source_time > retrieved_at + Duration::minutes(5) {
        return WorldMonitorFreshness::Unknown;
    }
    if retrieved_at - source_time <= tool.freshness_window() {
        WorldMonitorFreshness::Fresh
    } else {
        WorldMonitorFreshness::Stale
    }
}

fn extract_source_time(value: &Value) -> Option<DateTime<Utc>> {
    match value {
        Value::Object(object) => extract_object_time(object)
            .or_else(|| object.values().filter_map(extract_source_time).max()),
        Value::Array(values) => values.iter().filter_map(extract_source_time).max(),
        _ => None,
    }
}

fn extract_object_time(object: &Map<String, Value>) -> Option<DateTime<Utc>> {
    const TIME_KEYS: &[&str] = &[
        "source_time",
        "sourceTime",
        "timestamp",
        "updated_at",
        "updatedAt",
        "published_at",
        "publishedAt",
        "date",
    ];
    TIME_KEYS
        .iter()
        .filter_map(|key| object.get(*key))
        .filter_map(parse_time)
        .max()
}

fn parse_time(value: &Value) -> Option<DateTime<Utc>> {
    if let Some(text) = value.as_str() {
        return DateTime::parse_from_rfc3339(text)
            .ok()
            .map(|time| time.with_timezone(&Utc));
    }
    let number = value.as_i64()?;
    let seconds = if number.abs() >= 100_000_000_000 {
        number.checked_div(1_000)?
    } else {
        number
    };
    Utc.timestamp_opt(seconds, 0).single()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn exact_tool_names_are_pinned() {
        assert_eq!(WorldMonitorTool::CountryRisk.as_str(), "get_country_risk");
        assert_eq!(
            WorldMonitorTool::ConflictEvents.as_str(),
            "get_conflict_events"
        );
        assert_eq!(
            WorldMonitorTool::MilitaryPosture.as_str(),
            "get_military_posture"
        );
        assert_eq!(
            WorldMonitorTool::NewsIntelligence.as_str(),
            "get_news_intelligence"
        );
        assert_eq!(
            WorldMonitorTool::MaritimeActivity.as_str(),
            "get_maritime_activity"
        );
        assert_eq!(
            WorldMonitorTool::ChokepointStatus.as_str(),
            "get_chokepoint_status"
        );
        assert_eq!(
            WorldMonitorTool::SupplyChainData.as_str(),
            "get_supply_chain_data"
        );
    }

    #[test]
    fn validates_application_generated_arguments() {
        assert!(WorldMonitorRequest::new(
            WorldMonitorTool::NewsIntelligence,
            json!({"country_code":"PH","topic":"maritime","limit":30})
        )
        .is_ok());
        for invalid in [
            json!({"country_code":"ph"}),
            json!({"country_code":"PHL"}),
            json!({"limit":0}),
            json!({"limit":31}),
            json!({"topic":"politics"}),
            json!({"jmespath":"secrets"}),
            json!({"unexpected":true}),
        ] {
            assert!(WorldMonitorRequest::new(WorldMonitorTool::NewsIntelligence, invalid).is_err());
        }
    }

    #[test]
    fn derives_tactical_and_strategic_freshness() {
        let now = "2026-07-28T00:00:00Z".parse().expect("time");
        let tactical = WorldMonitorRequest::new(
            WorldMonitorTool::NewsIntelligence,
            json!({"country_code":"PH"}),
        )
        .expect("request");
        assert_eq!(
            NormalizedWorldMonitorEvidence::new(
                tactical.clone(),
                json!({"timestamp":"2026-07-27T12:00:00Z"}),
                now
            )
            .freshness,
            WorldMonitorFreshness::Fresh
        );
        assert_eq!(
            NormalizedWorldMonitorEvidence::new(
                tactical,
                json!({"timestamp":"2026-07-26T12:00:00Z"}),
                now
            )
            .freshness,
            WorldMonitorFreshness::Stale
        );
        let strategic =
            WorldMonitorRequest::new(WorldMonitorTool::CountryRisk, json!({"country_code":"PH"}))
                .expect("request");
        assert_eq!(
            NormalizedWorldMonitorEvidence::new(
                strategic.clone(),
                json!({"timestamp": now.timestamp_millis()}),
                now
            )
            .freshness,
            WorldMonitorFreshness::Fresh
        );
        for payload in [
            json!({}),
            json!({"timestamp":0}),
            json!({"timestamp":"not-a-time"}),
            json!({"timestamp":"2026-07-28T00:06:00Z"}),
        ] {
            assert_eq!(
                NormalizedWorldMonitorEvidence::new(strategic.clone(), payload, now).freshness,
                WorldMonitorFreshness::Unknown
            );
        }
    }
}
