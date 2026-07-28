use super::command_team_discussions::COMMAND_TEAM_COLLECTION;
use super::*;

const MAX_CANONICAL_LEDGER_ITEMS: usize = 48;
const MAX_CANONICAL_SOURCE_QUOTE_BYTES: usize = 1_024;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct CandidateSource {
    pub(super) source_id: String,
    pub(super) source_kind: SourceKind,
    pub(super) collection: String,
    pub(super) document_id: String,
    pub(super) chunk_id: String,
    pub(super) timestamp: String,
    pub(super) location: String,
    pub(super) retrieved_at: String,
    pub(super) observed_at: String,
    pub(super) quote: String,
}

pub(super) fn snapshot_catalogue_source(
    snapshot: &VerifiedRagSnapshot,
) -> Result<CandidateSource, SourceCollectionError> {
    let (source_id, collection, chunk_id, location, quote_value) = match snapshot.assurance() {
        RagSnapshotAssurance::SignedSnapshot => (
            format!("rag:snapshot:{}", snapshot.snapshot_id()),
            "verified_catalogue",
            "active_snapshot",
            "cryptographically verified active snapshot catalogue",
            json!({
                "active_snapshot_id": snapshot.snapshot_id(),
                "logical_collections": snapshot.logical_collections(),
                "physical_collections": snapshot.physical_collections(),
                "snapshot_time": snapshot.snapshot_time(),
                "verified_at": snapshot.verified_at(),
            }),
        ),
        RagSnapshotAssurance::TrustedLanObserved => (
            format!("rag:catalogue:{}", snapshot.snapshot_id()),
            "observed_catalogue",
            "trusted-lan-observed",
            "trusted-lan-observed catalogue fingerprint; audit metadata only",
            json!({
                "assurance": "trusted-lan-observed",
                "catalogue_fingerprint": snapshot.snapshot_id(),
                "logical_collections": snapshot.logical_collections(),
                "observed_at": snapshot.verified_at(),
            }),
        ),
    };
    let quote = serde_jcs::to_vec(&quote_value)
        .ok()
        .and_then(|bytes| String::from_utf8(bytes).ok())
        .ok_or(SourceCollectionError::RagInvalid)?;
    Ok(CandidateSource {
        source_id,
        source_kind: SourceKind::Rag,
        collection: collection.to_string(),
        document_id: snapshot.snapshot_id().to_string(),
        chunk_id: chunk_id.to_string(),
        timestamp: snapshot.snapshot_time().to_string(),
        location: location.to_string(),
        retrieved_at: snapshot.verified_at().to_string(),
        observed_at: snapshot.verified_at().to_string(),
        quote,
    })
}

pub(super) fn collect_apple_response(
    selection: &AppleBriefSelection,
    request: &AppleInputRequest,
    response: AppleInputResponse,
    candidates: &mut Vec<CandidateSource>,
    degraded: &mut BTreeSet<BriefSection>,
    limitations: &mut BTreeSet<String>,
) {
    let expected_source = request.source_name();
    if response.source_name() != expected_source {
        degraded.insert(BriefSection::DailyRoutine);
        limitations.insert(format!(
            "Apple {expected_source} input failed signed-helper source binding."
        ));
        return;
    }
    if response.permission() != AppleInputPermission::Authorized {
        degraded.insert(BriefSection::DailyRoutine);
        limitations.insert(format!(
            "Apple {expected_source} input permission is {}.",
            response.permission().name()
        ));
        return;
    }
    if response.error().is_some() {
        degraded.insert(BriefSection::DailyRoutine);
        limitations.insert(format!(
            "Apple {expected_source} input failed in the signed helper."
        ));
        return;
    }
    if DateTime::parse_from_rfc3339(response.observed_at()).is_err() {
        degraded.insert(BriefSection::DailyRoutine);
        limitations.insert(format!(
            "Apple {expected_source} input had an invalid observation time."
        ));
        return;
    }
    if response.truncated() {
        degraded.insert(BriefSection::DailyRoutine);
        limitations.insert(format!(
            "Apple {expected_source} input was truncated by the signed helper."
        ));
    }
    for record in response.records() {
        let fields = record.fields();
        if !selection.permits_record(expected_source, fields) {
            degraded.insert(BriefSection::DailyRoutine);
            limitations.insert(format!(
                "Apple {expected_source} returned a record outside the protected allowlist."
            ));
            continue;
        }
        let deleted = bool_field(fields, "is_deleted");
        let stale = bool_field(fields, "is_stale");
        if deleted == Some(true) {
            degraded.insert(BriefSection::DailyRoutine);
            limitations.insert(format!(
                "A deleted Apple {expected_source} record was excluded."
            ));
            continue;
        }
        if stale == Some(true) {
            degraded.insert(BriefSection::DailyRoutine);
            limitations.insert(format!(
                "A stale Apple {expected_source} record was excluded."
            ));
            continue;
        }
        if deleted.is_none() && fields.contains_key("is_deleted")
            || stale.is_none() && fields.contains_key("is_stale")
        {
            degraded.insert(BriefSection::DailyRoutine);
            limitations.insert(format!(
                "An Apple {expected_source} record had invalid freshness metadata."
            ));
            continue;
        }
        if let Some(candidate) = apple_candidate(request, response.observed_at(), fields) {
            candidates.push(candidate);
        } else {
            degraded.insert(BriefSection::DailyRoutine);
            limitations.insert(format!("An Apple {expected_source} record was malformed."));
        }
    }
}

