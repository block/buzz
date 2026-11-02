//! Shared typed content and namespace rules for Colony business records.
//!
//! Business records remain ordinary signed Nostr events. A community is the
//! outer tenant, client groups are private NIP-29 channels, and every d-tag is
//! namespaced so NIP-33 coordinates cannot collide across clients.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;
use uuid::Uuid;

/// Current business-record JSON schema version.
pub const BUSINESS_RECORD_SCHEMA_VERSION: u8 = 1;
/// Longest work item title, in characters.
pub const MAX_WORK_ITEM_TITLE_CHARS: usize = 180;
/// Longest company work item done condition, in characters.
pub const MAX_WORK_ITEM_DONE_CONDITION_CHARS: usize = 1000;
/// Longest company work item evidence text, in characters.
pub const MAX_WORK_ITEM_EVIDENCE_CHARS: usize = 2000;
/// Longest company work item status or verification reason, in characters.
pub const MAX_WORK_ITEM_REASON_CHARS: usize = 1000;

/// Errors returned while parsing or binding a business-record event.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum BusinessRecordError {
    /// The event kind does not have a member-authored business command schema.
    #[error("unsupported business command kind")]
    UnsupportedKind,
    /// The JSON content does not match the kind's typed schema.
    #[error("invalid business record content")]
    InvalidContent,
    /// The record uses a schema version the relay does not understand.
    #[error("unsupported business record schema version")]
    UnsupportedSchemaVersion,
    /// The client UUID does not match the NIP-29 channel UUID.
    #[error("client id does not match channel scope")]
    ClientChannelMismatch,
    /// The d-tag is not in the expected namespace for this record.
    #[error("business record d-tag does not match its namespace")]
    DTagMismatch,
    /// A version or digest is not a 32-byte lowercase hexadecimal value.
    #[error("invalid event id or SHA-256 digest")]
    InvalidHexReference,
    /// A business field breaks a contract rule.
    #[error("invalid business record: {0}")]
    Invalid(&'static str),
}

/// Member-authored business command content parsed by the relay broker.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "record", rename_all = "snake_case")]
pub enum BusinessCommand {
    /// Party identity mutation.
    PartyAction(PartyAction),
    /// Business service catalog mutation.
    ServiceAction(ServiceAction),
    /// Prospect qualification, save, or pipeline mutation.
    ProspectAction(Box<ProspectAction>),
    /// Client relationship mutation.
    ClientAction(ClientAction),
    /// Work-item mutation.
    WorkItemAction(WorkItemAction),
    /// Company work-item mutation using the shared work item kinds.
    CompanyWorkItemAction(CompanyWorkItemAction),
    /// Immutable proposal revision.
    ProposalVersion(ProposalVersion),
    /// Exact-version proposal acceptance and conversion request.
    ProposalAcceptance(ProposalAcceptance),
    /// Immutable deliverable revision.
    DeliverableVersion(DeliverableVersion),
    /// Exact-version deliverable approval decision.
    DeliverableApproval(DeliverableApproval),
    /// Immutable invoice revision request and lifecycle transition.
    InvoiceVersion(InvoiceVersion),
    /// Append-only evidence that a client payment was received.
    PaymentEvidence(PaymentEvidence),
    /// Append-only credit, refund, or write-off evidence.
    MoneyAdjustment(MoneyAdjustment),
    /// Draft, review, or approval of an overdue follow-up intent.
    MoneyFollowUp(MoneyFollowUpAction),
}

/// A business or client record action.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RecordAction {
    /// Create a record that has no current head.
    Create,
    /// Replace a record using an exact expected head event id.
    Update,
    /// Mark a record as archived while retaining its history.
    Archive,
    /// Restore a previously archived record.
    Restore,
}

/// Approval outcome attached to one immutable version.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalDecision {
    /// The exact version is accepted.
    Approved,
    /// The exact version needs changes.
    ChangesRequested,
    /// The exact version is rejected.
    Rejected,
}

/// Party identity shared by a prospect and a client relationship.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct PartyRecord {
    /// Stable party UUID.
    pub party_id: Uuid,
    /// `person` or `organization`.
    pub party_type: String,
    /// Display name from the source record.
    pub display_name: String,
    /// Source-linked external identifiers, never secret credentials.
    pub external_ids: Vec<String>,
}

/// Relay-authored canonical party head content.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct PartyHead {
    /// Schema version.
    pub schema_version: u8,
    /// The stable party UUID.
    pub party_id: Uuid,
    /// Whether this party identity is active or archived.
    pub status: String,
    /// Current party identity data.
    pub party: PartyRecord,
    /// Event id of the member action that produced this head.
    pub source_action_event_id: String,
}

/// Member request to create or update a party identity.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct PartyAction {
    /// Schema version.
    pub schema_version: u8,
    /// Stable party UUID.
    pub party_id: Uuid,
    /// Requested mutation.
    pub action: RecordAction,
    /// Expected current party head id, absent only for creation.
    pub expected_head_event_id: Option<String>,
    /// Replacement party identity data.
    pub party: PartyRecord,
}

/// Editable terms for one agency service offering.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct ServiceRecord {
    /// Stable service UUID.
    pub service_id: Uuid,
    /// Service name shown in the catalog and proposals.
    pub name: String,
    /// Short service scope.
    pub description: String,
    /// ISO 4217 currency code.
    pub currency: String,
    /// Monthly fee in minor currency units.
    pub monthly_fee_minor: i64,
    /// Number of social posts included each month.
    pub posts_per_month: u16,
    /// Included revision rounds.
    pub revision_rounds: u8,
}

/// Relay-authored current business service head.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct ServiceHead {
    /// Schema version.
    pub schema_version: u8,
    /// Stable service UUID.
    pub service_id: Uuid,
    /// Whether this service is available or archived.
    pub status: String,
    /// Current service terms.
    pub service: ServiceRecord,
    /// Event id of the member action that produced this head.
    pub source_action_event_id: String,
}

/// Member request to create, update, archive, or restore a service.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct ServiceAction {
    /// Schema version.
    pub schema_version: u8,
    /// Stable service UUID.
    pub service_id: Uuid,
    /// Requested mutation.
    pub action: RecordAction,
    /// Expected current service head id, absent only for creation.
    pub expected_head_event_id: Option<String>,
    /// Replacement service terms.
    pub service: ServiceRecord,
}

/// Current stage of a prospect in the sales pipeline.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProspectStage {
    /// Prospect has been qualified for a conversation.
    Qualified,
    /// A conversation is active.
    InConversation,
    /// A proposal has been sent.
    Proposal,
    /// The prospect accepted and converted.
    Won,
    /// The opportunity was lost and may be reopened.
    Lost,
}

/// Qualification outcome recorded for a prospect.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProspectQualification {
    /// Not yet reviewed.
    Unreviewed,
    /// Fits the service and is worth pursuing.
    Qualified,
    /// Does not fit the current service.
    NotFit,
}

/// Evidence collected for a prospect, with its observation time.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct ProspectEvidence {
    /// Evidence title.
    pub title: String,
    /// HTTPS source URL.
    pub url: String,
    /// Short source excerpt.
    pub excerpt: String,
    /// Unix time when the source was checked.
    pub observed_at: i64,
}

/// How a prospect entered the company's pipeline.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProspectOrigin {
    /// Added directly by a company member.
    Manual,
    /// Returned by a Discovery search.
    Discovery,
}

/// A taxonomy tag matched against fields returned by the discovery provider.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct ProspectDiscoveryTagInput {
    /// The matched tag text.
    pub tag: String,
    /// The generated taxonomy category for the tag.
    pub kind: String,
    /// Whether the searcher picked this tag as a search refinement.
    pub picked: bool,
}

/// Activity kind for a qualification or contact note.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProspectActivityKind {
    /// Internal qualification note.
    Note,
    /// A contact attempt, without sending anything through this event.
    Contact,
}

/// Relay-stamped immutable prospect activity.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct ProspectActivity {
    /// Stable activity UUID.
    pub activity_id: Uuid,
    /// Activity type.
    pub activity_kind: ProspectActivityKind,
    /// Note content.
    pub content: String,
    /// Proposal this activity requests changes to, when applicable.
    #[serde(default)]
    pub proposal_id: Option<Uuid>,
    /// Exact proposal version this activity references, when applicable.
    #[serde(default)]
    pub proposal_version_event_id: Option<String>,
    /// Member pubkey that authored this activity.
    pub author_pubkey: String,
    /// Unix time from the signed source event.
    pub created_at: i64,
}

/// Member-supplied fields for a prospect head.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct ProspectRecordInput {
    /// Stable prospect UUID.
    pub prospect_id: Uuid,
    /// Party identity shared with proposals and client conversion.
    pub party: PartyRecord,
    /// Industry taxonomy label.
    pub industry: String,
    /// Vertical taxonomy label.
    pub vertical: String,
    /// Provider or reviewer fit score from zero through one hundred.
    #[serde(default)]
    pub fit_score: Option<u8>,
    /// Provenance of the prospect. Missing on older records.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub origin: Option<ProspectOrigin>,
    /// Generated tag evidence matched from the provider listing.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub discovery_tags: Vec<ProspectDiscoveryTagInput>,
    /// Potential monthly service value in minor currency units, when known.
    #[serde(default)]
    pub potential_monthly_value_minor: Option<i64>,
    /// Prospect website, if known.
    pub website: Option<String>,
    /// Primary business contact name, if known.
    #[serde(default)]
    pub contact_name: Option<String>,
    /// Geographic area, if known.
    pub location: Option<String>,
    /// Business contact email, if known.
    pub email: Option<String>,
    /// Business contact phone, if known.
    pub phone: Option<String>,
    /// Source evidence and observation times.
    pub evidence: Vec<ProspectEvidence>,
    /// Time the prospect's current details were last checked.
    pub last_verified_at: Option<i64>,
    /// Qualification outcome.
    pub qualification: ProspectQualification,
    /// Whether this prospect is saved for follow-up.
    pub saved: bool,
    /// Current pipeline stage.
    pub stage: ProspectStage,
    /// Optional reason the opportunity was lost.
    pub lost_reason: Option<String>,
}

