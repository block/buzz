use chrono::DateTime;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

mod status;

pub use status::BriefRunStatus;

/// The only permitted classification for Daily Command Brief material.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize)]
pub enum Classification {
    #[serde(rename = "OFFICIAL")]
    Official,
}

/// The fixed adviser identities used by the trusted brief pipeline.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AdviserId {
    ChiefOfStaff,
    Operations,
    Navigation,
    DailyRoutine,
    Reporting,
    Plans,
}

/// The complete, closed Daily Command Brief section vocabulary.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum BriefSection {
    Today,
    Operations,
    Navigation,
    DailyRoutine,
    Reports,
    #[serde(rename = "planning_30_60_90")]
    Planning306090,
    Decisions,
    ConflictsAndGaps,
    Sources,
}

/// The admitted source kinds that may enter the frozen source ledger.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceKind {
    Rag,
    Memory,
    Calendar,
    Reminders,
    Notes,
    File,
}

/// The run lifecycle states shared by orchestration, persistence, and display.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum BriefRunState {
    Queued,
    CollectingSources,
    RunningSpecialists,
    Consolidating,
    Persisting,
    Completed,
    Degraded,
    Cancelled,
    Failed,
}

/// The local publication states permitted after a signed lifecycle event exists.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PublicationState {
    Queued,
    Published,
}

/// The mandatory final limitation shown on every Daily Command Brief.
pub const ADVISORY_LIMITATION: &str = "This Daily Command Brief is advisory only. Navigation content identifies considerations and source limitations; it does not generate executable navigation orders or make navigational decisions.";
/// Maximum UTF-8 byte length of a text field in the canonical contract.
pub const MAX_TEXT_BYTES: usize = 4096;
/// Maximum count for general contract arrays.
pub const MAX_ARRAY_ITEMS: usize = 64;
/// Number of specialist contributions required by one complete brief.
pub const SPECIALIST_COUNT: usize = 5;
/// Exact specialist identities required by a complete brief and Chief input.
pub const SPECIALIST_ADVISERS: [AdviserId; SPECIALIST_COUNT] = [
    AdviserId::Operations,
    AdviserId::Navigation,
    AdviserId::DailyRoutine,
    AdviserId::Reporting,
    AdviserId::Plans,
];
/// Maximum final dissent retained across all five specialist contributions.
pub const MAX_AGGREGATE_DISSENT_ITEMS: usize = SPECIALIST_COUNT * MAX_ARRAY_ITEMS;
/// Maximum sources admitted to one frozen run ledger.
pub const MAX_SOURCE_LEDGER_ITEMS: usize = 256;

/// A validation error for untrusted Daily Command Brief wire data.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ContractError;

impl fmt::Display for ContractError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("invalid command brief contract")
    }
}

impl std::error::Error for ContractError {}

/// The preserved location of an inert source quote.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QuotedLocation {
    quote: String,
    location: String,
}

/// A stable source-ledger entry, including retrieval and observation provenance.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceLedgerEntry {
    classification: Classification,
    ledger_id: String,
    source_id: String,
    source_kind: SourceKind,
    collection: String,
    document_id: String,
    chunk_id: String,
    timestamp: String,
    snapshot_id: String,
    quoted_location: QuotedLocation,
    retrieved_at: String,
    observed_at: String,
}

impl SourceLedgerEntry {
    /// Parses one untrusted source into the same OFFICIAL, bounded source
    /// contract used by a complete brief.
    pub fn parse_for_snapshot(
        value: Value,
        expected_snapshot_id: &str,
    ) -> Result<Self, ContractError> {
        let raw: RawSourceLedgerEntry = serde_json::from_value(value).map_err(|_| ContractError)?;
        parse_raw_source(raw, expected_snapshot_id)
    }

    /// Returns the stable run-ledger identity.
    pub fn ledger_id(&self) -> &str {
        &self.ledger_id
    }

    /// Returns the unique upstream source identity.
    pub fn source_id(&self) -> &str {
        &self.source_id
    }

    /// Returns the closed source-kind classification.
    pub const fn source_kind(&self) -> SourceKind {
        self.source_kind
    }

    /// Returns the trusted collection identity.
    pub fn collection(&self) -> &str {
        &self.collection
    }