fn bool_field(fields: &BTreeMap<String, String>, key: &str) -> Option<bool> {
    fields.get(key).and_then(|value| match value.as_str() {
        "true" => Some(true),
        "false" => Some(false),
        _ => None,
    })
}

fn apple_candidate(
    request: &AppleInputRequest,
    observed_at: &str,
    fields: &BTreeMap<String, String>,
) -> Option<CandidateSource> {
    let source = request.source_name();
    let (kind, identity, collection, location) = match source {
        "calendar" => {
            if !exact_apple_fields(
                fields,
                &[
                    "identifier",
                    "calendar_identifier",
                    "title",
                    "start",
                    "end",
                    "is_recurring",
                    "recurrence_identifier",
                    "is_deleted",
                    "is_stale",
                ],
            ) || bool_field(fields, "is_recurring").is_none()
                || bool_field(fields, "is_deleted").is_none()
                || bool_field(fields, "is_stale").is_none()
                || !calendar_record_in_window(request, fields)
            {
                return None;
            }
            let identifier = fields.get("identifier")?;
            let recurrence = fields.get("recurrence_identifier")?;
            let location = if recurrence.is_empty() {
                format!("calendar event {identifier}")
            } else {
                format!("calendar event {identifier} recurrence {recurrence}")
            };
            (
                SourceKind::Calendar,
                format!("calendar:{identifier}:{recurrence}"),
                fields.get("calendar_identifier")?.clone(),
                location,
            )
        }
        "reminders" => {
            if !exact_apple_fields(
                fields,
                &[
                    "identifier",
                    "list_identifier",
                    "title",
                    "is_completed",
                    "recurrence_identifier",
                    "due_date",
                    "completion_date",
                    "is_deleted",
                    "is_stale",
                ],
            ) || bool_field(fields, "is_completed").is_none()
                || bool_field(fields, "is_deleted").is_none()
                || bool_field(fields, "is_stale").is_none()
                || !reminder_record_in_window(request, fields)
            {
                return None;
            }
            let identifier = fields.get("identifier")?;
            let recurrence = fields.get("recurrence_identifier")?;
            let location = if recurrence.is_empty() {
                format!("reminder {identifier}")
            } else {
                format!("reminder {identifier} recurrence {recurrence}")
            };
            (
                SourceKind::Reminders,
                format!("reminder:{identifier}:{recurrence}"),
                fields.get("list_identifier")?.clone(),
                location,
            )
        }
        "notes" => {
            if !exact_apple_fields(
                fields,
                &["identifier", "folder_identifier", "title", "body"],
            ) {
                return None;
            }
            let identifier = fields.get("identifier")?;
            (
                SourceKind::Notes,
                format!("note:{identifier}"),
                fields.get("folder_identifier")?.clone(),
                format!("note {identifier}"),
            )
        }
        "files" => {
            if !exact_apple_fields(fields, &["path", "contents", "device", "inode"])
                || fields
                    .get("device")
                    .and_then(|value| value.parse::<u64>().ok())
                    .is_none()
                || fields
                    .get("inode")
                    .and_then(|value| value.parse::<u64>().ok())
                    .is_none()
            {
                return None;
            }
            let path = fields.get("path")?;
            let identity = format!(
                "file:{}",
                digest_text(&format!(
                    "{path}:{}:{}",
                    fields.get("device")?,
                    fields.get("inode")?
                ))
            );
            (
                SourceKind::File,
                identity,
                "approved_files".to_string(),
                path.clone(),
            )
        }
        _ => return None,
    };
    let quote = serde_jcs::to_vec(fields)
        .ok()
        .and_then(|bytes| String::from_utf8(bytes).ok())?;
    Some(CandidateSource {
        source_id: identity.clone(),
        source_kind: kind,
        collection,
        document_id: identity.clone(),
        chunk_id: identity,
        timestamp: observed_at.to_string(),
        location,
        retrieved_at: observed_at.to_string(),
        observed_at: observed_at.to_string(),
        quote,
    })
}