/// Optional append to a prospect's immutable activity history.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct ProspectActivityInput {
    /// Stable activity UUID.
    pub activity_id: Uuid,
    /// Activity type.
    pub activity_kind: ProspectActivityKind,
    /// Note content.
    pub content: String,
    /// Proposal this activity requests changes to, when applicable.
    #[serde(default)]
    pub proposal_id: Option<Uuid>,
    /// Exact proposal version this activity references, when applicable.
    #[serde(default)]
    pub proposal_version_event_id: Option<String>,
}

/// Relay-authored complete prospect head.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct ProspectHead {
    /// Schema version.
    pub schema_version: u8,
    /// Stable prospect UUID.
    pub prospect_id: Uuid,
    /// Whether this prospect is active or archived.
    pub status: String,
    /// Current prospect fields.
    pub prospect: ProspectRecordInput,
    /// Immutable notes and contact activity.
    pub activities: Vec<ProspectActivity>,
    /// Event id of the member action that produced this head.
    pub source_action_event_id: String,
}

/// Member request to create or update a prospect.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct ProspectAction {
    /// Schema version.
    pub schema_version: u8,
    /// Stable prospect UUID.
    pub prospect_id: Uuid,
    /// Requested mutation.
    pub action: RecordAction,
    /// Expected current prospect head id, absent only for creation.
    pub expected_head_event_id: Option<String>,
    /// Replacement prospect fields.
    pub prospect: ProspectRecordInput,
    /// One optional new activity entry to append.
    pub activity: Option<ProspectActivityInput>,
}

/// Relay-authored canonical client relationship head content.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct ClientHead {
    /// Schema version.
    pub schema_version: u8,
    /// Stable client UUID, equal to the private NIP-29 client channel UUID.
    pub client_id: Uuid,
    /// Canonical person or organization identity.
    pub party_id: Uuid,
    /// Client-facing display name.
    pub display_name: String,
    /// Pubkeys authorized to approve client-facing work.
    pub approver_pubkeys: Vec<String>,
    /// Current relationship state.
    pub status: String,
    /// Event id of the member action that produced this head.
    pub source_action_event_id: String,
}

/// Member-supplied client head fields. The relay fills the source event id.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct ClientHeadInput {
    /// Schema version.
    pub schema_version: u8,
    /// Stable client UUID, equal to the private channel UUID.
    pub client_id: Uuid,
    /// Canonical person or organization identity.
    pub party_id: Uuid,
    /// Client-facing display name.
    pub display_name: String,
    /// Pubkeys authorized to approve client-facing work.
    pub approver_pubkeys: Vec<String>,
    /// Current relationship state.
    pub status: String,
}

/// Member request to create or update a client relationship.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct ClientAction {
    /// Schema version.
    pub schema_version: u8,
    /// Stable client UUID and required `h` channel UUID.
    pub client_id: Uuid,
    /// Requested mutation.
    pub action: RecordAction,
    /// Expected current client head id, absent only for creation.
    pub expected_head_event_id: Option<String>,
    /// Canonical client record to write.
    pub head: ClientHeadInput,
}

/// Current deliverable version pointer in a work-item head.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct DeliverablePointer {
    /// Stable deliverable UUID.
    pub deliverable_id: Uuid,
    /// Exact current immutable version event id.
    pub version_event_id: String,
    /// SHA-256 digest of the canonical JSON body, excluding the version envelope.
    pub content_digest: String,
    /// SHA-256 digest of the ordered media digest list.
    pub media_digest: String,
    /// SHA-256 digest combining the content and media digests.
    pub version_digest: String,
}

/// Relay-authored canonical client work-item head content.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct WorkItemHead {
    /// Schema version.
    pub schema_version: u8,
    /// Stable client UUID and required `h` channel UUID.
    pub client_id: Uuid,
    /// Stable work-item UUID.
    pub work_item_id: Uuid,
    /// Short work title.
    pub title: String,
    /// Current work state.
    pub status: String,
    /// Pubkeys allowed to submit work output.
    pub assigned_pubkeys: Vec<String>,
    /// Pubkeys authorized to accept client-facing output.
    pub approver_pubkeys: Vec<String>,
    /// Current version of each deliverable in this work item.
    pub deliverables: Vec<DeliverablePointer>,
    /// Event id of the member action or version event that produced this head.
    pub source_event_id: String,
}

/// Member-supplied work-item fields. The relay fills the source event id.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct WorkItemHeadInput {
    /// Schema version.
    pub schema_version: u8,
    /// Stable client UUID and private channel UUID.
    pub client_id: Uuid,
    /// Stable work-item UUID.
    pub work_item_id: Uuid,
    /// Short work title.
    pub title: String,
    /// Current work state.
    pub status: String,
    /// Pubkeys allowed to submit work output.
    pub assigned_pubkeys: Vec<String>,
    /// Pubkeys authorized to accept client-facing output.
    pub approver_pubkeys: Vec<String>,
    /// Current version of each deliverable in this work item.
    pub deliverables: Vec<DeliverablePointer>,
}

/// Member request to create or update a client work item.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct WorkItemAction {
    /// Schema version.
    pub schema_version: u8,
    /// Stable client UUID and required `h` channel UUID.
    pub client_id: Uuid,
    /// Stable work-item UUID.
    pub work_item_id: Uuid,
    /// Requested mutation.
    pub action: RecordAction,
    /// Expected current work-item head id, absent only for creation.
    pub expected_head_event_id: Option<String>,
    /// Replacement work-item head content.
    pub head: WorkItemHeadInput,
}

/// Lifecycle status for a company work item.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CompanyWorkStatus {
    /// Work is in progress.
    Active,
    /// Work is intentionally paused.
    Paused,
    /// Work cannot proceed without help or a decision.
    Blocked,
    /// The owner says the done condition is met, pending review.
    DoneUnverified,
    /// An authorized reviewer accepted the submitted evidence.
    DoneVerified,
    /// Work is archived and retained for reference.
    Archived,
}

/// A company work item action.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CompanyWorkItemActionKind {
    /// Create a company work item.
    Create,
    /// Replace editable company work item fields.
    Update,
    /// Change the work item's progress status.
    SetStatus,
    /// Submit a reviewer verdict for an item awaiting verification.
    Verify,
    /// Archive a company work item.
    Archive,
    /// Restore an archived company work item.
    Restore,
    /// Set the company work item's due date.
    SetDueDate,
    /// Clear the company work item's due date.
    ClearDueDate,
}

/// Result recorded when an authorized reviewer checks completed work.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CompanyWorkVerdict {
    /// The evidence meets the done condition.
    Pass,
    /// More work is required.
    RevisionRequested,
}

/// Verification evidence stored with the relay-authored work item head.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct CompanyWorkVerification {
    /// Reviewer decision.
    pub verdict: CompanyWorkVerdict,
    /// Why the decision was made.
    pub reason: String,
    /// Evidence checked by the reviewer.
    pub evidence: String,
    /// Pubkey of the member who signed the verification action.
    pub reviewer_pubkey: String,
    /// Time at which the relay recorded the verification.
    pub reviewed_at: String,
    /// Member action that produced this verification.
    pub source_action_event_id: String,
}

/// Relay-authored company work item head using kind 30634.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct CompanyWorkItemHead {
    /// Schema version.
    pub schema_version: u8,
    /// Stable work item UUID.
    pub work_item_id: Uuid,
    /// Short work title.
    pub title: String,
    /// Current work state.
    pub status: CompanyWorkStatus,
    /// Exactly one responsible owner for this commitment.
    pub assigned_pubkeys: Vec<String>,
    /// Existing work item reviewer list, used by deliverable workflows.
    pub approver_pubkeys: Vec<String>,
    /// Current deliverable pointers shared with client work items.
    pub deliverables: Vec<DeliverablePointer>,
    /// Person who requested the commitment.
    pub requester_pubkey: String,
    /// Plain-language condition that means the work is done.
    pub done_condition: String,
    /// Optional live company goal linked to this work item.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub goal_id: Option<Uuid>,
    /// Original source message, absent only for standalone Work creation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_event_id: Option<String>,
    /// Current conversation root; standalone work may gain one when moved to a thread.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thread_root_event_id: Option<String>,
    /// Current deliverable or evidence note.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub evidence: Option<String>,
    /// Reason from the most recent status action.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status_reason: Option<String>,
    /// Latest reviewer decision, if one has been recorded.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub verification: Option<CompanyWorkVerification>,
    /// Time when the relay accepted this work item.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub accepted_at: Option<String>,
    /// Current due date in UTC RFC 3339 form.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub due_at: Option<String>,
    /// Member action that produced this current head.
    pub source_action_event_id: String,
}

/// Member-supplied editable fields for a company work item.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct CompanyWorkItemInput {
    /// Schema version.
    pub schema_version: u8,
    /// Stable work item UUID.
    pub work_item_id: Uuid,
    /// Short work title.
    pub title: String,
    /// Current work state. Status changes use a status action.
    pub status: CompanyWorkStatus,
    /// Exactly one responsible owner for this commitment.
    pub assigned_pubkeys: Vec<String>,
    /// Existing work item reviewer list, used by deliverable workflows.
    pub approver_pubkeys: Vec<String>,
    /// Current deliverable pointers shared with client work items.
    pub deliverables: Vec<DeliverablePointer>,
    /// Person who requested the commitment.
    pub requester_pubkey: String,
    /// Plain-language condition that means the work is done.
    pub done_condition: String,
    /// Optional live company goal linked to this work item.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub goal_id: Option<Uuid>,
    /// Original source message, absent only for standalone Work creation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_event_id: Option<String>,
    /// Current conversation root; absent for standalone work not attached to a thread.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thread_root_event_id: Option<String>,
    /// Current deliverable or evidence note.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub evidence: Option<String>,
    /// Optional due date on creation. Later changes use explicit due-date actions.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub due_at: Option<String>,
}

/// Verification payload on a company work-item action.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct CompanyWorkVerificationInput {
    /// Reviewer decision.
    pub verdict: CompanyWorkVerdict,
    /// Why the decision was made.
    pub reason: String,
    /// Evidence checked by the reviewer.
    pub evidence: String,
}

/// Member request to mutate a company work item using kind 47006.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct CompanyWorkItemAction {
    /// Schema version.
    pub schema_version: u8,
    /// Stable work-item UUID.
    pub work_item_id: Uuid,
    /// Requested mutation.
    pub action: CompanyWorkItemActionKind,
    /// Exact current head event id; omitted only on create.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_head_event_id: Option<String>,
    /// Replacement fields for create and update.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub head: Option<CompanyWorkItemInput>,
    /// Target for set_status.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<CompanyWorkStatus>,
    /// Required explanation for set_status.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    /// Required verdict and evidence for verify.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub verification: Option<CompanyWorkVerificationInput>,
    /// Target timestamp for set_due_date.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub due_at: Option<String>,
}

