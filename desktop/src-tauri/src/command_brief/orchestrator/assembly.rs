use super::*;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct RawChiefOutput {
    classification: Classification,
    adviser: AdviserId,
    findings: Vec<Value>,
    limitations: Vec<String>,
    dissent: Vec<String>,
}

pub(super) struct ValidatedChief {
    findings: Vec<CitedFinding>,
}

pub(super) struct AssemblyLimitations<'a> {
    pub(super) degraded: &'a [BriefSection],
    pub(super) failed_advisers: &'a [AdviserId],
    pub(super) runtime: &'a [String],
}

pub(super) fn deterministic_chief(contributions: &[AdviserContribution]) -> ValidatedChief {
    let mut seen = BTreeSet::new();
    let findings = contributions
        .iter()
        .flat_map(|contribution| contribution.findings().iter().cloned())
        .filter(|finding| seen.insert((finding.text().to_string(), finding.source_ids().to_vec())))
        .take(MAX_ARRAY_ITEMS)
        .collect();
    ValidatedChief { findings }
}

pub(super) fn validate_chief_output(
    value: Value,
    contributions: &[AdviserContribution],
    source_limitations: &[String],
    ledger_ids: &BTreeSet<String>,
) -> Result<ValidatedChief, ()> {
    let raw: RawChiefOutput = serde_json::from_value(value).map_err(|_| ())?;
    if raw.classification != Classification::Official
        || raw.adviser != AdviserId::ChiefOfStaff
        || raw.findings.len() > MAX_ARRAY_ITEMS
        || !valid_text_array(&raw.limitations, MAX_ARRAY_ITEMS)
        || !valid_text_array(&raw.dissent, MAX_AGGREGATE_DISSENT_ITEMS)
    {
        return Err(());
    }
    let expected_dissent = contributions
        .iter()
        .flat_map(|contribution| contribution.dissent().iter().cloned())
        .collect::<Vec<_>>();
    if raw.dissent != expected_dissent {
        return Err(());
    }
    let allowed_limitations = source_limitations
        .iter()
        .chain(
            contributions
                .iter()
                .flat_map(|contribution| contribution.limitations()),
        )
        .collect::<BTreeSet<_>>();
    let mut seen_limitations = BTreeSet::new();
    if raw.limitations.iter().any(|limitation| {
        !allowed_limitations.contains(limitation) || !seen_limitations.insert(limitation)
    }) {
        return Err(());
    }
    let allowed = contributions
        .iter()
        .flat_map(|contribution| contribution.findings())
        .map(|finding| (finding.text().to_string(), finding.source_ids().to_vec()))
        .collect::<BTreeSet<_>>();
    let mut seen = BTreeSet::new();
    let mut findings = Vec::with_capacity(raw.findings.len());
    for value in raw.findings {
        let finding = CitedFinding::parse_for_ledger(value, ledger_ids).map_err(|_| ())?;
        let identity = (finding.text().to_string(), finding.source_ids().to_vec());
        if !allowed.contains(&identity) || !seen.insert(identity) {
            return Err(());
        }
        findings.push(finding);
    }
    Ok(ValidatedChief { findings })
}

pub(super) fn assemble_brief(
    run_id: &str,
    request: &CommandBriefRequest,
    context: &FrozenSourceContext,
    contributions: Vec<AdviserContribution>,
    chief: ValidatedChief,
    limitations: AssemblyLimitations<'_>,
) -> Result<CommandBrief, ()> {
    let mut sections = BTreeMap::<BriefSection, Vec<CitedFinding>>::from([
        (BriefSection::Today, chief.findings),
        (BriefSection::Operations, Vec::new()),
        (BriefSection::Navigation, Vec::new()),
        (BriefSection::DailyRoutine, Vec::new()),
        (BriefSection::Reports, Vec::new()),
        (BriefSection::Planning306090, Vec::new()),
        (BriefSection::Decisions, Vec::new()),
        (BriefSection::ConflictsAndGaps, Vec::new()),
        (BriefSection::Sources, Vec::new()),
    ]);
    for contribution in &contributions {
        sections.insert(contribution.section(), contribution.findings().to_vec());
    }
    let dissent = contributions
        .iter()
        .flat_map(|contribution| contribution.dissent().iter().cloned())
        .collect::<Vec<_>>();
    let missing_information = bounded_missing_information(
        limitations.failed_advisers,
        &contributions,
        context
            .limitations()
            .iter()
            .chain(limitations.runtime)
            .cloned()
            .collect::<Vec<_>>()
            .as_slice(),
    );
    let value = json!({
        "version": 1,
        "classification": "OFFICIAL",
        "generatedAt": timestamp(),
        "runId": run_id,
        "scheduleId": request.schedule_id,
        "snapshotId": context.snapshot_id(),
        "sections": sections,
        "degradedSections": limitations.degraded,
        "missingInformation": missing_information,
        "dissent": dissent,
        "sourceLedger": context.ledger(),
        "sourceFreshness": {
            "classification": "OFFICIAL",
            "asOf": context.observed_at(),
            "staleSourceIds": []
        },
        "contributions": contributions,
        "advisoryLimitation": crate::command_brief::types::ADVISORY_LIMITATION
    });
    CommandBrief::try_from(value).map_err(|_| ())
}

pub(super) fn limitation_only_contribution(
    adviser: AdviserId,
    limitation: &str,
    ledger_ids: &BTreeSet<String>,
) -> Result<AdviserContribution, ()> {
    AdviserContribution::parse_for_adviser(
        json!({
            "classification": "OFFICIAL",
            "adviser": adviser,
            "section": section_for_adviser(adviser),
            "findings": [],
            "confidence": 0.0,
            "limitations": [limitation],
            "dissent": [],
            "proposedActions": []
        }),
        adviser,
        ledger_ids,
    )
    .map_err(|_| ())
}