fn exact_apple_fields(fields: &BTreeMap<String, String>, expected: &[&str]) -> bool {
    fields.len() == expected.len()
        && expected.iter().all(|key| fields.contains_key(*key))
        && fields.iter().all(|(key, value)| {
            value.len() <= 1024 * 1024
                && (matches!(
                    key.as_str(),
                    "title"
                        | "body"
                        | "contents"
                        | "recurrence_identifier"
                        | "due_date"
                        | "completion_date"
                ) || (!value.is_empty()
                    && value.trim() == value
                    && !value.chars().any(char::is_control)))
        })
}

fn calendar_record_in_window(
    request: &AppleInputRequest,
    fields: &BTreeMap<String, String>,
) -> bool {
    let Some((window_start, window_end)) = request.read_window() else {
        return false;
    };
    let (Ok(window_start), Ok(window_end), Some(start), Some(end)) = (
        DateTime::parse_from_rfc3339(window_start),
        DateTime::parse_from_rfc3339(window_end),
        fields
            .get("start")
            .and_then(|value| DateTime::parse_from_rfc3339(value).ok()),
        fields
            .get("end")
            .and_then(|value| DateTime::parse_from_rfc3339(value).ok()),
    ) else {
        return false;
    };
    start < end && start < window_end && end > window_start
}

fn reminder_record_in_window(
    request: &AppleInputRequest,
    fields: &BTreeMap<String, String>,
) -> bool {
    let Some((window_start, window_end)) = request.read_window() else {
        return false;
    };
    let (Ok(window_start), Ok(window_end), Some(completed)) = (
        DateTime::parse_from_rfc3339(window_start),
        DateTime::parse_from_rfc3339(window_end),
        bool_field(fields, "is_completed"),
    ) else {
        return false;
    };
    let parse_optional = |key: &str| {
        fields.get(key).and_then(|value| {
            if value.is_empty() {
                Some(None)
            } else {
                DateTime::parse_from_rfc3339(value).ok().map(Some)
            }
        })
    };
    let (Some(due), Some(completion)) = (
        parse_optional("due_date"),
        parse_optional("completion_date"),
    ) else {
        return false;
    };
    let inside = |value: &DateTime<_>| *value >= window_start && *value < window_end;
    if completed {
        completion.as_ref().is_some_and(inside)
    } else {
        due.as_ref().is_some_and(inside)
    }
}