/// Immutable client deliverable version content.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct DeliverableVersion {
    /// Schema version.
    pub schema_version: u8,
    /// Stable client UUID and required `h` channel UUID.
    pub client_id: Uuid,
    /// Parent work-item UUID.
    pub work_item_id: Uuid,
    /// Stable deliverable UUID.
    pub deliverable_id: Uuid,
    /// Monotonically increasing version number for this deliverable.
    pub version: u32,
    /// Expected prior version event id, absent only for the first version.
    pub previous_version_event_id: Option<String>,
    /// SHA-256 digest of the canonical JSON body, excluding the version envelope.
    pub content_digest: String,
    /// SHA-256 digests for attached media, unique and sorted lexicographically.
    pub media_digests: Vec<String>,
    /// Versioned deliverable body, which may include structured text and references.
    pub body: Value,
}

/// Append-only decision on one exact deliverable version.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct DeliverableApproval {
    /// Schema version.
    pub schema_version: u8,
    /// Stable client UUID and required `h` channel UUID.
    pub client_id: Uuid,
    /// Parent work-item UUID.
    pub work_item_id: Uuid,
    /// Stable deliverable UUID.
    pub deliverable_id: Uuid,
    /// Exact immutable version event id being reviewed.
    pub version_event_id: String,
    /// Exact content digest from that version.
    pub content_digest: String,
    /// Digest of the exact ordered media digest list from that version.
    pub media_digest: String,
    /// Decision on the named version.
    pub decision: ApprovalDecision,
    /// Optional review note.
    pub note: Option<String>,
}

/// Immutable proposal revision authored in the internal business channel.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct ProposalVersion {
    /// Schema version.
    pub schema_version: u8,
    /// Stable proposal UUID.
    pub proposal_id: Uuid,
    /// Stable prospective party UUID.
    pub prospect_party_id: Uuid,
    /// Pubkey allowed to accept this proposal revision.
    pub named_acceptor_pubkey: String,
    /// Monotonically increasing proposal revision.
    pub revision: u32,
    /// Expected prior proposal version event id, absent only for the first version.
    pub previous_version_event_id: Option<String>,
    /// Optional expiry time in Unix seconds.
    pub expires_at: Option<i64>,
    /// ISO 4217 currency code.
    pub currency: String,
    /// Scope, quantity, and amount in minor currency units.
    pub lines: Vec<ProposalLine>,
    /// Terms the named acceptor is accepting.
    pub terms: String,
}

/// Relay-authored current pointer for a prospect proposal.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct ProposalHead {
    /// Schema version.
    pub schema_version: u8,
    /// Stable proposal UUID.
    pub proposal_id: Uuid,
    /// Current immutable version event id.
    pub current_version_event_id: String,
    /// SHA-256 digest of the exact current version event content.
    pub current_version_digest: String,
    /// Current revision number.
    pub revision: u32,
    /// Event that produced this head.
    pub source_event_id: String,
}

/// Priced line in a proposal revision.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct ProposalLine {
    /// Stable service UUID, when linked to a service catalog record.
    pub service_id: Option<Uuid>,
    /// Work scope description.
    pub description: String,
    /// Quantity in hundredths, avoiding floating-point money arithmetic.
    pub quantity_hundredths: u32,
    /// Unit amount in minor currency units.
    pub unit_amount_minor: i64,
}

/// Acceptance of one exact proposal version and its idempotent conversion ids.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct ProposalAcceptance {
    /// Schema version.
    pub schema_version: u8,
    /// Stable proposal UUID.
    pub proposal_id: Uuid,
    /// Exact proposal version event id accepted by the named signer.
    pub proposal_version_event_id: String,
    /// SHA-256 digest of that version event's content.
    pub proposal_version_digest: String,
    /// Stable retry key for the conversion.
    pub conversion_id: Uuid,
    /// Client UUID and target private NIP-29 channel UUID.
    pub client_id: Uuid,
    /// Stable work-item UUID to create or link during conversion.
    pub work_item_id: Uuid,
    /// Stable draft-invoice UUID to create or link during conversion.
    pub draft_invoice_id: Uuid,
    /// Evidence captured when the named acceptor confirmed this exact version.
    #[serde(default)]
    pub evidence: Option<ProposalAcceptanceEvidence>,
}

/// Human-readable evidence tied to an exact proposal acceptance event.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct ProposalAcceptanceEvidence {
    /// Name entered by the person accepting the proposal.
    pub accepted_by_name: String,
    /// Unix time shown and confirmed in the acceptance form.
    pub accepted_at: i64,
    /// Evidence reference recorded with the acceptance.
    pub evidence_reference: String,
    /// Confirms acceptance of the exact displayed scope, price, and revision.
    pub exact_terms_confirmed: bool,
}

/// Relay-authored receipt for a completed proposal conversion claim.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct ProposalConversionReceipt {
    /// Schema version.
    pub schema_version: u8,
    /// Stable retry key.
    pub conversion_id: Uuid,
    /// Accepted proposal UUID and exact version event id.
    pub proposal_id: Uuid,
    /// Accepted proposal version event id.
    pub proposal_version_event_id: String,
    /// Stable client UUID and target client channel UUID.
    pub client_id: Uuid,
    /// Stable work-item UUID.
    pub work_item_id: Uuid,
    /// Stable draft-invoice UUID.
    pub draft_invoice_id: Uuid,
    /// Signed source acceptance event id.
    pub acceptance_event_id: String,
}

/// Lifecycle state of an invoice.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InvoiceStatus {
    /// Invoice is editable and excluded from revenue totals.
    Draft,
    /// Invoice is issued and may receive payment evidence.
    Issued,
    /// Invoice was voided with a reason.
    Void,
}

/// Operation represented by an immutable invoice version event.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InvoiceVersionAction {
    /// Initial draft produced by accepting a proposal.
    ProposalAcceptance,
    /// Initial draft created directly for an existing client, with no proposal.
    Create,
    /// Draft terms were edited before issue.
    DraftEdit,
    /// Draft invoice was issued.
    Issue,
    /// Invoice was voided with a reason.
    Void,
}

/// Optional tax rule captured on an invoice version.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct InvoiceTaxLine {
    /// Display label supplied by the business, when present.
    pub label: Option<String>,
    /// Rate in integer basis points. One hundred basis points is one percent.
    pub rate_basis_points: u32,
}

/// Relay-authored current invoice head created from an accepted proposal.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct InvoiceHead {
    /// Schema version.
    pub schema_version: u8,
    /// Stable client UUID and private client channel UUID.
    pub client_id: Uuid,
    /// Stable invoice UUID.
    pub invoice_id: Uuid,
    /// Source proposal and exact accepted version.
    pub proposal_id: Uuid,
    /// Exact accepted proposal version event id.
    pub proposal_version_event_id: String,
    /// Currency inherited from the proposal.
    pub currency: String,
    /// Proposed invoice lines, still in draft state.
    pub lines: Vec<ProposalLine>,
    /// Optional tax rules captured by the business for this invoice.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tax_lines: Vec<InvoiceTaxLine>,
    /// Optional seller tax registration number captured on the invoice.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seller_tax_number: Option<String>,
    /// Optional customer tax registration number captured on the invoice.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub customer_tax_number: Option<String>,
    /// Gross invoice total in integer minor currency units.
    pub total_minor: i64,
    /// Total credit notes applied to this invoice in minor units.
    pub credited_minor: i64,
    /// Total write-offs applied to this invoice in minor units.
    pub written_off_minor: i64,
    /// Net payment evidence less recorded refunds in minor units.
    pub collected_minor: i64,
    /// Remaining amount due after credits, write-offs, and collections.
    pub outstanding_minor: i64,
    /// Count of payment records, including later-refunded payments.
    pub payment_evidence_count: u32,
    /// Current invoice version number.
    pub version: u32,
    /// Exact current invoice version event id.
    pub current_version_event_id: String,
    /// Current lifecycle state.
    pub status: InvoiceStatus,
    /// Due time in Unix seconds when supplied.
    pub due_at: Option<i64>,
    /// Issue time in Unix seconds, absent for a draft.
    pub issued_at: Option<i64>,
    /// Event that produced this head.
    pub source_event_id: String,
}

/// Compatibility alias for callers that used the original draft-only name.
pub type DraftInvoiceHead = InvoiceHead;

/// Immutable invoice version event and the member action that produced it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct InvoiceVersion {
    /// Schema version.
    pub schema_version: u8,
    /// Stable client UUID and private client channel UUID.
    pub client_id: Uuid,
    /// Stable invoice UUID.
    pub invoice_id: Uuid,
    /// Monotonically increasing invoice version number.
    pub version: u32,
    /// Exact prior invoice version event id, absent only for proposal acceptance.
    pub previous_version_event_id: Option<String>,
    /// Exact accepted proposal revision, when the invoice originated from one.
    pub proposal_version_event_id: Option<String>,
    /// Expected relay-signed invoice head event id for a mutation.
    pub expected_head_event_id: Option<String>,
    /// Operation represented by this version.
    pub action: InvoiceVersionAction,
    /// ISO 4217 currency code.
    pub currency: String,
    /// Invoice line items before configured tax.
    pub lines: Vec<ProposalLine>,
    /// Optional tax rules captured by the business for this invoice.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tax_lines: Vec<InvoiceTaxLine>,
    /// Optional seller tax registration number captured on the invoice.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seller_tax_number: Option<String>,
    /// Optional customer tax registration number captured on the invoice.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub customer_tax_number: Option<String>,
    /// Checked gross sum of line totals and configured tax in minor units.
    pub total_minor: i64,
    /// Invoice lifecycle state after this operation.
    pub status: InvoiceStatus,
    /// Due time in Unix seconds when supplied.
    pub due_at: Option<i64>,
    /// Required reason for a void operation.
    pub void_reason: Option<String>,
}