    /// Returns the trusted document identity.
    pub fn document_id(&self) -> &str {
        &self.document_id
    }

    /// Returns the trusted chunk identity.
    pub fn chunk_id(&self) -> &str {
        &self.chunk_id
    }

    /// Returns the validated frozen snapshot identity.
    pub fn snapshot_id(&self) -> &str {
        &self.snapshot_id
    }

    /// Returns the inert bounded source quote.
    pub fn quote(&self) -> &str {
        &self.quoted_location.quote
    }

    /// Returns the bounded source location.
    pub fn location(&self) -> &str {
        &self.quoted_location.location
    }

    /// Returns the validated retrieval timestamp.
    pub fn retrieved_at(&self) -> &str {
        &self.retrieved_at
    }

    /// Returns the validated observation timestamp.
    pub fn observed_at(&self) -> &str {
        &self.observed_at
    }
}

/// A factual finding cited by stable source-ledger IDs.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CitedFinding {
    classification: Classification,
    text: String,
    source_ids: Vec<String>,
}

/// A proposal which must remain pending until an explicit external approval flow.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PendingProposal {
    classification: Classification,
    action_id: String,
    text: String,
    approval_state: PendingApprovalState,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
enum PendingApprovalState {
    Pending,
}

/// Validated structured output from one specialist adviser.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AdviserContribution {
    classification: Classification,
    adviser: AdviserId,
    section: BriefSection,
    findings: Vec<CitedFinding>,
    confidence: f64,
    limitations: Vec<String>,
    dissent: Vec<String>,
    proposed_actions: Vec<PendingProposal>,
}

impl CitedFinding {
    /// Parses one untrusted finding against the exact run-ledger IDs.
    pub fn parse_for_ledger(
        value: Value,
        run_ledger_ids: &BTreeSet<String>,
    ) -> Result<Self, ContractError> {
        let raw: RawCitedFinding = serde_json::from_value(value).map_err(|_| ContractError)?;
        let mut findings = parse_raw_findings(vec![raw], run_ledger_ids)?;
        findings.pop().ok_or(ContractError)
    }

    /// Returns the validated factual text.
    pub fn text(&self) -> &str {
        &self.text
    }

    /// Returns the sorted, unique run-ledger citations.
    pub fn source_ids(&self) -> &[String] {
        &self.source_ids
    }
}

impl AdviserContribution {
    /// Parses one untrusted specialist terminal message against the canonical
    /// contribution contract and the exact run-ledger IDs visible to the model.
    pub fn parse_for_adviser(
        value: Value,
        expected_adviser: AdviserId,
        run_ledger_ids: &BTreeSet<String>,
    ) -> Result<Self, ContractError> {
        let raw: RawAdviserContribution =
            serde_json::from_value(value).map_err(|_| ContractError)?;
        parse_raw_contribution(raw, expected_adviser, run_ledger_ids)
    }

    /// Returns the validated specialist identity.
    pub const fn adviser(&self) -> AdviserId {
        self.adviser
    }

    /// Returns the one section assigned to this specialist.
    pub const fn section(&self) -> BriefSection {
        self.section
    }

    /// Returns the validated ledger-cited findings.
    pub fn findings(&self) -> &[CitedFinding] {
        &self.findings
    }

    /// Returns bounded limitations preserved from the specialist.
    pub fn limitations(&self) -> &[String] {
        &self.limitations
    }

    /// Returns bounded dissent preserved from the specialist.
    pub fn dissent(&self) -> &[String] {
        &self.dissent
    }
}

/// Freshness assessment referencing only entries in the source ledger.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceFreshness {
    classification: Classification,
    as_of: String,
    stale_source_ids: Vec<String>,
}

/// The trusted, validated canonical Daily Command Brief.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CommandBrief {
    version: u32,
    classification: Classification,
    generated_at: String,
    run_id: String,
    schedule_id: String,
    snapshot_id: String,
    sections: BTreeMap<BriefSection, Vec<CitedFinding>>,
    degraded_sections: Vec<BriefSection>,
    missing_information: Vec<String>,
    dissent: Vec<String>,
    source_ledger: Vec<SourceLedgerEntry>,
    source_freshness: SourceFreshness,
    contributions: Vec<AdviserContribution>,
    advisory_limitation: String,
}