pub(super) fn canonical_ledger(
    run_id: &str,
    snapshot_id: &str,
    candidates: Vec<CandidateSource>,
) -> Result<CanonicalLedgerOutput, SourceCollectionError> {
    let mut by_source = BTreeMap::<String, CandidateSource>::new();
    let mut limitations = BTreeSet::new();
    let mut rejected_by_kind = [0_usize; 7];
    let mut rejected_command_team_discussions = 0_usize;
    for mut candidate in candidates {
        let Some((quote, truncated)) =
            canonical_quote(&candidate.quote, MAX_CANONICAL_SOURCE_QUOTE_BYTES)
        else {
            if is_command_team_discussion(&candidate) {
                rejected_command_team_discussions += 1;
            } else {
                rejected_by_kind[source_priority(candidate.source_kind) as usize] += 1;
            }
            continue;
        };
        candidate.quote = quote;
        if truncated {
            limitations.insert(format!(
                "Source {} was truncated to the canonical source-size limit.",
                candidate.source_id
            ));
        }
        match by_source.get_mut(&candidate.source_id) {
            None => {
                by_source.insert(candidate.source_id.clone(), candidate);
            }
            Some(existing) if same_source_content(existing, &candidate) => {
                if candidate.retrieved_at < existing.retrieved_at {
                    existing.retrieved_at = candidate.retrieved_at;
                }
                if candidate.observed_at < existing.observed_at {
                    existing.observed_at = candidate.observed_at;
                }
            }
            Some(_) => return Err(SourceCollectionError::ConflictingSourceIdentity),
        }
    }
    let mut candidates = by_source.into_values().collect::<Vec<_>>();
    candidates.sort_by(|left, right| {
        retention_priority(left.source_kind)
            .cmp(&retention_priority(right.source_kind))
            .then_with(|| left.source_id.cmp(&right.source_id))
    });
    let mut omitted_by_kind = [0_usize; 7];
    let mut omitted_command_team_discussions = 0_usize;
    if candidates.len() > MAX_CANONICAL_LEDGER_ITEMS {
        for candidate in &candidates[MAX_CANONICAL_LEDGER_ITEMS..] {
            if is_command_team_discussion(candidate) {
                omitted_command_team_discussions += 1;
            } else {
                omitted_by_kind[source_priority(candidate.source_kind) as usize] += 1;
            }
        }
        candidates.truncate(MAX_CANONICAL_LEDGER_ITEMS);
    }
    let mut ledger = Vec::with_capacity(candidates.len());
    for candidate in candidates {
        let priority = source_priority(candidate.source_kind);
        let command_team_discussion = is_command_team_discussion(&candidate);
        let ledger_id = format!(
            "source-{}",
            &digest_text(&format!(
                "{run_id}:{priority}:{}:{snapshot_id}",
                candidate.source_id
            ))[..24]
        );
        let value = json!({
            "classification": "OFFICIAL",
            "ledgerId": ledger_id,
            "sourceId": candidate.source_id,
            "sourceKind": candidate.source_kind,
            "collection": candidate.collection,
            "documentId": candidate.document_id,
            "chunkId": candidate.chunk_id,
            "timestamp": candidate.timestamp,
            "snapshotId": snapshot_id,
            "quotedLocation": {
                "quote": candidate.quote,
                "location": candidate.location
            },
            "retrievedAt": candidate.retrieved_at,
            "observedAt": candidate.observed_at
        });
        match SourceLedgerEntry::parse_for_snapshot(value, snapshot_id) {
            Ok(entry) => ledger.push(entry),
            Err(_) if command_team_discussion => {
                rejected_command_team_discussions += 1;
            }
            Err(_) => rejected_by_kind[priority as usize] += 1,
        }
    }
    Ok(CanonicalLedgerOutput {
        ledger,
        limitations,
        omitted_by_kind,
        rejected_by_kind,
        omitted_command_team_discussions,
        rejected_command_team_discussions,
    })
}

fn is_command_team_discussion(candidate: &CandidateSource) -> bool {
    candidate.source_kind == SourceKind::Memory && candidate.collection == COMMAND_TEAM_COLLECTION
}

const fn retention_priority(kind: SourceKind) -> u8 {
    match kind {
        SourceKind::Calendar => 0,
        SourceKind::Reminders => 1,
        SourceKind::Notes => 2,
        SourceKind::File => 3,
        SourceKind::Memory => 4,
        SourceKind::Rag => 5,
        SourceKind::WorldMonitor => 6,
    }
}

pub(super) struct CanonicalLedgerOutput {
    pub(super) ledger: Vec<SourceLedgerEntry>,
    pub(super) limitations: BTreeSet<String>,
    pub(super) omitted_by_kind: [usize; 7],
    pub(super) rejected_by_kind: [usize; 7],
    pub(super) omitted_command_team_discussions: usize,
    pub(super) rejected_command_team_discussions: usize,
}