/// Evidence that a client payment has already been received outside Colony.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct PaymentEvidence {
    /// Schema version.
    pub schema_version: u8,
    /// Stable client UUID and private client channel UUID.
    pub client_id: Uuid,
    /// Invoice that received the payment.
    pub invoice_id: Uuid,
    /// Stable payment evidence UUID.
    pub payment_id: Uuid,
    /// `manual` or the name of the external provider that supplied the evidence.
    pub provider: String,
    /// Provider transaction reference when one exists.
    pub provider_reference: Option<String>,
    /// Positive payment amount in integer minor units.
    pub amount_minor: i64,
    /// ISO 4217 currency code.
    pub currency: String,
    /// Time the payment was received in Unix seconds.
    pub occurred_at: i64,
    /// Non-secret reference to the supporting evidence.
    pub evidence_ref: String,
    /// Exact relay-signed invoice head used for validation.
    pub expected_invoice_head_event_id: String,
}

/// Type of invoice adjustment represented by an evidence record.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MoneyAdjustmentType {
    /// Reduces the invoice amount and outstanding balance.
    CreditNote,
    /// Records a refund already made outside Colony against a client credit.
    Refund,
    /// Reduces the outstanding balance without recording a cash payment.
    WriteOff,
}

/// Evidence for a credit note, completed refund, or write-off.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct MoneyAdjustment {
    /// Schema version.
    pub schema_version: u8,
    /// Stable client UUID and private client channel UUID.
    pub client_id: Uuid,
    /// Invoice affected by this adjustment.
    pub invoice_id: Uuid,
    /// Stable adjustment UUID.
    pub adjustment_id: Uuid,
    /// Adjustment operation.
    pub adjustment_type: MoneyAdjustmentType,
    /// Positive amount in integer minor units.
    pub amount_minor: i64,
    /// ISO 4217 currency code.
    pub currency: String,
    /// Date the credit, refund, or write-off took effect in Unix seconds.
    pub occurred_at: i64,
    /// Required explanation for the adjustment.
    pub reason: String,
    /// Non-secret reference to the supporting evidence.
    pub evidence_ref: String,
    /// Exact relay-signed invoice head used for validation.
    pub expected_invoice_head_event_id: String,
}

/// Action in the overdue follow-up review lifecycle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MoneyFollowUpActionKind {
    /// Create a follow-up draft for review.
    Draft,
    /// Move the current draft into review.
    Review,
    /// Approve the draft and record intent only.
    Approve,
}

/// Current state of an overdue follow-up intent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MoneyFollowUpStatus {
    /// Draft has not been submitted for review.
    Draft,
    /// Draft is awaiting an owner or admin decision.
    InReview,
    /// An owner or admin approved intent to follow up.
    Approved,
}

/// Member command for drafting, reviewing, or approving a follow-up intent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct MoneyFollowUpAction {
    /// Schema version.
    pub schema_version: u8,
    /// Stable client UUID and private client channel UUID.
    pub client_id: Uuid,
    /// Invoice with an overdue outstanding balance.
    pub invoice_id: Uuid,
    /// Stable follow-up UUID.
    pub follow_up_id: Uuid,
    /// Lifecycle action.
    pub action: MoneyFollowUpActionKind,
    /// Exact prior relay-signed follow-up head, absent only when drafting.
    pub expected_head_event_id: Option<String>,
    /// Exact relay-signed invoice head that proves the balance is still overdue.
    pub expected_invoice_head_event_id: String,
    /// Intended follow-up time in Unix seconds when supplied.
    pub due_at: Option<i64>,
    /// Human-reviewed draft text; persisted exactly and never sent automatically.
    pub draft_content: String,
}

/// Relay-signed current state of one follow-up intent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct MoneyFollowUpHead {
    /// Schema version.
    pub schema_version: u8,
    /// Stable client UUID and private client channel UUID.
    pub client_id: Uuid,
    /// Invoice with an overdue outstanding balance.
    pub invoice_id: Uuid,
    /// Stable follow-up UUID.
    pub follow_up_id: Uuid,
    /// Current lifecycle state.
    pub status: MoneyFollowUpStatus,
    /// Current immutable follow-up version number.
    pub version: u32,
    /// Exact current follow-up command event id.
    pub current_version_event_id: String,
    /// Intended follow-up time in Unix seconds when supplied.
    pub due_at: Option<i64>,
    /// Human-reviewed draft text; no external message is sent.
    pub draft_content: String,
    /// Whether an approval represents intent only.
    pub approval_intent_only: bool,
    /// Pubkey that approved the follow-up intent, when approved.
    pub approved_by_pubkey: Option<String>,
    /// Approval time in Unix seconds, when approved.
    pub approved_at: Option<i64>,
    /// Event that produced this head.
    pub source_event_id: String,
}

/// Validate that a client UUID is exactly the private channel UUID in its `h` tag.
pub fn validate_client_channel(
    client_id: Uuid,
    channel_id: Uuid,
) -> Result<(), BusinessRecordError> {
    if client_id == channel_id {
        Ok(())
    } else {
        Err(BusinessRecordError::ClientChannelMismatch)
    }
}

/// Build a d-tag for one client-scoped record coordinate.
pub fn client_d_tag(client_id: Uuid, record_type: &str, record_id: Uuid) -> String {
    format!("client:{client_id}:{record_type}:{record_id}")
}

/// Build the company-wide coordinate for one channel-scoped company work item.
pub fn company_work_d_tag(work_item_id: Uuid) -> String {
    format!("company:work:{work_item_id}")
}

/// Check a supplied d-tag against the company work-item coordinate.
pub fn validate_company_work_d_tag(
    actual: &str,
    work_item_id: Uuid,
) -> Result<(), BusinessRecordError> {
    if actual == company_work_d_tag(work_item_id) {
        Ok(())
    } else {
        Err(BusinessRecordError::DTagMismatch)
    }
}

/// Build a d-tag for a prospect or business-level record before client conversion.
pub fn business_d_tag(community_id: Uuid, record_type: &str, record_id: Uuid) -> String {
    format!("business:{community_id}:{record_type}:{record_id}")
}

/// Build the immutable d-tag for one proposal revision.
pub fn proposal_version_d_tag(community_id: Uuid, proposal_id: Uuid, revision: u32) -> String {
    format!(
        "{}:version:{revision}",
        business_d_tag(community_id, "proposal", proposal_id)
    )
}

/// Build the namespaced d-tag for a prospect record.
pub fn prospect_d_tag(community_id: Uuid, prospect_id: Uuid) -> String {
    business_d_tag(community_id, "prospect", prospect_id)
}

/// Build the immutable d-tag for one client deliverable revision.
pub fn deliverable_version_d_tag(client_id: Uuid, deliverable_id: Uuid, revision: u32) -> String {
    format!(
        "{}:version:{revision}",
        client_d_tag(client_id, "deliverable", deliverable_id)
    )
}

/// Build the append-only d-tag for a decision on one exact deliverable version.
pub fn deliverable_approval_d_tag(client_id: Uuid, version_event_id: &str) -> String {
    format!("client:{client_id}:deliverable-approval:{version_event_id}")
}

/// Build the current relay-signed head coordinate for one client invoice.
pub fn invoice_head_d_tag(client_id: Uuid, invoice_id: Uuid) -> String {
    client_d_tag(client_id, "invoice", invoice_id)
}

/// Build the immutable d-tag for one client invoice revision.
pub fn invoice_version_d_tag(client_id: Uuid, invoice_id: Uuid, version: u32) -> String {
    format!(
        "{}:version:{version}",
        invoice_head_d_tag(client_id, invoice_id)
    )
}

/// Build the append-only d-tag for one payment evidence record.
pub fn payment_d_tag(client_id: Uuid, payment_id: Uuid) -> String {
    client_d_tag(client_id, "payment", payment_id)
}

/// Build the append-only d-tag for one money adjustment record.
pub fn money_adjustment_d_tag(client_id: Uuid, adjustment_id: Uuid) -> String {
    client_d_tag(client_id, "money-adjustment", adjustment_id)
}

/// Build the follow-up command and head coordinate for one client record.
pub fn money_follow_up_d_tag(client_id: Uuid, follow_up_id: Uuid) -> String {
    client_d_tag(client_id, "money-follow-up", follow_up_id)
}

/// Return the checked sum of invoice lines in integer minor currency units.
pub fn invoice_lines_total_minor(lines: &[ProposalLine]) -> Option<i64> {
    lines.iter().try_fold(0_i64, |total, line| {
        let line_total = i64::from(line.quantity_hundredths)
            .checked_mul(line.unit_amount_minor)?
            .checked_div(100)?;
        total.checked_add(line_total)
    })
}

/// Return checked tax in integer minor units, rounded per invoice line and tax rule.
///
/// Each line's net amount uses the existing hundredths quantity floor. Tax is
/// then rounded to the nearest minor unit per line using half-up rounding.
pub fn invoice_tax_total_minor(
    lines: &[ProposalLine],
    tax_lines: &[InvoiceTaxLine],
) -> Option<i64> {
    lines.iter().try_fold(0_i64, |total, line| {
        let line_minor = i128::from(line.quantity_hundredths)
            .checked_mul(i128::from(line.unit_amount_minor))?
            .checked_div(100)?;
        if line_minor < 0 {
            return None;
        }
        let line_minor = i64::try_from(line_minor).ok()?;
        tax_lines.iter().try_fold(total, |tax_total, tax_line| {
            let tax_minor = i128::from(line_minor)
                .checked_mul(i128::from(tax_line.rate_basis_points))?
                .checked_add(5_000)?
                .checked_div(10_000)?;
            tax_total.checked_add(i64::try_from(tax_minor).ok()?)
        })
    })
}

/// Return the checked invoice gross total in integer minor units.
pub fn invoice_total_minor(lines: &[ProposalLine], tax_lines: &[InvoiceTaxLine]) -> Option<i64> {
    invoice_lines_total_minor(lines)?.checked_add(invoice_tax_total_minor(lines, tax_lines)?)
}

/// Return whether a currency uses the uppercase three-letter ISO code form.
pub fn is_iso_currency_code(currency: &str) -> bool {
    currency.len() == 3 && currency.bytes().all(|byte| byte.is_ascii_uppercase())
}

/// Check a supplied d-tag against the exact client-scoped coordinate.
pub fn validate_client_d_tag(
    actual: &str,
    client_id: Uuid,
    record_type: &str,
    record_id: Uuid,
) -> Result<(), BusinessRecordError> {
    if actual == client_d_tag(client_id, record_type, record_id) {
        Ok(())
    } else {
        Err(BusinessRecordError::DTagMismatch)
    }
}