/// A post-signing wrapper; its event ID is deliberately absent from `CommandBrief`.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PublishedCommandBrief {
    classification: Classification,
    brief: CommandBrief,
    lifecycle_audit_event_id: String,
    publication_state: PublicationState,
}

/// The persisted local Daily Command Brief schedule contract.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BriefSchedule {
    classification: Classification,
    schedule_id: String,
    enabled: bool,
    local_time: String,
    timezone: String,
    catch_up_same_day: bool,
    concurrency: u8,
}

/// One append-only lifecycle transition associated with a run.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BriefLifecycleRecord {
    classification: Classification,
    run_id: String,
    schedule_id: String,
    state: BriefRunState,
    occurred_at: String,
    snapshot_id: String,
    previous_lifecycle_audit_event_id: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RawQuotedLocation {
    quote: String,
    location: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RawSourceLedgerEntry {
    classification: Classification,
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

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RawCitedFinding {
    classification: Classification,
    text: String,
    source_ids: Vec<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RawPendingProposal {
    classification: Classification,
    action_id: String,
    text: String,
    approval_state: PendingApprovalState,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RawAdviserContribution {
    classification: Classification,
    adviser: AdviserId,
    section: BriefSection,
    findings: Vec<RawCitedFinding>,
    confidence: f64,
    limitations: Vec<String>,
    dissent: Vec<String>,
    proposed_actions: Vec<RawPendingProposal>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RawSourceFreshness {
    classification: Classification,
    as_of: String,
    stale_source_ids: Vec<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RawCommandBrief {
    version: u32,
    classification: Classification,
    generated_at: String,
    run_id: String,
    schedule_id: String,
    snapshot_id: String,
    sections: BTreeMap<BriefSection, Vec<RawCitedFinding>>,
    degraded_sections: Vec<BriefSection>,
    missing_information: Vec<String>,
    dissent: Vec<String>,
    source_ledger: Vec<RawSourceLedgerEntry>,
    source_freshness: RawSourceFreshness,
    contributions: Vec<RawAdviserContribution>,
    advisory_limitation: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RawPublishedCommandBrief {
    classification: Classification,
    brief: Value,
    lifecycle_audit_event_id: String,
    publication_state: PublicationState,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RawBriefSchedule {
    classification: Classification,
    schedule_id: String,
    enabled: bool,
    local_time: String,
    timezone: String,
    catch_up_same_day: bool,
    concurrency: u8,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RawBriefLifecycleRecord {
    classification: Classification,
    run_id: String,
    schedule_id: String,
    state: BriefRunState,
    occurred_at: String,
    snapshot_id: String,
    previous_lifecycle_audit_event_id: Option<String>,
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

fn parse_raw_source(
    source: RawSourceLedgerEntry,
    expected_snapshot_id: &str,
) -> Result<SourceLedgerEntry, ContractError> {
    if !valid_text(&source.ledger_id)
        || !valid_text(&source.source_id)
        || !valid_text(&source.collection)
        || !valid_text(&source.document_id)
        || !valid_text(&source.chunk_id)
        || !valid_time(&source.timestamp)
        || !valid_text(&source.snapshot_id)
        || source.snapshot_id != expected_snapshot_id
        || !valid_text(&source.quoted_location.quote)
        || !valid_text(&source.quoted_location.location)
        || !valid_time(&source.retrieved_at)
        || !valid_time(&source.observed_at)
    {
        return Err(ContractError);
    }
    Ok(SourceLedgerEntry {
        classification: source.classification,
        ledger_id: source.ledger_id,
        source_id: source.source_id,
        source_kind: source.source_kind,
        collection: source.collection,
        document_id: source.document_id,
        chunk_id: source.chunk_id,
        timestamp: source.timestamp,
        snapshot_id: source.snapshot_id,
        quoted_location: QuotedLocation {
            quote: source.quoted_location.quote,
            location: source.quoted_location.location,
        },
        retrieved_at: source.retrieved_at,
        observed_at: source.observed_at,
    })
}

fn valid_text_array(values: &[String]) -> bool {
    valid_text_array_with_limit(values, MAX_ARRAY_ITEMS)
}

fn valid_text_array_with_limit(values: &[String], maximum_items: usize) -> bool {
    values.len() <= maximum_items && values.iter().all(|value| valid_text(value))
}

fn unique_valid_text_array(values: &[String]) -> bool {
    valid_text_array(values) && values.iter().collect::<BTreeSet<_>>().len() == values.len()
}

fn valid_sections(sections: &BTreeMap<BriefSection, Vec<CitedFinding>>) -> bool {
    let expected = BTreeSet::from([
        BriefSection::Today,
        BriefSection::Operations,
        BriefSection::Navigation,
        BriefSection::DailyRoutine,
        BriefSection::Reports,
        BriefSection::Planning306090,
        BriefSection::Decisions,
        BriefSection::ConflictsAndGaps,
        BriefSection::Sources,
    ]);
    sections.keys().copied().collect::<BTreeSet<_>>() == expected
        && sections
            .values()
            .all(|findings| findings.len() <= MAX_ARRAY_ITEMS)
}

fn parse_raw_findings(
    raw_findings: Vec<RawCitedFinding>,
    ledger_ids: &BTreeSet<String>,
) -> Result<Vec<CitedFinding>, ContractError> {
    if raw_findings.len() > MAX_ARRAY_ITEMS {
        return Err(ContractError);
    }
    raw_findings
        .into_iter()
        .map(|finding| {
            if !valid_text(&finding.text)
                || finding.source_ids.is_empty()
                || !unique_valid_text_array(&finding.source_ids)
                || finding
                    .source_ids
                    .iter()
                    .any(|source_id| !ledger_ids.contains(source_id))
            {
                return Err(ContractError);
            }
            let mut source_ids = finding.source_ids;
            source_ids.sort();
            Ok(CitedFinding {
                classification: finding.classification,
                text: finding.text,
                source_ids,
            })
        })
        .collect()
}

fn parse_raw_contribution(
    contribution: RawAdviserContribution,
    expected_adviser: AdviserId,
    ledger_ids: &BTreeSet<String>,
) -> Result<AdviserContribution, ContractError> {
    if contribution.adviser != expected_adviser
        || expected_adviser == AdviserId::ChiefOfStaff
        || !contribution.confidence.is_finite()
        || !(0.0..=1.0).contains(&contribution.confidence)
        || !valid_text_array(&contribution.limitations)
        || !valid_text_array(&contribution.dissent)
        || contribution.proposed_actions.len() > MAX_ARRAY_ITEMS
    {
        return Err(ContractError);
    }
    let expected_section = match contribution.adviser {
        AdviserId::Operations => BriefSection::Operations,
        AdviserId::Navigation => BriefSection::Navigation,
        AdviserId::DailyRoutine => BriefSection::DailyRoutine,
        AdviserId::Reporting => BriefSection::Reports,
        AdviserId::Plans => BriefSection::Planning306090,
        AdviserId::ChiefOfStaff => return Err(ContractError),
    };
    if contribution.section != expected_section {
        return Err(ContractError);
    }
    let proposed_actions = contribution
        .proposed_actions
        .into_iter()
        .map(|proposal| {
            if !valid_text(&proposal.action_id) || !valid_text(&proposal.text) {
                return Err(ContractError);
            }
            Ok(PendingProposal {
                classification: proposal.classification,
                action_id: proposal.action_id,
                text: proposal.text,
                approval_state: proposal.approval_state,
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(AdviserContribution {
        classification: contribution.classification,
        adviser: contribution.adviser,
        section: contribution.section,
        findings: parse_raw_findings(contribution.findings, ledger_ids)?,
        confidence: contribution.confidence,
        limitations: contribution.limitations,
        dissent: contribution.dissent,
        proposed_actions,
    })
}

impl TryFrom<Value> for CommandBrief {
    type Error = ContractError;

    fn try_from(value: Value) -> Result<Self, Self::Error> {
        let raw: RawCommandBrief = serde_json::from_value(value).map_err(|_| ContractError)?;
        if raw.version != 1
            || !valid_time(&raw.generated_at)
            || !valid_text(&raw.run_id)
            || !valid_text(&raw.schedule_id)
            || !valid_text(&raw.snapshot_id)
            || raw.advisory_limitation != ADVISORY_LIMITATION
            || raw.source_ledger.len() > MAX_SOURCE_LEDGER_ITEMS
            || raw.contributions.len() != SPECIALIST_COUNT
            || raw.degraded_sections.len() > MAX_ARRAY_ITEMS
            || raw.degraded_sections.iter().collect::<BTreeSet<_>>().len()
                != raw.degraded_sections.len()
            || !valid_text_array(&raw.missing_information)
            || !valid_text_array_with_limit(&raw.dissent, MAX_AGGREGATE_DISSENT_ITEMS)
        {
            return Err(ContractError);
        }

        let mut ledger_ids = BTreeSet::new();
        let mut source_ids = BTreeSet::new();
        let mut source_ledger = Vec::with_capacity(raw.source_ledger.len());
        for source in raw.source_ledger {
            let source = parse_raw_source(source, &raw.snapshot_id)?;
            if !ledger_ids.insert(source.ledger_id.clone())
                || !source_ids.insert(source.source_id.clone())
            {
                return Err(ContractError);
            }
            source_ledger.push(source);
        }

        if !valid_time(&raw.source_freshness.as_of)
            || !unique_valid_text_array(&raw.source_freshness.stale_source_ids)
            || raw
                .source_freshness
                .stale_source_ids
                .iter()
                .any(|source_id| !ledger_ids.contains(source_id))
        {
            return Err(ContractError);
        }

        let mut sections = BTreeMap::new();
        for (section, findings) in raw.sections {
            sections.insert(section, parse_raw_findings(findings, &ledger_ids)?);
        }
        if !valid_sections(&sections) {
            return Err(ContractError);
        }

        let expected_specialists = BTreeSet::from(SPECIALIST_ADVISERS);
        let mut seen_specialists = BTreeSet::new();
        let mut contributions = Vec::with_capacity(raw.contributions.len());
        for contribution in raw.contributions {
            if !expected_specialists.contains(&contribution.adviser)
                || !seen_specialists.insert(contribution.adviser)
            {
                return Err(ContractError);
            }
            let adviser = contribution.adviser;
            contributions.push(parse_raw_contribution(contribution, adviser, &ledger_ids)?);
        }
        if seen_specialists != expected_specialists {
            return Err(ContractError);
        }
        if !contributions
            .iter()
            .flat_map(|contribution| contribution.dissent.iter())
            .eq(raw.dissent.iter())
        {
            return Err(ContractError);
        }
        let specialist_findings = contributions
            .iter()
            .flat_map(|contribution| contribution.findings.iter())
            .map(|finding| (finding.text.as_str(), finding.source_ids.as_slice()))
            .collect::<BTreeSet<_>>();
        if sections.values().flatten().any(|finding| {
            !specialist_findings.contains(&(finding.text.as_str(), finding.source_ids.as_slice()))
        }) {
            return Err(ContractError);
        }

        Ok(Self {
            version: raw.version,
            classification: raw.classification,
            generated_at: raw.generated_at,
            run_id: raw.run_id,
            schedule_id: raw.schedule_id,
            snapshot_id: raw.snapshot_id,
            sections,
            degraded_sections: raw.degraded_sections,
            missing_information: raw.missing_information,
            dissent: raw.dissent,
            source_ledger,
            source_freshness: SourceFreshness {
                classification: raw.source_freshness.classification,
                as_of: raw.source_freshness.as_of,
                stale_source_ids: raw.source_freshness.stale_source_ids,
            },
            contributions,
            advisory_limitation: raw.advisory_limitation,
        })
    }
}

impl CommandBrief {
    /// Return the validated run identity.
    pub fn run_id(&self) -> &str {
        &self.run_id
    }

    /// Return the validated schedule identity.
    pub fn schedule_id(&self) -> &str {
        &self.schedule_id
    }

    /// Return the frozen signed snapshot identity.
    pub fn snapshot_id(&self) -> &str {
        &self.snapshot_id
    }

    /// Return the trusted generation timestamp.
    pub fn generated_at(&self) -> &str {
        &self.generated_at
    }

    /// Whether the completed result contains visible degradation.
    pub fn is_degraded(&self) -> bool {
        !self.degraded_sections.is_empty() || !self.missing_information.is_empty()
    }
}

impl PublishedCommandBrief {
    /// Construct the post-signing envelope after the event ID is fixed.
    pub(crate) fn new(
        brief: CommandBrief,
        lifecycle_audit_event_id: String,
        publication_state: PublicationState,
    ) -> Self {
        Self {
            classification: Classification::Official,
            brief,
            lifecycle_audit_event_id,
            publication_state,
        }
    }

    /// Return the immutable validated final brief.
    pub fn brief(&self) -> &CommandBrief {
        &self.brief
    }

    /// Return the signed audit event ID.
    pub fn lifecycle_audit_event_id(&self) -> &str {
        &self.lifecycle_audit_event_id
    }

    /// Return the current local publication state.
    pub const fn publication_state(&self) -> PublicationState {
        self.publication_state
    }
}

impl TryFrom<Value> for PublishedCommandBrief {
    type Error = ContractError;

    fn try_from(value: Value) -> Result<Self, Self::Error> {
        let raw: RawPublishedCommandBrief =
            serde_json::from_value(value).map_err(|_| ContractError)?;
        if !valid_text(&raw.lifecycle_audit_event_id) {
            return Err(ContractError);
        }
        Ok(Self {
            classification: raw.classification,
            brief: CommandBrief::try_from(raw.brief)?,
            lifecycle_audit_event_id: raw.lifecycle_audit_event_id,
            publication_state: raw.publication_state,
        })
    }
}

impl TryFrom<Value> for BriefSchedule {
    type Error = ContractError;

    fn try_from(value: Value) -> Result<Self, Self::Error> {
        let raw: RawBriefSchedule = serde_json::from_value(value).map_err(|_| ContractError)?;
        let time = raw.local_time.as_bytes();
        let valid_local_time = time.len() == 5
            && time[0].is_ascii_digit()
            && time[1].is_ascii_digit()
            && time[2] == b':'
            && time[3].is_ascii_digit()
            && time[4].is_ascii_digit()
            && (time[0] - b'0') * 10 + (time[1] - b'0') < 24
            && (time[3] - b'0') * 10 + (time[4] - b'0') < 60;
        if !valid_text(&raw.schedule_id)
            || !valid_local_time
            || !valid_text(&raw.timezone)
            || !matches!(raw.concurrency, 1 | 2)
        {
            return Err(ContractError);
        }
        Ok(Self {
            classification: raw.classification,
            schedule_id: raw.schedule_id,
            enabled: raw.enabled,
            local_time: raw.local_time,
            timezone: raw.timezone,
            catch_up_same_day: raw.catch_up_same_day,
            concurrency: raw.concurrency,
        })
    }
}

impl BriefSchedule {
    /// Return the fixed trusted schedule identity.
    pub fn schedule_id(&self) -> &str {
        &self.schedule_id
    }

    /// Return whether scheduled generation is enabled.
    pub const fn enabled(&self) -> bool {
        self.enabled
    }

    /// Return the configured local wall time as `HH:MM`.
    pub fn local_time(&self) -> &str {
        &self.local_time
    }

    /// Return the configured IANA timezone name.
    pub fn timezone(&self) -> &str {
        &self.timezone
    }

    /// Return whether startup and wake may perform a same-day catch-up.
    pub const fn catch_up_same_day(&self) -> bool {
        self.catch_up_same_day
    }

    /// Return the bounded local-model concurrency.
    pub const fn concurrency(&self) -> u8 {
        self.concurrency
    }
}

impl TryFrom<Value> for BriefLifecycleRecord {
    type Error = ContractError;

    fn try_from(value: Value) -> Result<Self, Self::Error> {
        let raw: RawBriefLifecycleRecord =
            serde_json::from_value(value).map_err(|_| ContractError)?;
        if !valid_text(&raw.run_id)
            || !valid_text(&raw.schedule_id)
            || !valid_time(&raw.occurred_at)
            || !valid_text(&raw.snapshot_id)
            || raw
                .previous_lifecycle_audit_event_id
                .as_deref()
                .is_some_and(|event_id| !valid_text(event_id))
        {
            return Err(ContractError);
        }
        Ok(Self {
            classification: raw.classification,
            run_id: raw.run_id,
            schedule_id: raw.schedule_id,
            state: raw.state,
            occurred_at: raw.occurred_at,
            snapshot_id: raw.snapshot_id,
            previous_lifecycle_audit_event_id: raw.previous_lifecycle_audit_event_id,
        })
    }
}