pub(super) fn apply_command_team_ledger_losses(
    omitted: usize,
    rejected: usize,
    limitations: &mut BTreeSet<String>,
) {
    if omitted > 0 {
        limitations.insert(format!(
            "{omitted} Command-team discussion sources were omitted by the canonical ledger limit."
        ));
    }
    if rejected > 0 {
        limitations.insert(format!(
            "{rejected} malformed Command-team discussion sources were excluded from the canonical ledger."
        ));
    }
}

pub(super) fn apply_ledger_omissions(
    omitted_by_kind: &[usize; 7],
    degraded: &mut BTreeSet<BriefSection>,
    limitations: &mut BTreeSet<String>,
) {
    for (kind, count) in [
        (SourceKind::Rag, omitted_by_kind[0]),
        (SourceKind::Memory, omitted_by_kind[1]),
        (SourceKind::WorldMonitor, omitted_by_kind[2]),
        (SourceKind::Calendar, omitted_by_kind[3]),
        (SourceKind::Reminders, omitted_by_kind[4]),
        (SourceKind::Notes, omitted_by_kind[5]),
        (SourceKind::File, omitted_by_kind[6]),
    ] {
        if count == 0 {
            continue;
        }
        limitations.insert(format!(
            "{count} {} sources were omitted by the canonical ledger limit.",
            source_kind_name(kind)
        ));
        match kind {
            SourceKind::Rag | SourceKind::Memory => degraded.extend(RAG_MEMORY_SECTIONS),
            SourceKind::WorldMonitor => {
                degraded.insert(BriefSection::Intelligence);
            }
            SourceKind::Calendar | SourceKind::Reminders | SourceKind::Notes | SourceKind::File => {
                degraded.insert(BriefSection::DailyRoutine);
            }
        }
    }
}

pub(super) fn apply_ledger_rejections(
    rejected_by_kind: &[usize; 7],
    degraded: &mut BTreeSet<BriefSection>,
    limitations: &mut BTreeSet<String>,
) {
    for (kind, count) in [
        (SourceKind::Rag, rejected_by_kind[0]),
        (SourceKind::Memory, rejected_by_kind[1]),
        (SourceKind::WorldMonitor, rejected_by_kind[2]),
        (SourceKind::Calendar, rejected_by_kind[3]),
        (SourceKind::Reminders, rejected_by_kind[4]),
        (SourceKind::Notes, rejected_by_kind[5]),
        (SourceKind::File, rejected_by_kind[6]),
    ] {
        if count == 0 {
            continue;
        }
        limitations.insert(format!(
            "{count} malformed {} sources were excluded from the canonical ledger.",
            source_kind_name(kind)
        ));
        match kind {
            SourceKind::Rag | SourceKind::Memory => degraded.extend(RAG_MEMORY_SECTIONS),
            SourceKind::WorldMonitor => {
                degraded.insert(BriefSection::Intelligence);
            }
            SourceKind::Calendar | SourceKind::Reminders | SourceKind::Notes | SourceKind::File => {
                degraded.insert(BriefSection::DailyRoutine);
            }
        }
    }
}

const fn source_kind_name(kind: SourceKind) -> &'static str {
    match kind {
        SourceKind::Rag => "RAG",
        SourceKind::Memory => "Memory",
        SourceKind::WorldMonitor => "World Monitor",
        SourceKind::Calendar => "calendar",
        SourceKind::Reminders => "reminder",
        SourceKind::Notes => "note",
        SourceKind::File => "file",
    }
}

fn same_source_content(left: &CandidateSource, right: &CandidateSource) -> bool {
    left.source_kind == right.source_kind
        && left.collection == right.collection
        && left.document_id == right.document_id
        && left.chunk_id == right.chunk_id
        && left.timestamp == right.timestamp
        && left.location == right.location
        && left.quote == right.quote
}

const fn source_priority(kind: SourceKind) -> u8 {
    match kind {
        SourceKind::Rag => 0,
        SourceKind::Memory => 1,
        SourceKind::WorldMonitor => 2,
        SourceKind::Calendar => 3,
        SourceKind::Reminders => 4,
        SourceKind::Notes => 5,
        SourceKind::File => 6,
    }
}

fn digest_text(value: &str) -> String {
    hex::encode(Sha256::digest(value.as_bytes()))
}