/// Check a supplied d-tag against the exact business-scoped coordinate.
pub fn validate_business_d_tag(
    actual: &str,
    community_id: Uuid,
    record_type: &str,
    record_id: Uuid,
) -> Result<(), BusinessRecordError> {
    if actual == business_d_tag(community_id, record_type, record_id) {
        Ok(())
    } else {
        Err(BusinessRecordError::DTagMismatch)
    }
}

/// Validate the h and d coordinates against a parsed business command.
pub fn validate_business_command_scope(
    community_id: Uuid,
    channel_id: Uuid,
    d_tag: &str,
    command: &BusinessCommand,
) -> Result<(), BusinessRecordError> {
    match command {
        BusinessCommand::PartyAction(value) => {
            validate_business_d_tag(d_tag, community_id, "party", value.party_id)
        }
        BusinessCommand::ServiceAction(value) => {
            if value.service.service_id != value.service_id {
                return Err(BusinessRecordError::InvalidContent);
            }
            validate_business_d_tag(d_tag, community_id, "service", value.service_id)
        }
        BusinessCommand::ProspectAction(value) => {
            if value.prospect.prospect_id != value.prospect_id
                || value.prospect.party.party_id != value.prospect_id
            {
                return Err(BusinessRecordError::InvalidContent);
            }
            if d_tag == prospect_d_tag(community_id, value.prospect_id) {
                Ok(())
            } else {
                Err(BusinessRecordError::DTagMismatch)
            }
        }
        BusinessCommand::ClientAction(value) => {
            validate_client_channel(value.client_id, channel_id)?;
            if value.head.client_id != value.client_id
                || value.head.schema_version != value.schema_version
            {
                return Err(BusinessRecordError::InvalidContent);
            }
            validate_client_d_tag(d_tag, value.client_id, "client", value.client_id)
        }
        BusinessCommand::WorkItemAction(value) => {
            validate_client_channel(value.client_id, channel_id)?;
            if value.head.schema_version != value.schema_version
                || value.head.client_id != value.client_id
                || value.head.work_item_id != value.work_item_id
            {
                return Err(BusinessRecordError::InvalidContent);
            }
            validate_client_d_tag(d_tag, value.client_id, "work", value.work_item_id)
        }
        BusinessCommand::CompanyWorkItemAction(value) => {
            validate_company_work_item_action(value)?;
            validate_company_work_d_tag(d_tag, value.work_item_id)
        }
        BusinessCommand::ProposalVersion(value) => {
            if d_tag == proposal_version_d_tag(community_id, value.proposal_id, value.revision) {
                Ok(())
            } else {
                Err(BusinessRecordError::DTagMismatch)
            }
        }
        BusinessCommand::ProposalAcceptance(value) => {
            validate_business_d_tag(d_tag, community_id, "conversion", value.conversion_id)
        }
        BusinessCommand::DeliverableVersion(value) => {
            validate_client_channel(value.client_id, channel_id)?;
            if d_tag
                == deliverable_version_d_tag(value.client_id, value.deliverable_id, value.version)
            {
                Ok(())
            } else {
                Err(BusinessRecordError::DTagMismatch)
            }
        }
        BusinessCommand::DeliverableApproval(value) => {
            validate_client_channel(value.client_id, channel_id)?;
            if d_tag == deliverable_approval_d_tag(value.client_id, &value.version_event_id) {
                Ok(())
            } else {
                Err(BusinessRecordError::DTagMismatch)
            }
        }
        BusinessCommand::InvoiceVersion(value) => {
            validate_client_channel(value.client_id, channel_id)?;
            if d_tag == invoice_version_d_tag(value.client_id, value.invoice_id, value.version) {
                Ok(())
            } else {
                Err(BusinessRecordError::DTagMismatch)
            }
        }
        BusinessCommand::PaymentEvidence(value) => {
            validate_client_channel(value.client_id, channel_id)?;
            validate_client_d_tag(d_tag, value.client_id, "payment", value.payment_id)
        }
        BusinessCommand::MoneyAdjustment(value) => {
            validate_client_channel(value.client_id, channel_id)?;
            validate_client_d_tag(
                d_tag,
                value.client_id,
                "money-adjustment",
                value.adjustment_id,
            )
        }
        BusinessCommand::MoneyFollowUp(value) => {
            validate_client_channel(value.client_id, channel_id)?;
            validate_client_d_tag(
                d_tag,
                value.client_id,
                "money-follow-up",
                value.follow_up_id,
            )
        }
    }
}

/// Validate an event id or SHA-256 digest as exactly 32 lowercase hex bytes.
pub fn validate_hex_reference(value: &str) -> Result<(), BusinessRecordError> {
    if value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        Ok(())
    } else {
        Err(BusinessRecordError::InvalidHexReference)
    }
}

/// Validate a company work-item action's shape and payload rules.
pub fn validate_company_work_item_action(
    action: &CompanyWorkItemAction,
) -> Result<(), BusinessRecordError> {
    if action.schema_version != BUSINESS_RECORD_SCHEMA_VERSION {
        return Err(BusinessRecordError::UnsupportedSchemaVersion);
    }
    if action.work_item_id.is_nil() {
        return Err(BusinessRecordError::Invalid("workItemId must not be nil"));
    }
    if let Some(expected) = action.expected_head_event_id.as_deref() {
        validate_hex_reference(expected)?;
    }

    let no_payloads = action.head.is_none()
        && action.status.is_none()
        && action.reason.is_none()
        && action.verification.is_none()
        && action.due_at.is_none();
    match action.action {
        CompanyWorkItemActionKind::Create => {
            if action.expected_head_event_id.is_some()
                || action.status.is_some()
                || action.reason.is_some()
                || action.verification.is_some()
                || action.due_at.is_some()
            {
                return Err(BusinessRecordError::Invalid(
                    "create must contain only a head and must omit expectedHeadEventId",
                ));
            }
            let head = action
                .head
                .as_ref()
                .ok_or(BusinessRecordError::Invalid("create head is required"))?;
            validate_company_work_item_input(action.work_item_id, head)?;
            if head.status != CompanyWorkStatus::Active {
                return Err(BusinessRecordError::Invalid(
                    "company work items must be created as active",
                ));
            }
            if head.source_event_id.is_some() != head.thread_root_event_id.is_some() {
                return Err(BusinessRecordError::Invalid(
                    "sourceEventId and threadRootEventId must be supplied together",
                ));
            }
        }
        CompanyWorkItemActionKind::Update => {
            require_company_work_expected_head(action)?;
            if action.status.is_some()
                || action.reason.is_some()
                || action.verification.is_some()
                || action.due_at.is_some()
            {
                return Err(BusinessRecordError::Invalid(
                    "update must contain only a head and expectedHeadEventId",
                ));
            }
            let head = action
                .head
                .as_ref()
                .ok_or(BusinessRecordError::Invalid("update head is required"))?;
            validate_company_work_item_input(action.work_item_id, head)?;
            if head.source_event_id.is_some() && head.thread_root_event_id.is_none() {
                return Err(BusinessRecordError::Invalid(
                    "sourceEventId requires a threadRootEventId",
                ));
            }
        }
        CompanyWorkItemActionKind::SetStatus => {
            require_company_work_expected_head(action)?;
            if action.head.is_some() || action.verification.is_some() || action.due_at.is_some() {
                return Err(BusinessRecordError::Invalid(
                    "set_status cannot contain head or verification payloads",
                ));
            }
            if !matches!(
                action.status,
                Some(
                    CompanyWorkStatus::Active
                        | CompanyWorkStatus::Paused
                        | CompanyWorkStatus::Blocked
                        | CompanyWorkStatus::DoneUnverified
                )
            ) {
                return Err(BusinessRecordError::Invalid(
                    "set_status target must be active, paused, blocked, or done_unverified",
                ));
            }
            validate_work_item_reason(action.reason.as_deref())?;
        }
        CompanyWorkItemActionKind::Verify => {
            require_company_work_expected_head(action)?;
            if action.head.is_some()
                || action.status.is_some()
                || action.reason.is_some()
                || action.due_at.is_some()
            {
                return Err(BusinessRecordError::Invalid(
                    "verify must contain only a verification and expectedHeadEventId",
                ));
            }
            let verification = action
                .verification
                .as_ref()
                .ok_or(BusinessRecordError::Invalid("verification is required"))?;
            validate_work_item_reason(Some(&verification.reason))?;
            validate_work_item_evidence(Some(&verification.evidence))?;
        }
        CompanyWorkItemActionKind::Archive | CompanyWorkItemActionKind::Restore => {
            require_company_work_expected_head(action)?;
            if !no_payloads {
                return Err(BusinessRecordError::Invalid(
                    "archive and restore cannot contain a work-item payload",
                ));
            }
        }
        CompanyWorkItemActionKind::SetDueDate => {
            require_company_work_expected_head(action)?;
            if action.head.is_some()
                || action.status.is_some()
                || action.reason.is_some()
                || action.verification.is_some()
            {
                return Err(BusinessRecordError::Invalid(
                    "set_due_date must contain only dueAt and expectedHeadEventId",
                ));
            }
            let due_at = action
                .due_at
                .as_deref()
                .ok_or(BusinessRecordError::Invalid("dueAt is required"))?;
            validate_utc_rfc3339(due_at)?;
        }
        CompanyWorkItemActionKind::ClearDueDate => {
            require_company_work_expected_head(action)?;
            if !no_payloads {
                return Err(BusinessRecordError::Invalid(
                    "clear_due_date cannot contain a work-item payload",
                ));
            }
        }
    }
    Ok(())
}

fn require_company_work_expected_head(
    action: &CompanyWorkItemAction,
) -> Result<(), BusinessRecordError> {
    if action.expected_head_event_id.is_some() {
        Ok(())
    } else {
        Err(BusinessRecordError::Invalid(
            "expectedHeadEventId is required for this action",
        ))
    }
}