pub(super) fn section_for_adviser(adviser: AdviserId) -> BriefSection {
    match adviser {
        AdviserId::Operations => BriefSection::Operations,
        AdviserId::Navigation => BriefSection::Navigation,
        AdviserId::DailyRoutine => BriefSection::DailyRoutine,
        AdviserId::Reporting => BriefSection::Reports,
        AdviserId::Plans => BriefSection::Planning306090,
        AdviserId::ChiefOfStaff => BriefSection::ConflictsAndGaps,
    }
}

pub(super) fn adviser_unavailable(adviser: AdviserId) -> String {
    format!(
        "{} adviser output was unavailable.",
        adviser_display(adviser)
    )
}

pub(super) fn adviser_display(adviser: AdviserId) -> &'static str {
    match adviser {
        AdviserId::ChiefOfStaff => "Chief of Staff",
        AdviserId::Operations => "Operations",
        AdviserId::Navigation => "Navigation",
        AdviserId::DailyRoutine => "Daily Routine",
        AdviserId::Reporting => "Reporting",
        AdviserId::Plans => "Plans",
    }
}

pub(super) fn adviser_label(adviser: AdviserId) -> &'static str {
    match adviser {
        AdviserId::ChiefOfStaff => "chief_of_staff",
        AdviserId::Operations => "operations",
        AdviserId::Navigation => "navigation",
        AdviserId::DailyRoutine => "daily_routine",
        AdviserId::Reporting => "reporting",
        AdviserId::Plans => "plans",
    }
}

pub(super) fn source_error_code(error: &SourceCollectionError) -> CommandBriefFailureCode {
    match error {
        SourceCollectionError::Cancelled => CommandBriefFailureCode::CancellationRequested,
        SourceCollectionError::SnapshotChanged => CommandBriefFailureCode::SnapshotChanged,
        SourceCollectionError::RagUnavailable => CommandBriefFailureCode::RagUnavailable,
        SourceCollectionError::RagStale => CommandBriefFailureCode::RagStale,
        SourceCollectionError::RagInvalid => CommandBriefFailureCode::RagInvalid,
        SourceCollectionError::InvalidRequest => CommandBriefFailureCode::SourceRequestRejected,
        SourceCollectionError::InvalidTime => CommandBriefFailureCode::SourceTimeRejected,
        SourceCollectionError::ConflictingSourceIdentity => {
            CommandBriefFailureCode::SourceIdentityConflict
        }
    }
}

pub(super) fn status_value(
    run_id: &str,
    schedule_id: &str,
    sequence: u64,
    state: BriefRunState,
    degraded: &[BriefSection],
    error: Option<&str>,
) -> Result<BriefRunStatus, ()> {
    BriefRunStatus::try_from(json!({
        "classification": "OFFICIAL",
        "runId": run_id,
        "scheduleId": schedule_id,
        "sequence": sequence,
        "state": state,
        "updatedAt": timestamp(),
        "degradedSections": degraded,
        "error": error
    }))
    .map_err(|_| ())
}

pub(super) fn valid_bounded_text(value: &str, maximum: usize) -> bool {
    !value.is_empty()
        && value.trim() == value
        && value.len() <= maximum
        && !value.chars().any(char::is_control)
}

fn valid_text_array(values: &[String], maximum_items: usize) -> bool {
    values.len() <= maximum_items
        && values
            .iter()
            .all(|value| valid_bounded_text(value, MAX_TEXT_BYTES))
}

fn bounded_missing_information(
    failed_advisers: &[AdviserId],
    contributions: &[AdviserContribution],
    source_limitations: &[String],
) -> Vec<String> {
    let mut seen = BTreeSet::new();
    let mut required = Vec::new();
    for adviser in SPECIALIST_ADVISERS {
        if failed_advisers.contains(&adviser) {
            let limitation = adviser_unavailable(adviser);
            if valid_bounded_text(&limitation, MAX_TEXT_BYTES) && seen.insert(limitation.clone()) {
                required.push(limitation);
            }
        }
    }

    let mut specialist_limitations = contributions
        .iter()
        .flat_map(|contribution| contribution.limitations().iter().cloned())
        .collect::<Vec<_>>();
    specialist_limitations.sort();
    let mut source_limitations = source_limitations.to_vec();
    source_limitations.sort();

    let mut optional = Vec::new();
    for value in specialist_limitations.into_iter().chain(source_limitations) {
        if valid_bounded_text(&value, MAX_TEXT_BYTES) && seen.insert(value.clone()) {
            optional.push(value);
        }
    }

    if required.len() + optional.len() <= MAX_ARRAY_ITEMS {
        required.extend(optional);
        return required;
    }

    let optional_capacity = MAX_ARRAY_ITEMS
        .saturating_sub(required.len())
        .saturating_sub(1);
    let omitted = optional.len().saturating_sub(optional_capacity);
    required.extend(optional.into_iter().take(optional_capacity));
    required.push(format!(
        "{omitted} additional trusted limitations omitted after the canonical limit."
    ));
    required
}

pub(super) fn timestamp() -> String {
    Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true)
}

pub(super) fn is_terminal(state: BriefRunState) -> bool {
    matches!(
        state,
        BriefRunState::Completed
            | BriefRunState::Degraded
            | BriefRunState::Cancelled
            | BriefRunState::Failed
    )
}