fn canonical_quote(value: &str, maximum_bytes: usize) -> Option<(String, bool)> {
    if value.trim().is_empty() {
        return None;
    }
    let encoded = serde_json::to_string(value).ok()?;
    if encoded.len() <= maximum_bytes {
        return Some((encoded, false));
    }

    let boundaries = value
        .char_indices()
        .map(|(index, character)| index + character.len_utf8())
        .collect::<Vec<_>>();
    let mut low = 0;
    let mut high = boundaries.len();
    let mut best = None;
    while low < high {
        let middle = low + (high - low) / 2;
        let end = boundaries[middle];
        let candidate = serde_json::to_string(&value[..end]).ok()?;
        if candidate.len() <= maximum_bytes {
            best = Some(candidate);
            low = middle + 1;
        } else {
            high = middle;
        }
    }
    best.map(|candidate| (candidate, true))
}

pub(super) fn truncate_utf8(value: &str, maximum_bytes: usize) -> (String, bool) {
    if value.len() <= maximum_bytes {
        return (value.to_string(), false);
    }
    let mut end = maximum_bytes;
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    (value[..end].to_string(), true)
}

impl From<RagSnapshotError> for SourceCollectionError {
    fn from(value: RagSnapshotError) -> Self {
        match value {
            RagSnapshotError::Invalid => Self::RagInvalid,
            RagSnapshotError::Changed => Self::SnapshotChanged,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::command_brief::sources::command_team_discussions::COMMAND_TEAM_COLLECTION;

    fn candidate(index: usize, kind: SourceKind, collection: &str, quote: &str) -> CandidateSource {
        let identity = format!("{index:064x}");
        CandidateSource {
            source_id: identity.clone(),
            source_kind: kind,
            collection: collection.to_string(),
            document_id: identity.clone(),
            chunk_id: identity,
            timestamp: "2026-07-27T02:00:00Z".to_string(),
            location: format!("test source {index}"),
            retrieved_at: "2026-07-27T02:01:00Z".to_string(),
            observed_at: "2026-07-27T02:01:00Z".to_string(),
            quote: quote.to_string(),
        }
    }

    #[test]
    fn omitted_command_team_discussions_warn_without_degrading_sections() {
        let mut candidates = (0..MAX_CANONICAL_LEDGER_ITEMS)
            .map(|index| candidate(index, SourceKind::Calendar, "calendar", "calendar evidence"))
            .collect::<Vec<_>>();
        candidates.push(candidate(
            MAX_CANONICAL_LEDGER_ITEMS + 1,
            SourceKind::Memory,
            COMMAND_TEAM_COLLECTION,
            "discussion evidence",
        ));

        let canonical =
            canonical_ledger("brief-run:test", "a".repeat(64).as_str(), candidates).unwrap();
        let mut degraded = BTreeSet::new();
        let mut limitations = BTreeSet::new();
        apply_ledger_omissions(&canonical.omitted_by_kind, &mut degraded, &mut limitations);
        apply_ledger_rejections(&canonical.rejected_by_kind, &mut degraded, &mut limitations);
        apply_command_team_ledger_losses(
            canonical.omitted_command_team_discussions,
            canonical.rejected_command_team_discussions,
            &mut limitations,
        );

        assert!(degraded.is_empty());
        assert!(limitations
            .iter()
            .any(|item| item.contains("Command-team discussion")));
    }

    #[test]
    fn malformed_command_team_discussions_warn_without_degrading_sections() {
        let candidates = vec![candidate(
            1,
            SourceKind::Memory,
            COMMAND_TEAM_COLLECTION,
            "",
        )];

        let canonical =
            canonical_ledger("brief-run:test", "a".repeat(64).as_str(), candidates).unwrap();
        let mut degraded = BTreeSet::new();
        let mut limitations = BTreeSet::new();
        apply_ledger_rejections(&canonical.rejected_by_kind, &mut degraded, &mut limitations);
        apply_command_team_ledger_losses(
            canonical.omitted_command_team_discussions,
            canonical.rejected_command_team_discussions,
            &mut limitations,
        );

        assert!(degraded.is_empty());
        assert!(limitations
            .iter()
            .any(|item| item.contains("malformed Command-team discussion")));
    }
}