fn validate_company_work_item_input(
    work_item_id: Uuid,
    head: &CompanyWorkItemInput,
) -> Result<(), BusinessRecordError> {
    if head.schema_version != BUSINESS_RECORD_SCHEMA_VERSION {
        return Err(BusinessRecordError::UnsupportedSchemaVersion);
    }
    if head.work_item_id != work_item_id || head.work_item_id.is_nil() {
        return Err(BusinessRecordError::Invalid(
            "head.workItemId must match workItemId",
        ));
    }
    if head.title.trim().is_empty() || head.title.chars().count() > MAX_WORK_ITEM_TITLE_CHARS {
        return Err(BusinessRecordError::Invalid(
            "title must contain 1 to 180 characters",
        ));
    }
    if head.done_condition.trim().is_empty()
        || head.done_condition.chars().count() > MAX_WORK_ITEM_DONE_CONDITION_CHARS
    {
        return Err(BusinessRecordError::Invalid(
            "doneCondition must contain 1 to 1000 characters",
        ));
    }
    validate_hex_reference(&head.requester_pubkey)?;
    if head.assigned_pubkeys.len() != 1 {
        return Err(BusinessRecordError::Invalid(
            "company work items require exactly one owner",
        ));
    }
    validate_unique_pubkeys(&head.assigned_pubkeys)?;
    validate_unique_pubkeys(&head.approver_pubkeys)?;
    if let Some(goal_id) = head.goal_id {
        if goal_id.is_nil() {
            return Err(BusinessRecordError::Invalid("goalId must not be nil"));
        }
    }
    for event_id in [
        head.source_event_id.as_deref(),
        head.thread_root_event_id.as_deref(),
    ]
    .into_iter()
    .flatten()
    {
        validate_hex_reference(event_id)?;
    }
    validate_work_item_evidence(head.evidence.as_deref())?;
    if let Some(due_at) = head.due_at.as_deref() {
        validate_utc_rfc3339(due_at)?;
    }
    Ok(())
}

/// Validate a timestamp is RFC 3339 and explicitly uses the UTC `Z` suffix.
pub fn validate_utc_rfc3339(value: &str) -> Result<(), BusinessRecordError> {
    if !value.ends_with('Z') || chrono::DateTime::parse_from_rfc3339(value).is_err() {
        return Err(BusinessRecordError::Invalid(
            "timestamp must be valid RFC 3339 UTC ending in Z",
        ));
    }
    Ok(())
}

fn validate_unique_pubkeys(pubkeys: &[String]) -> Result<(), BusinessRecordError> {
    let mut unique = BTreeSet::new();
    for pubkey in pubkeys {
        validate_hex_reference(pubkey)?;
        if !unique.insert(pubkey) {
            return Err(BusinessRecordError::Invalid(
                "pubkey lists must not contain duplicates",
            ));
        }
    }
    Ok(())
}

fn validate_work_item_reason(reason: Option<&str>) -> Result<(), BusinessRecordError> {
    let reason = reason.ok_or(BusinessRecordError::Invalid("reason is required"))?;
    if reason.trim().is_empty() || reason.chars().count() > MAX_WORK_ITEM_REASON_CHARS {
        return Err(BusinessRecordError::Invalid(
            "reason must contain 1 to 1000 characters",
        ));
    }
    Ok(())
}

fn validate_work_item_evidence(evidence: Option<&str>) -> Result<(), BusinessRecordError> {
    if let Some(evidence) = evidence {
        if evidence.trim().is_empty() || evidence.chars().count() > MAX_WORK_ITEM_EVIDENCE_CHARS {
            return Err(BusinessRecordError::Invalid(
                "evidence must contain 1 to 2000 characters",
            ));
        }
    }
    Ok(())
}

/// Return whether an approval still targets the current deliverable version.
///
/// A later version changes the work head's current event id, so all earlier
/// approvals immediately cease to represent current approval state.
pub fn approval_matches_current_version(
    approval: &DeliverableApproval,
    current: Option<&DeliverablePointer>,
) -> bool {
    let Some(current) = current else {
        return false;
    };
    approval.version_event_id == current.version_event_id
        && approval.content_digest == current.content_digest
        && approval.media_digest == current.media_digest
}

/// Parse a member-authored business command according to its Nostr kind.
pub fn parse_business_command(
    kind: u32,
    content: &str,
) -> Result<BusinessCommand, BusinessRecordError> {
    macro_rules! parse {
        ($type:ty, $variant:ident) => {
            serde_json::from_str::<$type>(content)
                .map(BusinessCommand::$variant)
                .map_err(|_| BusinessRecordError::InvalidContent)
        };
    }
    macro_rules! parse_boxed {
        ($type:ty, $variant:ident) => {
            serde_json::from_str::<$type>(content)
                .map(|value| BusinessCommand::$variant(Box::new(value)))
                .map_err(|_| BusinessRecordError::InvalidContent)
        };
    }

    let command = match kind {
        crate::kind::KIND_PARTY_ACTION => parse!(PartyAction, PartyAction)?,
        crate::kind::KIND_SERVICE_ACTION => parse!(ServiceAction, ServiceAction)?,
        crate::kind::KIND_PROSPECT_ACTION => parse_boxed!(ProspectAction, ProspectAction)?,
        crate::kind::KIND_CLIENT_ACTION => parse!(ClientAction, ClientAction)?,
        crate::kind::KIND_WORK_ITEM_ACTION => parse_work_item_action(content)?,
        crate::kind::KIND_PROPOSAL_VERSION => parse!(ProposalVersion, ProposalVersion)?,
        crate::kind::KIND_PROPOSAL_ACCEPTANCE => parse!(ProposalAcceptance, ProposalAcceptance)?,
        crate::kind::KIND_DELIVERABLE_VERSION => parse!(DeliverableVersion, DeliverableVersion)?,
        crate::kind::KIND_DELIVERABLE_APPROVAL => parse!(DeliverableApproval, DeliverableApproval)?,
        crate::kind::KIND_INVOICE_VERSION => parse!(InvoiceVersion, InvoiceVersion)?,
        crate::kind::KIND_PAYMENT => parse!(PaymentEvidence, PaymentEvidence)?,
        crate::kind::KIND_MONEY_ADJUSTMENT => parse!(MoneyAdjustment, MoneyAdjustment)?,
        crate::kind::KIND_MONEY_FOLLOW_UP => parse!(MoneyFollowUpAction, MoneyFollowUp)?,
        _ => return Err(BusinessRecordError::UnsupportedKind),
    };

    let schema_version = match &command {
        BusinessCommand::PartyAction(value) => value.schema_version,
        BusinessCommand::ServiceAction(value) => value.schema_version,
        BusinessCommand::ProspectAction(value) => value.schema_version,
        BusinessCommand::ClientAction(value) => value.schema_version,
        BusinessCommand::WorkItemAction(value) => value.schema_version,
        BusinessCommand::CompanyWorkItemAction(value) => value.schema_version,
        BusinessCommand::ProposalVersion(value) => value.schema_version,
        BusinessCommand::ProposalAcceptance(value) => value.schema_version,
        BusinessCommand::DeliverableVersion(value) => value.schema_version,
        BusinessCommand::DeliverableApproval(value) => value.schema_version,
        BusinessCommand::InvoiceVersion(value) => value.schema_version,
        BusinessCommand::PaymentEvidence(value) => value.schema_version,
        BusinessCommand::MoneyAdjustment(value) => value.schema_version,
        BusinessCommand::MoneyFollowUp(value) => value.schema_version,
    };
    if schema_version != BUSINESS_RECORD_SCHEMA_VERSION {
        return Err(BusinessRecordError::UnsupportedSchemaVersion);
    }
    Ok(command)
}

