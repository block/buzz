//! Strict, opaque wire contract for an authoritative Daily Command Brief.

use std::collections::{BTreeMap, BTreeSet};

use chrono::DateTime;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::Value;

const MAX_TEXT_BYTES: usize = 4096;
const MAX_ARRAY_ITEMS: usize = 64;
const MAX_SOURCE_LEDGER_ITEMS: usize = 256;
const MAX_AGGREGATE_DISSENT_ITEMS: usize = 7 * MAX_ARRAY_ITEMS;
const ADVISORY_LIMITATION: &str = "This Daily Command Brief is advisory only. Navigation content identifies considerations and source limitations; it does not generate executable navigation orders or make navigational decisions.";

/// An exact, fully validated canonical `CommandBrief` JSON value.
///
/// The contained JSON is deliberately opaque. Construction and deserialization
/// validate the complete closed schema, source ledger, and every citation.
#[derive(Clone, Debug, PartialEq)]
pub struct CommandBriefWire(Value);

impl CommandBriefWire {
    /// Return the validated canonical JSON view for a trusted local conversion.
    pub fn as_value(&self) -> &Value {
        &self.0
    }

    /// Consume this wrapper and return the validated canonical JSON view.
    pub fn into_value(self) -> Value {
        self.0
    }
}

impl TryFrom<Value> for CommandBriefWire {
    type Error = ();

    fn try_from(value: Value) -> Result<Self, Self::Error> {
        let raw: RawCommandBrief = serde_json::from_value(value.clone()).map_err(|_| ())?;
        validate_command_brief(&raw)?;
        Ok(Self(value))
    }
}

impl Serialize for CommandBriefWire {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        self.0.serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for CommandBriefWire {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = Value::deserialize(deserializer)?;
        Self::try_from(value).map_err(|()| serde::de::Error::custom("invalid command brief"))
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd)]
enum OfficialClassification {
    #[serde(rename = "OFFICIAL")]
    Official,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd)]
#[serde(rename_all = "snake_case")]
enum Adviser {
    Operations,
    Intelligence,
    Logistics,
    Navigation,
    DailyRoutine,
    Reporting,
    Plans,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd)]