fn parse_work_item_action(content: &str) -> Result<BusinessCommand, BusinessRecordError> {
    let value =
        serde_json::from_str::<Value>(content).map_err(|_| BusinessRecordError::InvalidContent)?;
    if value.get("clientId").is_some() {
        serde_json::from_value::<WorkItemAction>(value)
            .map(BusinessCommand::WorkItemAction)
            .map_err(|_| BusinessRecordError::InvalidContent)
    } else {
        serde_json::from_value::<CompanyWorkItemAction>(value)
            .map(BusinessCommand::CompanyWorkItemAction)
            .map_err(|_| BusinessRecordError::InvalidContent)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn example_prospect_action() -> ProspectAction {
        let party_id = Uuid::from_u128(12);
        let prospect_id = party_id;
        ProspectAction {
            schema_version: BUSINESS_RECORD_SCHEMA_VERSION,
            prospect_id,
            action: RecordAction::Create,
            expected_head_event_id: None,
            prospect: ProspectRecordInput {
                prospect_id,
                party: PartyRecord {
                    party_id,
                    party_type: "organization".into(),
                    display_name: "Example Studio".into(),
                    external_ids: Vec::new(),
                },
                industry: "Professional Services".into(),
                vertical: "Marketing Agency".into(),
                fit_score: None,
                origin: None,
                discovery_tags: Vec::new(),
                potential_monthly_value_minor: None,
                website: Some("https://example.test".into()),
                contact_name: None,
                location: Some("Cape Town".into()),
                email: None,
                phone: None,
                evidence: Vec::new(),
                last_verified_at: None,
                qualification: ProspectQualification::Unreviewed,
                saved: false,
                stage: ProspectStage::Qualified,
                lost_reason: None,
            },
            activity: None,
        }
    }

    fn version(event_id: &str, digest: &str) -> DeliverablePointer {
        DeliverablePointer {
            deliverable_id: Uuid::from_u128(3),
            version_event_id: event_id.to_owned(),
            content_digest: digest.to_owned(),
            media_digest: digest.to_owned(),
            version_digest: digest.to_owned(),
        }
    }

    fn approval(event_id: &str, digest: &str) -> DeliverableApproval {
        DeliverableApproval {
            schema_version: BUSINESS_RECORD_SCHEMA_VERSION,
            client_id: Uuid::from_u128(1),
            work_item_id: Uuid::from_u128(2),
            deliverable_id: Uuid::from_u128(3),
            version_event_id: event_id.to_owned(),
            content_digest: digest.to_owned(),
            media_digest: digest.to_owned(),
            decision: ApprovalDecision::Approved,
            note: None,
        }
    }

    fn company_work_input(work_item_id: Uuid) -> CompanyWorkItemInput {
        CompanyWorkItemInput {
            schema_version: BUSINESS_RECORD_SCHEMA_VERSION,
            work_item_id,
            title: "Prepare the launch plan".into(),
            status: CompanyWorkStatus::Active,
            assigned_pubkeys: vec!["a".repeat(64)],
            approver_pubkeys: Vec::new(),
            deliverables: Vec::new(),
            requester_pubkey: "b".repeat(64),
            done_condition: "The launch plan is approved by the team.".into(),
            goal_id: Some(Uuid::from_u128(8)),
            source_event_id: None,
            thread_root_event_id: None,
            evidence: None,
            due_at: None,
        }
    }

    fn company_work_action(
        work_item_id: Uuid,
        action: CompanyWorkItemActionKind,
        expected_head_event_id: Option<String>,
        head: Option<CompanyWorkItemInput>,
    ) -> CompanyWorkItemAction {
        CompanyWorkItemAction {
            schema_version: BUSINESS_RECORD_SCHEMA_VERSION,
            work_item_id,
            action,
            expected_head_event_id,
            head,
            status: None,
            reason: None,
            verification: None,
            due_at: None,
        }
    }

    #[test]
    fn namespaced_client_records_bind_to_the_channel_id() {
        let client_id = Uuid::from_u128(1);
        let other_client_id = Uuid::from_u128(2);
        assert_eq!(validate_client_channel(client_id, client_id), Ok(()));
        assert_eq!(
            validate_client_channel(other_client_id, client_id),
            Err(BusinessRecordError::ClientChannelMismatch)
        );
        assert_eq!(
            validate_client_d_tag(
                &client_d_tag(other_client_id, "work", Uuid::from_u128(3)),
                client_id,
                "work",
                Uuid::from_u128(3),
            ),
            Err(BusinessRecordError::DTagMismatch)
        );
    }

    #[test]
    fn business_command_scope_denies_cross_client_channel_binding() {
        let client_id = Uuid::from_u128(1);
        let other_channel_id = Uuid::from_u128(2);
        let action = ClientAction {
            schema_version: BUSINESS_RECORD_SCHEMA_VERSION,
            client_id,
            action: RecordAction::Create,
            expected_head_event_id: None,
            head: ClientHeadInput {
                schema_version: BUSINESS_RECORD_SCHEMA_VERSION,
                client_id,
                party_id: Uuid::from_u128(3),
                display_name: "Example Client".into(),
                approver_pubkeys: Vec::new(),
                status: "active".into(),
            },
        };

        assert_eq!(
            validate_business_command_scope(
                Uuid::from_u128(4),
                other_channel_id,
                &client_d_tag(client_id, "client", client_id),
                &BusinessCommand::ClientAction(action),
            ),
            Err(BusinessRecordError::ClientChannelMismatch)
        );
    }

    #[test]
    fn exact_version_approval_is_invalidated_by_a_new_version() {
        let first = version("a".repeat(64).as_str(), "b".repeat(64).as_str());
        let second = version("c".repeat(64).as_str(), "d".repeat(64).as_str());
        let prior_approval = approval(&first.version_event_id, &first.version_digest);

        assert!(approval_matches_current_version(
            &prior_approval,
            Some(&first)
        ));
        assert!(!approval_matches_current_version(
            &prior_approval,
            Some(&second)
        ));
        assert!(!approval_matches_current_version(&prior_approval, None));
    }

    #[test]
    fn client_and_business_d_tags_include_the_outer_identifier() {
        let id = Uuid::from_u128(9);
        let other = Uuid::from_u128(10);
        assert_eq!(
            client_d_tag(id, "work", other),
            format!("client:{id}:work:{other}")
        );
        assert_eq!(
            business_d_tag(id, "proposal", other),
            format!("business:{id}:proposal:{other}")
        );
    }

    #[test]
    fn service_and_prospect_commands_parse_and_bind_to_business_coordinates() {
        let community_id = Uuid::from_u128(20);
        let service_id = Uuid::from_u128(21);
        let service = ServiceAction {
            schema_version: BUSINESS_RECORD_SCHEMA_VERSION,
            service_id,
            action: RecordAction::Create,
            expected_head_event_id: None,
            service: ServiceRecord {
                service_id,
                name: "Monthly social content".into(),
                description: "Planning and publishing".into(),
                currency: "ZAR".into(),
                monthly_fee_minor: 25_000,
                posts_per_month: 12,
                revision_rounds: 2,
            },
        };
        let parsed_service = parse_business_command(
            crate::kind::KIND_SERVICE_ACTION,
            &serde_json::to_string(&service).expect("serialize service action"),
        )
        .expect("parse service action");
        assert_eq!(
            validate_business_command_scope(
                community_id,
                Uuid::nil(),
                &business_d_tag(community_id, "service", service_id),
                &parsed_service,
            ),
            Ok(())
        );

        let prospect = example_prospect_action();
        let parsed_prospect = parse_business_command(
            crate::kind::KIND_PROSPECT_ACTION,
            &serde_json::to_string(&prospect).expect("serialize prospect action"),
        )
        .expect("parse prospect action");
        assert_eq!(
            validate_business_command_scope(
                community_id,
                Uuid::nil(),
                &prospect_d_tag(community_id, prospect.prospect_id),
                &parsed_prospect,
            ),
            Ok(())
        );
        assert_eq!(
            validate_business_command_scope(
                community_id,
                Uuid::nil(),
                &prospect_d_tag(community_id, Uuid::from_u128(99)),
                &parsed_prospect,
            ),
            Err(BusinessRecordError::DTagMismatch)
        );
    }

    #[test]
    fn proposal_acceptance_without_new_evidence_remains_parseable() {
        let json = serde_json::json!({
            "schemaVersion": BUSINESS_RECORD_SCHEMA_VERSION,
            "proposalId": Uuid::from_u128(1),
            "proposalVersionEventId": "a".repeat(64),
            "proposalVersionDigest": "b".repeat(64),
            "conversionId": Uuid::from_u128(2),
            "clientId": Uuid::from_u128(3),
            "workItemId": Uuid::from_u128(4),
            "draftInvoiceId": Uuid::from_u128(5)
        });
        let acceptance: ProposalAcceptance =
            serde_json::from_value(json).expect("deserialize legacy acceptance");
        assert_eq!(acceptance.evidence, None);
    }

    #[test]
    fn money_coordinates_bind_each_record_to_the_client_channel() {
        let client_id = Uuid::from_u128(31);
        let invoice_id = Uuid::from_u128(32);
        let other_client_id = Uuid::from_u128(33);
        let invoice_version = InvoiceVersion {
            schema_version: BUSINESS_RECORD_SCHEMA_VERSION,
            client_id,
            invoice_id,
            version: 2,
            previous_version_event_id: Some("a".repeat(64)),
            proposal_version_event_id: Some("b".repeat(64)),
            expected_head_event_id: Some("c".repeat(64)),
            action: InvoiceVersionAction::DraftEdit,
            currency: "ZAR".into(),
            lines: vec![ProposalLine {
                service_id: None,
                description: "Monthly service".into(),
                quantity_hundredths: 100,
                unit_amount_minor: 12_345,
            }],
            tax_lines: Vec::new(),
            seller_tax_number: None,
            customer_tax_number: None,
            total_minor: 12_345,
            status: InvoiceStatus::Draft,
            due_at: Some(1_800_000_000),
            void_reason: None,
        };
        let parsed_invoice = parse_business_command(
            crate::kind::KIND_INVOICE_VERSION,
            &serde_json::to_string(&invoice_version).expect("serialize invoice version"),
        )
        .expect("parse invoice version");
        assert_eq!(
            validate_business_command_scope(
                client_id,
                client_id,
                &invoice_version_d_tag(client_id, invoice_id, 2),
                &parsed_invoice,
            ),
            Ok(())
        );
        assert_eq!(
            validate_business_command_scope(
                client_id,
                other_client_id,
                &invoice_version_d_tag(client_id, invoice_id, 2),
                &parsed_invoice,
            ),
            Err(BusinessRecordError::ClientChannelMismatch)
        );

        let payment = PaymentEvidence {
            schema_version: BUSINESS_RECORD_SCHEMA_VERSION,
            client_id,
            invoice_id,
            payment_id: Uuid::from_u128(34),
            provider: "manual".into(),
            provider_reference: Some("bank-ref-1".into()),
            amount_minor: 500,
            currency: "ZAR".into(),
            occurred_at: 1_800_000_001,
            evidence_ref: "receipt:one".into(),
            expected_invoice_head_event_id: "d".repeat(64),
        };
        let parsed_payment = parse_business_command(
            crate::kind::KIND_PAYMENT,
            &serde_json::to_string(&payment).expect("serialize payment"),
        )
        .expect("parse payment");
        assert_eq!(
            validate_business_command_scope(
                client_id,
                client_id,
                &payment_d_tag(client_id, payment.payment_id),
                &parsed_payment,
            ),
            Ok(())
        );
        assert_eq!(
            validate_business_command_scope(
                client_id,
                other_client_id,
                &payment_d_tag(client_id, payment.payment_id),
                &parsed_payment,
            ),
            Err(BusinessRecordError::ClientChannelMismatch)
        );

        let adjustment = MoneyAdjustment {
            schema_version: BUSINESS_RECORD_SCHEMA_VERSION,
            client_id,
            invoice_id,
            adjustment_id: Uuid::from_u128(35),
            adjustment_type: MoneyAdjustmentType::CreditNote,
            amount_minor: 100,
            currency: "ZAR".into(),
            occurred_at: 1_800_000_001,
            reason: "Scope reduced".into(),
            evidence_ref: "credit-note:one".into(),
            expected_invoice_head_event_id: "e".repeat(64),
        };
        let parsed_adjustment = parse_business_command(
            crate::kind::KIND_MONEY_ADJUSTMENT,
            &serde_json::to_string(&adjustment).expect("serialize adjustment"),
        )
        .expect("parse adjustment");
        assert_eq!(
            validate_business_command_scope(
                client_id,
                client_id,
                &money_adjustment_d_tag(client_id, adjustment.adjustment_id),
                &parsed_adjustment,
            ),
            Ok(())
        );
        assert_eq!(
            validate_business_command_scope(
                client_id,
                other_client_id,
                &money_adjustment_d_tag(client_id, adjustment.adjustment_id),
                &parsed_adjustment,
            ),
            Err(BusinessRecordError::ClientChannelMismatch)
        );

        let follow_up = MoneyFollowUpAction {
            schema_version: BUSINESS_RECORD_SCHEMA_VERSION,
            client_id,
            invoice_id,
            follow_up_id: Uuid::from_u128(36),
            action: MoneyFollowUpActionKind::Draft,
            expected_head_event_id: None,
            expected_invoice_head_event_id: "f".repeat(64),
            due_at: Some(1_800_000_002),
            draft_content: "Please review the outstanding invoice.".into(),
        };
        let parsed_follow_up = parse_business_command(
            crate::kind::KIND_MONEY_FOLLOW_UP,
            &serde_json::to_string(&follow_up).expect("serialize follow-up"),
        )
        .expect("parse follow-up");
        assert_eq!(
            validate_business_command_scope(
                client_id,
                client_id,
                &money_follow_up_d_tag(client_id, follow_up.follow_up_id),
                &parsed_follow_up,
            ),
            Ok(())
        );
        assert_eq!(
            validate_business_command_scope(
                client_id,
                other_client_id,
                &money_follow_up_d_tag(client_id, follow_up.follow_up_id),
                &parsed_follow_up,
            ),
            Err(BusinessRecordError::ClientChannelMismatch)
        );
    }

    #[test]
    fn money_helpers_use_checked_integer_totals_and_currency_code_form() {
        let lines = [ProposalLine {
            service_id: None,
            description: "Fractional quantity".into(),
            quantity_hundredths: 125,
            unit_amount_minor: 999,
        }];
        assert_eq!(invoice_lines_total_minor(&lines), Some(1_248));
        assert_eq!(
            invoice_lines_total_minor(&[ProposalLine {
                service_id: None,
                description: "Overflow".into(),
                quantity_hundredths: u32::MAX,
                unit_amount_minor: i64::MAX,
            }]),
            None
        );
        assert!(is_iso_currency_code("ZAR"));
        assert!(is_iso_currency_code("USD"));
        assert!(!is_iso_currency_code("zar"));
        assert!(!is_iso_currency_code("US"));
        assert!(!is_iso_currency_code("US1"));
    }

    #[test]
    fn invoice_tax_rounds_half_up_for_each_line_and_rule() {
        let lines = [
            ProposalLine {
                service_id: None,
                description: "First one-minor line".into(),
                quantity_hundredths: 100,
                unit_amount_minor: 1,
            },
            ProposalLine {
                service_id: None,
                description: "Second one-minor line".into(),
                quantity_hundredths: 100,
                unit_amount_minor: 1,
            },
        ];
        let taxes = [InvoiceTaxLine {
            label: Some("Illustrative tax".into()),
            rate_basis_points: 5_000,
        }];
        assert_eq!(invoice_tax_total_minor(&lines, &taxes), Some(2));
        assert_eq!(invoice_total_minor(&lines, &taxes), Some(4));
        assert_eq!(invoice_tax_total_minor(&lines, &[]), Some(0));
        assert_eq!(
            invoice_tax_total_minor(
                &lines[..1],
                &[InvoiceTaxLine {
                    label: None,
                    rate_basis_points: 4_999,
                }]
            ),
            Some(0)
        );
    }

    #[test]
    fn legacy_invoice_version_without_tax_fields_defaults_to_no_tax() {
        let invoice_version = InvoiceVersion {
            schema_version: BUSINESS_RECORD_SCHEMA_VERSION,
            client_id: Uuid::from_u128(31),
            invoice_id: Uuid::from_u128(32),
            version: 1,
            previous_version_event_id: None,
            proposal_version_event_id: Some("a".repeat(64)),
            expected_head_event_id: None,
            action: InvoiceVersionAction::ProposalAcceptance,
            currency: "ZAR".into(),
            lines: vec![ProposalLine {
                service_id: None,
                description: "Legacy line".into(),
                quantity_hundredths: 100,
                unit_amount_minor: 1_000,
            }],
            tax_lines: Vec::new(),
            seller_tax_number: None,
            customer_tax_number: None,
            total_minor: 1_000,
            status: InvoiceStatus::Draft,
            due_at: None,
            void_reason: None,
        };
        let mut legacy = serde_json::to_value(invoice_version).expect("serialize version");
        let object = legacy.as_object_mut().expect("object content");
        object.remove("taxLines");
        object.remove("sellerTaxNumber");
        object.remove("customerTaxNumber");
        let parsed: InvoiceVersion = serde_json::from_value(legacy).expect("legacy version");
        assert!(parsed.tax_lines.is_empty());
        assert_eq!(parsed.seller_tax_number, None);
        assert_eq!(parsed.customer_tax_number, None);
    }

    #[test]
    fn client_work_item_actions_keep_the_existing_payload_shape() {
        let client_id = Uuid::from_u128(50);
        let work_item_id = Uuid::from_u128(51);
        let action = WorkItemAction {
            schema_version: BUSINESS_RECORD_SCHEMA_VERSION,
            client_id,
            work_item_id,
            action: RecordAction::Create,
            expected_head_event_id: None,
            head: WorkItemHeadInput {
                schema_version: BUSINESS_RECORD_SCHEMA_VERSION,
                client_id,
                work_item_id,
                title: "Existing client work".into(),
                status: "open".into(),
                assigned_pubkeys: Vec::new(),
                approver_pubkeys: Vec::new(),
                deliverables: Vec::new(),
            },
        };
        let parsed = parse_business_command(
            crate::kind::KIND_WORK_ITEM_ACTION,
            &serde_json::to_string(&action).expect("serialize client work action"),
        )
        .expect("parse client work action");
        assert!(matches!(parsed, BusinessCommand::WorkItemAction(_)));
    }

    #[test]
    fn company_work_requires_a_single_owner_and_chat_source_pair() {
        let work_item_id = Uuid::from_u128(43);
        let mut action = company_work_action(
            work_item_id,
            CompanyWorkItemActionKind::Create,
            None,
            Some(company_work_input(work_item_id)),
        );
        assert_eq!(validate_company_work_item_action(&action), Ok(()));

        let head = action.head.as_mut().expect("create head");
        head.assigned_pubkeys.push("c".repeat(64));
        assert!(validate_company_work_item_action(&action).is_err());

        {
            let head = action.head.as_mut().expect("create head");
            head.assigned_pubkeys.truncate(1);
            head.source_event_id = Some("d".repeat(64));
        }
        assert!(validate_company_work_item_action(&action).is_err());
        action
            .head
            .as_mut()
            .expect("create head")
            .thread_root_event_id = Some("e".repeat(64));
        assert_eq!(validate_company_work_item_action(&action), Ok(()));
    }

    #[test]
    fn company_work_due_date_actions_require_utc_rfc3339() {
        let work_item_id = Uuid::from_u128(44);
        let mut create = company_work_action(
            work_item_id,
            CompanyWorkItemActionKind::Create,
            None,
            Some(company_work_input(work_item_id)),
        );
        create.head.as_mut().expect("create head").due_at = Some("2026-10-02T12:00:00Z".into());
        assert_eq!(validate_company_work_item_action(&create), Ok(()));

        create.head.as_mut().expect("create head").due_at =
            Some("2026-10-02T12:00:00+00:00".into());
        assert!(validate_company_work_item_action(&create).is_err());
        create.head.as_mut().expect("create head").due_at = Some("tomorrow".into());
        assert!(validate_company_work_item_action(&create).is_err());

        let mut set = company_work_action(
            work_item_id,
            CompanyWorkItemActionKind::SetDueDate,
            Some("ab".repeat(32)),
            None,
        );
        set.due_at = Some("2026-10-02T12:00:00Z".into());
        assert_eq!(validate_company_work_item_action(&set), Ok(()));
        set.due_at = Some("2026-10-02T12:00:00-01:00".into());
        assert!(validate_company_work_item_action(&set).is_err());

        let clear = company_work_action(
            work_item_id,
            CompanyWorkItemActionKind::ClearDueDate,
            Some("cd".repeat(32)),
            None,
        );
        assert_eq!(validate_company_work_item_action(&clear), Ok(()));
    }

    #[test]
    fn company_work_updates_can_move_a_standalone_item_to_a_thread() {
        let work_item_id = Uuid::from_u128(45);
        let mut action = company_work_action(
            work_item_id,
            CompanyWorkItemActionKind::Update,
            Some("ab".repeat(32)),
            Some(company_work_input(work_item_id)),
        );
        action
            .head
            .as_mut()
            .expect("update head")
            .thread_root_event_id = Some("ef".repeat(32));
        assert_eq!(validate_company_work_item_action(&action), Ok(()));

        let mut source_without_root = action.clone();
        let head = source_without_root.head.as_mut().expect("update head");
        head.source_event_id = Some("ab".repeat(32));
        head.thread_root_event_id = None;
        assert!(validate_company_work_item_action(&source_without_root).is_err());
    }

    #[test]
    fn company_work_uses_a_host_scoped_coordinate_and_shared_kind() {
        let work_item_id = Uuid::from_u128(42);
        let action = company_work_action(
            work_item_id,
            CompanyWorkItemActionKind::Create,
            None,
            Some(company_work_input(work_item_id)),
        );
        let parsed = parse_business_command(
            crate::kind::KIND_WORK_ITEM_ACTION,
            &serde_json::to_string(&action).expect("serialize company work action"),
        )
        .expect("parse company work action");
        assert!(matches!(parsed, BusinessCommand::CompanyWorkItemAction(_)));
        assert_eq!(validate_company_work_item_action(&action), Ok(()));
        assert_eq!(
            company_work_d_tag(work_item_id),
            format!("company:work:{work_item_id}")
        );
        assert_eq!(
            validate_business_command_scope(
                Uuid::from_u128(10),
                Uuid::from_u128(11),
                &company_work_d_tag(work_item_id),
                &parsed,
            ),
            Ok(())
        );
        assert_eq!(
            validate_business_command_scope(
                Uuid::from_u128(12),
                Uuid::from_u128(11),
                &company_work_d_tag(Uuid::from_u128(99)),
                &parsed,
            ),
            Err(BusinessRecordError::DTagMismatch)
        );
    }

    #[test]
    fn verification_requires_reason_evidence_and_an_exact_head() {
        let work_item_id = Uuid::from_u128(44);
        let mut action = company_work_action(
            work_item_id,
            CompanyWorkItemActionKind::Verify,
            Some("f".repeat(64)),
            None,
        );
        action.verification = Some(CompanyWorkVerificationInput {
            verdict: CompanyWorkVerdict::Pass,
            reason: "The done condition is met.".into(),
            evidence: "Approved plan and calendar are attached.".into(),
        });
        assert_eq!(validate_company_work_item_action(&action), Ok(()));

        action.verification.as_mut().expect("verification").evidence = " ".into();
        assert!(validate_company_work_item_action(&action).is_err());
        action.verification.as_mut().expect("verification").evidence = "Reviewed attachment".into();
        action.expected_head_event_id = None;
        assert!(validate_company_work_item_action(&action).is_err());
    }
}