#[serde(rename_all = "snake_case")]
enum Section {
    Today,
    Operations,
    Intelligence,
    Logistics,
    Navigation,
    DailyRoutine,
    Reports,
    #[serde(rename = "planning_30_60_90")]
    Planning306090,
    Decisions,
    ConflictsAndGaps,
    Sources,
}

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
enum SourceKind {
    Rag,
    Memory,
    WorldMonitor,
    Calendar,
    Reminders,
    Notes,
    File,
}

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
enum ApprovalState {
    Pending,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RawQuotedLocation {
    quote: String,
    location: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RawSourceLedgerEntry {
    classification: OfficialClassification,
    ledger_id: String,
    source_id: String,
    source_kind: SourceKind,
    collection: String,
    document_id: String,
    chunk_id: String,
    timestamp: String,
    snapshot_id: String,
    quoted_location: RawQuotedLocation,
    retrieved_at: String,
    observed_at: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RawFinding {
    classification: OfficialClassification,
    text: String,
    source_ids: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RawProposal {
    classification: OfficialClassification,
    action_id: String,
    text: String,
    approval_state: ApprovalState,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RawContribution {
    classification: OfficialClassification,
    adviser: Adviser,
    section: Section,
    findings: Vec<RawFinding>,
    confidence: f64,
    limitations: Vec<String>,
    dissent: Vec<String>,
    proposed_actions: Vec<RawProposal>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RawFreshness {
    classification: OfficialClassification,
    as_of: String,
    stale_source_ids: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RawCommandBrief {
    version: u32,
    classification: OfficialClassification,
    generated_at: String,
    run_id: String,
    schedule_id: String,
    snapshot_id: String,
    sections: BTreeMap<Section, Vec<RawFinding>>,
    degraded_sections: Vec<Section>,
    missing_information: Vec<String>,
    dissent: Vec<String>,
    source_ledger: Vec<RawSourceLedgerEntry>,
    source_freshness: RawFreshness,
    contributions: Vec<RawContribution>,
    advisory_limitation: String,
}

fn validate_command_brief(raw: &RawCommandBrief) -> Result<(), ()> {
    let _ = raw.classification;
    if raw.version != 1
        || !valid_time(&raw.generated_at)
        || !valid_text(&raw.run_id)
        || !valid_text(&raw.schedule_id)
        || !valid_text(&raw.snapshot_id)
        || raw.advisory_limitation != ADVISORY_LIMITATION
        || raw.source_ledger.len() > MAX_SOURCE_LEDGER_ITEMS
        || raw.contributions.len() != 7
        || !valid_unique_sections(&raw.degraded_sections)
        || !valid_text_array(&raw.missing_information, MAX_ARRAY_ITEMS)
        || !valid_text_array(&raw.dissent, MAX_AGGREGATE_DISSENT_ITEMS)
    {
        return Err(());
    }

    let expected_sections = BTreeSet::from([
        Section::Today,
        Section::Operations,
        Section::Intelligence,
        Section::Logistics,
        Section::Navigation,
        Section::DailyRoutine,
        Section::Reports,
        Section::Planning306090,
        Section::Decisions,
        Section::ConflictsAndGaps,
        Section::Sources,
    ]);
    if raw.sections.keys().copied().collect::<BTreeSet<_>>() != expected_sections {
        return Err(());
    }

    let mut ledger_ids = BTreeSet::new();
    let mut source_ids = BTreeSet::new();
    for source in &raw.source_ledger {
        let _ = source.classification;
        let _ = source.source_kind;
        if !valid_text(&source.ledger_id)
            || !valid_text(&source.source_id)
            || !valid_text(&source.collection)
            || !valid_text(&source.document_id)
            || !valid_text(&source.chunk_id)
            || !valid_time(&source.timestamp)
            || source.snapshot_id != raw.snapshot_id
            || !valid_text(&source.quoted_location.quote)
            || !valid_text(&source.quoted_location.location)
            || !valid_time(&source.retrieved_at)
            || !valid_time(&source.observed_at)
            || !ledger_ids.insert(source.ledger_id.clone())
            || !source_ids.insert(source.source_id.clone())
        {
            return Err(());
        }
    }

    let _ = raw.source_freshness.classification;
    if !valid_time(&raw.source_freshness.as_of)
        || !valid_unique_text_array(&raw.source_freshness.stale_source_ids, MAX_ARRAY_ITEMS)
        || raw
            .source_freshness
            .stale_source_ids
            .iter()
            .any(|source_id| !ledger_ids.contains(source_id))
    {
        return Err(());
    }

    for findings in raw.sections.values() {
        validate_findings(findings, &ledger_ids)?;
    }

    let expected_advisers = BTreeSet::from([
        Adviser::Operations,
        Adviser::Intelligence,
        Adviser::Logistics,
        Adviser::Navigation,
        Adviser::DailyRoutine,
        Adviser::Reporting,
        Adviser::Plans,
    ]);
    let mut seen_advisers = BTreeSet::new();
    let mut aggregate_dissent = Vec::new();
    let mut specialist_findings = BTreeSet::new();
    for contribution in &raw.contributions {
        let _ = contribution.classification;
        if !seen_advisers.insert(contribution.adviser)
            || contribution.section != section_for_adviser(contribution.adviser)
            || !contribution.confidence.is_finite()
            || !(0.0..=1.0).contains(&contribution.confidence)
            || !valid_text_array(&contribution.limitations, MAX_ARRAY_ITEMS)
            || !valid_text_array(&contribution.dissent, MAX_ARRAY_ITEMS)
            || contribution.proposed_actions.len() > MAX_ARRAY_ITEMS
        {
            return Err(());
        }
        validate_findings(&contribution.findings, &ledger_ids)?;
        for proposal in &contribution.proposed_actions {
            let _ = proposal.classification;
            let _ = proposal.approval_state;
            if !valid_text(&proposal.action_id) || !valid_text(&proposal.text) {
                return Err(());
            }
        }
        aggregate_dissent.extend(contribution.dissent.iter().map(String::as_str));
        specialist_findings.extend(
            contribution
                .findings
                .iter()
                .map(|finding| (finding.text.as_str(), finding.source_ids.as_slice())),
        );
    }
    if seen_advisers != expected_advisers
        || aggregate_dissent != raw.dissent.iter().map(String::as_str).collect::<Vec<_>>()
    {
        return Err(());
    }
    if raw.sections.values().flatten().any(|finding| {
        !specialist_findings.contains(&(finding.text.as_str(), finding.source_ids.as_slice()))
    }) {
        return Err(());
    }
    Ok(())
}

fn section_for_adviser(adviser: Adviser) -> Section {
    match adviser {
        Adviser::Operations => Section::Operations,
        Adviser::Intelligence => Section::Intelligence,
        Adviser::Logistics => Section::Logistics,
        Adviser::Navigation => Section::Navigation,
        Adviser::DailyRoutine => Section::DailyRoutine,
        Adviser::Reporting => Section::Reports,
        Adviser::Plans => Section::Planning306090,
    }
}

fn validate_findings(findings: &[RawFinding], ledger_ids: &BTreeSet<String>) -> Result<(), ()> {
    if findings.len() > MAX_ARRAY_ITEMS {
        return Err(());
    }
    for finding in findings {
        let _ = finding.classification;
        if !valid_text(&finding.text)
            || finding.source_ids.is_empty()
            || !valid_unique_text_array(&finding.source_ids, MAX_ARRAY_ITEMS)
            || finding
                .source_ids
                .iter()
                .any(|source_id| !ledger_ids.contains(source_id))
        {
            return Err(());
        }
    }
    Ok(())
}

fn valid_text(value: &str) -> bool {
    !value.is_empty()
        && value.trim() == value
        && value.len() <= MAX_TEXT_BYTES
        && !value
            .chars()
            .any(|character| character.is_control() || character == '\u{7f}')
}

fn valid_time(value: &str) -> bool {
    valid_text(value) && DateTime::parse_from_rfc3339(value).is_ok()
}

fn valid_text_array(values: &[String], maximum: usize) -> bool {
    values.len() <= maximum && values.iter().all(|value| valid_text(value))
}

fn valid_unique_text_array(values: &[String], maximum: usize) -> bool {
    valid_text_array(values, maximum)
        && values.iter().collect::<BTreeSet<_>>().len() == values.len()
}

fn valid_unique_sections(values: &[Section]) -> bool {
    values.len() <= MAX_ARRAY_ITEMS && values.iter().collect::<BTreeSet<_>>().len() == values.len()
}
