//! Co-founder customer record: one record per business across its whole life,
//! from the lead found in Discovery to the client, the project and the money
//! (spec section 12).
//!
//! One relay-signed head kind (30679) and one member action kind (47069) carry
//! the record. Records are community-wide (no `h` or `p` tag) and keyed by the
//! **party id** the prospect, the proposals and the client already share, so a
//! lead that converts to a client keeps one record and every entry it had:
//!
//! | d-tag | Record |
//! | --- | --- |
//! | `customer:<partyId>` | The customer record of one business |
//!
//! **The relay builds the record from real records.** [`compute_customer_record`]
//! runs inside the relay over the company's own prospects, WhatsApp rows,
//! proposals, payments, invoices, clients and playbook running copies
//! ([`CustomerSource`]). An agent never writes a number or a timeline entry of
//! those kinds. What an agent or the owner does write (kind 47069) is a
//! *milestone* (a call booked, an agreement signed, anything else), a copied
//! email or a file reference. A calendar, e-sign or payment-link provider can
//! later write the very same milestone, so no provider is wired in here.
//!
//! [`apply_customer_action`] is the state machine and [`authorize_customer_action`]
//! the authority rule, both testable without a database. The relay broker
//! resolves who the caller is and whether co-founder mode is on first.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::business_records::{
    invoice_lines_total_minor, ClientHead, InvoiceHead, InvoiceStatus, PaymentEvidence,
    ProposalAcceptance, ProposalVersion, ProspectHead,
};
use crate::company_functions::CompanyFunction;
use crate::company_plans::{PlanActor, PlanActorRole, PlanContext, PlanError, PlanErrorKind};
use crate::company_playbooks::{JobStatus, PlaybookJobHead};
use crate::company_records::{is_hex_id, CompanyRecordError, COMPANY_RECORD_SCHEMA_VERSION};

/// Relay-authored customer record head (community-wide).
pub const KIND_CUSTOMER_HEAD: u32 = crate::kind::KIND_CUSTOMER_HEAD;
/// Member customer record write, brokered and community-wide.
pub const KIND_CUSTOMER_ACTION: u32 = crate::kind::KIND_CUSTOMER_ACTION;

const CUSTOMER_D_TAG_PREFIX: &str = "customer:";

/// A record younger than this is not recomputed by `refresh`.
pub const CUSTOMER_REFRESH_FRESH_SECS: i64 = 60;
/// Most milestones one record keeps.
pub const MAX_MILESTONES: usize = 200;
/// Most attachments (emails and files) one record keeps.
pub const MAX_ATTACHMENTS: usize = 200;
/// Most timeline entries one record keeps (the newest are kept).
pub const MAX_TIMELINE_ENTRIES: usize = 1_000;
/// Longest email summary, in bytes. An agent copies a short summary, never the whole body.
pub const MAX_EMAIL_SUMMARY_BYTES: usize = 4_096;
/// Longest note, title, subject, sender or label, in characters.
pub const MAX_CUSTOMER_TEXT_CHARS: usize = 300;
/// Longest file reference, in characters.
pub const MAX_FILE_REF_CHARS: usize = 500;
/// Most file references on one email.
pub const MAX_EMAIL_FILE_REFS: usize = 5;
/// Longest reply text shown on the timeline, in characters.
const REPLY_DETAIL_CHARS: usize = 160;
/// Earliest time a milestone may name (1 January 2000, Unix seconds).
const MIN_MILESTONE_AT: i64 = 946_684_800;
/// How far ahead of the relay clock a milestone time may be.
const MILESTONE_SKEW_SECS: i64 = 300;

/// d-tag of one customer record.
pub fn customer_d_tag(party_id: &str) -> Result<String, CompanyRecordError> {
    if !is_canonical_uuid(party_id) {
        return Err(CompanyRecordError::Invalid(
            "partyId must be a lowercase UUID",
        ));
    }
    Ok(format!("{CUSTOMER_D_TAG_PREFIX}{party_id}"))
}

/// The party id a customer d-tag names.
pub fn parse_customer_d_tag(d_tag: &str) -> Result<String, CompanyRecordError> {
    let id = d_tag
        .strip_prefix(CUSTOMER_D_TAG_PREFIX)
        .ok_or(CompanyRecordError::DTagMismatch)?;
    customer_d_tag(id)?;
    Ok(id.to_owned())
}

fn is_canonical_uuid(value: &str) -> bool {
    Uuid::parse_str(value)
        .map(|id| !id.is_nil() && id.hyphenated().to_string() == value)
        .unwrap_or(false)
}

fn clip(value: &str, max_chars: usize) -> String {
    let trimmed = value.trim();
    if trimmed.chars().count() <= max_chars {
        return trimmed.to_owned();
    }
    let mut out = trimmed
        .chars()
        .take(max_chars.saturating_sub(1))
        .collect::<String>();
    out.push('\u{2026}');
    out
}

fn text_within(value: &str, max_chars: usize) -> bool {
    !value.trim().is_empty() && value.chars().count() <= max_chars
}

fn norm_name(value: &str) -> String {
    value
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

// ───────────────────────────── head types ─────────────────────────────

/// Where a business is in its life with the company.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CustomerStage {
    /// Found, not a client yet.
    Lead,
    /// Converted to a client.
    Client,
}

/// What a recorded milestone is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MilestoneKind {
    /// A call was booked (by an agent, the owner or later a calendar provider).
    CallBooked,
    /// An agreement was signed (by an agent, the owner or later an e-sign provider).
    AgreementSigned,
    /// Anything else worth a line on the timeline; it carries its own `title`.
    Other,
}

/// What an attachment is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AttachmentKind {
    /// An email an agent copied from Gmail in the built-in browser.
    Email,
    /// A file stored in Blossom.
    File,
}

/// How the relay learned the business became a client.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConversionSource {
    /// A client record for this business exists (accepted proposal or a client created directly).
    ClientRecord,
    /// The owner, an admin or the Sales lead converted it with `convert_to_client`.
    OwnerAction,
}

/// What kind of line a timeline entry is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TimelineKind {
    /// The prospect was found in Discovery.
    Found,
    /// The WhatsApp opener was approved.
    OpenerApproved,
    /// The WhatsApp opener reached the provider.
    OpenerSent,
    /// The first reply to the opener.
    Replied,
    /// A recorded "call booked" milestone.
    CallBooked,
    /// A proposal version was written.
    QuoteDrafted,
    /// A proposal was accepted.
    QuoteAccepted,
    /// A recorded "agreement signed" milestone.
    AgreementSigned,
    /// Any other recorded milestone.
    Milestone,
    /// An email copied onto the record.
    Email,
    /// A file attached to the record.
    File,
    /// The business became a client.
    Converted,
    /// An invoice was issued.
    InvoiceIssued,
    /// A payment was received.
    PaymentReceived,
    /// A playbook running copy started for this business.
    ProjectStarted,
    /// A playbook running copy finished.
    ProjectCompleted,
}

impl TimelineKind {
    /// The wire name, also used in entry ids.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Found => "found",
            Self::OpenerApproved => "opener_approved",
            Self::OpenerSent => "opener_sent",
            Self::Replied => "replied",
            Self::CallBooked => "call_booked",
            Self::QuoteDrafted => "quote_drafted",
            Self::QuoteAccepted => "quote_accepted",
            Self::AgreementSigned => "agreement_signed",
            Self::Milestone => "milestone",
            Self::Email => "email",
            Self::File => "file",
            Self::Converted => "converted",
            Self::InvoiceIssued => "invoice_issued",
            Self::PaymentReceived => "payment_received",
            Self::ProjectStarted => "project_started",
            Self::ProjectCompleted => "project_completed",
        }
    }

    /// Order of entries that share a second: the life of a customer, in order.
    const fn rank(self) -> u8 {
        match self {
            Self::Found => 0,
            Self::OpenerApproved => 1,
            Self::OpenerSent => 2,
            Self::Replied => 3,
            Self::CallBooked => 4,
            Self::QuoteDrafted => 5,
            Self::QuoteAccepted => 6,
            Self::AgreementSigned => 7,
            Self::Milestone => 8,
            Self::Email => 9,
            Self::File => 10,
            Self::Converted => 11,
            Self::InvoiceIssued => 12,
            Self::PaymentReceived => 13,
            Self::ProjectStarted => 14,
            Self::ProjectCompleted => 15,
        }
    }
}

/// The record a timeline entry came from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct SourceRef {
    /// `prospect`, `whatsapp_batch`, `whatsapp_message`, `proposal_version`,
    /// `proposal_acceptance`, `invoice`, `payment`, `client`, `playbook_job`,
    /// `milestone` or `attachment`.
    pub kind: String,
    /// Id of that record.
    pub id: String,
}

/// One line of the customer's life.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct TimelineEntry {
    /// Stable id: `<source kind>:<source id>:<entry kind>`.
    pub entry_id: String,
    /// What kind of line this is.
    pub kind: TimelineKind,
    /// Short title, for example "Found in Discovery".
    pub title: String,
    /// One supporting line, if there is one.
    pub detail: Option<String>,
    /// Unix seconds the thing happened.
    pub at: i64,
    /// Who did it, when a person or agent did; `None` for the customer or a provider.
    pub actor_pubkey: Option<String>,
    /// The record it came from.
    pub source: SourceRef,
}

/// When and how the prospect was found.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct Found {
    /// Unix seconds the prospect was created.
    pub at: i64,
    /// Who created it.
    pub by_pubkey: Option<String>,
    /// The search it came from, in words (picked tags, else vertical and location).
    pub search: Option<String>,
}

/// How and when the business became a client.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct Conversion {
    /// Unix seconds it became a client.
    pub at: i64,
    /// How the relay learned it.
    pub source: ConversionSource,
    /// Who converted it, for `owner_action`.
    pub by_pubkey: Option<String>,
    /// The action that converted it, for `owner_action`.
    pub action_event_id: Option<String>,
}

/// The Money panel. Every amount is in minor units of `currency` and is `None`
/// when the records do not say, never `0` for "unknown".
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct Money {
    /// ISO 4217 code every figure is stated in; `None` when there is no money record.
    pub currency: Option<String>,
    /// Total of the accepted proposal version, else the latest proposal version.
    pub quote_total_minor: Option<i64>,
    /// Everything paid so far (the first payment is the deposit).
    pub deposit_paid_minor: Option<i64>,
    /// When the first payment arrived.
    pub deposit_paid_at: Option<i64>,
    /// What is still owed: issued invoices' outstanding amounts, else the quote less payments.
    pub balance_due_minor: Option<i64>,
    /// Money records left out because they are in another currency.
    pub other_currency_facts: u32,
}

/// One proposal, at its latest revision.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct QuoteSummary {
    /// Proposal id.
    pub proposal_id: String,
    /// Latest revision.
    pub revision: u32,
    /// Total of that revision, `None` when its lines overflow.
    pub total_minor: Option<i64>,
    /// ISO 4217 code.
    pub currency: String,
    /// When the first version was written.
    pub drafted_at: i64,
    /// When the proposal was accepted, if it was.
    pub accepted_at: Option<i64>,
}

/// The WhatsApp chat of this business.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct WhatsappSummary {
    /// Digits only, E.164 without the plus.
    pub wa_id: String,
    /// Messages in the chat, both directions.
    pub message_count: u32,
    /// Messages they sent.
    pub inbound_count: u32,
    /// Messages we sent or queued.
    pub outbound_count: u32,
    /// Unix seconds of the last message.
    pub last_message_at: Option<i64>,
    /// They opted out.
    pub opted_out: bool,
}

/// The Chats panel.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct Chats {
    /// The WhatsApp chat, when there is one.
    pub whatsapp: Option<WhatsappSummary>,
    /// Emails copied onto the record.
    pub email_count: u32,
}

/// The project in Operations: the playbook running copy for this business.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct ProjectSummary {
    /// Running copy id.
    pub job_id: String,
    /// The job title (the client's name).
    pub title: String,
    /// The playbook it runs.
    pub playbook_title: String,
    /// Lifecycle state.
    pub status: JobStatus,
    /// The current step, absent once the job ended.
    pub current_step_id: Option<String>,
    /// Title of the current step.
    pub current_step_title: Option<String>,
    /// 1-based position of the current step ("step 3 of 7").
    pub step_number: Option<u32>,
    /// Steps in the playbook.
    pub step_count: u32,
    /// Unix seconds it started.
    pub started_at: i64,
    /// Unix seconds it ended.
    pub ended_at: Option<i64>,
}

/// A milestone an agent or the owner recorded.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct Milestone {
    /// Id: the id of the action that recorded it.
    pub milestone_id: String,
    /// What it is.
    pub kind: MilestoneKind,
    /// The title, for `other`.
    pub title: Option<String>,
    /// Unix seconds it happened.
    pub at: i64,
    /// A note, for example the call time or who signed.
    pub note: Option<String>,
    /// A Blossom file reference, for example the signed agreement.
    pub file_ref: Option<String>,
    /// Who recorded it.
    pub by_pubkey: String,
    /// Unix seconds it was recorded.
    pub recorded_at: i64,
}

/// An email an agent copied onto the record.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct EmailInfo {
    /// Sender.
    pub from: String,
    /// Subject.
    pub subject: String,
    /// Unix seconds the email was sent.
    pub date: i64,
    /// A short summary; never the whole body.
    pub summary: String,
}

/// An email or a file attached to the record.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct Attachment {
    /// Id: the id of the action that added it.
    pub attachment_id: String,
    /// Email or file.
    pub kind: AttachmentKind,
    /// The file's label, or the email's subject.
    pub label: String,
    /// Blossom file references (one for a file, up to five for an email).
    pub file_refs: Vec<String>,
    /// The email, for `email`.
    pub email: Option<EmailInfo>,
    /// Who added it.
    pub added_by_pubkey: String,
    /// Unix seconds it was added.
    pub added_at: i64,
}

/// Relay-signed customer record head (kind 30679).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct CustomerRecordHead {
    /// Schema version.
    pub schema_version: u8,
    /// The party id every record of this business shares.
    pub party_id: String,
    /// The business name.
    pub display_name: String,
    /// Lead or client.
    pub stage: CustomerStage,
    /// The prospect id, when the business was found as a lead.
    pub lead_id: Option<String>,
    /// The client id, once there is a client record.
    pub client_id: Option<String>,
    /// The client record's status.
    pub client_status: Option<String>,
    /// How it was found.
    pub found: Option<Found>,
    /// How it became a client.
    pub conversion: Option<Conversion>,
    /// The Money panel.
    pub money: Money,
    /// Proposals, oldest first.
    pub quotes: Vec<QuoteSummary>,
    /// The Chats panel.
    pub chats: Chats,
    /// The project in Operations.
    pub project: Option<ProjectSummary>,
    /// Recorded milestones, oldest first.
    pub milestones: Vec<Milestone>,
    /// Emails and files, oldest first.
    pub attachments: Vec<Attachment>,
    /// The customer's life in order, oldest first.
    pub timeline: Vec<TimelineEntry>,
    /// Unix seconds the relay last recomputed the record.
    pub computed_at: i64,
    /// Who made the last write: the relay's actor or the member of the last action.
    pub updated_by_pubkey: String,
    /// Unix seconds of the last write.
    pub updated_at: i64,
    /// The action that produced this head; `None` when the relay wrote it by itself.
    pub source_action_event_id: Option<String>,
}

impl CustomerRecordHead {
    /// The same record without the fields that only say when it was last written.
    /// Two heads with equal content never need a new write.
    pub fn same_content(&self, other: &Self) -> bool {
        let strip = |head: &Self| {
            let mut copy = head.clone();
            copy.computed_at = 0;
            copy.updated_at = 0;
            copy.updated_by_pubkey = String::new();
            copy.source_action_event_id = None;
            copy
        };
        strip(self) == strip(other)
    }
}

/// What a record carries that no other record supplies: the milestones, the
/// attachments and a conversion already decided. Carried from head to head.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ManualState {
    /// Recorded milestones.
    pub milestones: Vec<Milestone>,
    /// Emails and files.
    pub attachments: Vec<Attachment>,
    /// A conversion already recorded; once set it never changes.
    pub conversion: Option<Conversion>,
}

impl ManualState {
    /// What the current head carries.
    pub fn from_head(head: &CustomerRecordHead) -> Self {
        Self {
            milestones: head.milestones.clone(),
            attachments: head.attachments.clone(),
            conversion: head.conversion.clone(),
        }
    }
}

// ───────────────────────────── sources ─────────────────────────────

/// A prospect and the accepted `create` that made it.
#[derive(Debug, Clone, PartialEq)]
pub struct SourceProspect {
    /// Current prospect head.
    pub head: ProspectHead,
    /// Unix seconds of its first accepted `create` action.
    pub created_at: i64,
    /// Who signed that `create`.
    pub created_by_pubkey: Option<String>,
}

/// One proposal version event.
#[derive(Debug, Clone, PartialEq)]
pub struct SourceProposalVersion {
    /// The version event id.
    pub event_id: String,
    /// Event time.
    pub at: i64,
    /// Who wrote it.
    pub author_pubkey: String,
    /// Its content.
    pub version: ProposalVersion,
}

/// One proposal acceptance event.
#[derive(Debug, Clone, PartialEq)]
pub struct SourceAcceptance {
    /// The acceptance event id.
    pub event_id: String,
    /// Event time.
    pub at: i64,
    /// Who signed it.
    pub actor_pubkey: String,
    /// Its content.
    pub acceptance: ProposalAcceptance,
}

/// One payment evidence event.
#[derive(Debug, Clone, PartialEq)]
pub struct SourcePayment {
    /// The event id.
    pub event_id: String,
    /// Who recorded it.
    pub recorded_by_pubkey: String,
    /// Its content.
    pub payment: PaymentEvidence,
}

/// A client head and the time of the event.
#[derive(Debug, Clone, PartialEq)]
pub struct SourceClient {
    /// Current client head.
    pub head: ClientHead,
    /// Time of the head event.
    pub at: i64,
}

/// A WhatsApp batch an outbound row belongs to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WhatsappBatchFact {
    /// Batch id.
    pub id: String,
    /// The batch was approved (`approved` or `completed`).
    pub approved: bool,
    /// Unix seconds it was decided.
    pub decided_at: Option<i64>,
    /// Who decided it.
    pub decided_by_pubkey: Option<String>,
}

/// One WhatsApp message of this business.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WhatsappRow {
    /// Message id.
    pub id: String,
    /// `true` when we sent or queued it.
    pub outbound: bool,
    /// Delivery state (`held`, `queued`, `sent`, `delivered`, `read`, `failed`, ...).
    pub state: String,
    /// Text, rendered for a template.
    pub body: String,
    /// Template name, for a template.
    pub template_name: Option<String>,
    /// The template's title, when known.
    pub template_title: Option<String>,
    /// First message of a new chat.
    pub new_chat: bool,
    /// Unix seconds: the send time of an outbound row, the arrival time of an inbound one.
    pub at: i64,
    /// Who asked for the send.
    pub sender_pubkey: Option<String>,
    /// The batch the row belongs to.
    pub batch: Option<WhatsappBatchFact>,
}

/// The WhatsApp chat of one business.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceWhatsapp {
    /// Digits only.
    pub wa_id: String,
    /// The contact opted out.
    pub opted_out: bool,
    /// Every message of the chat.
    pub rows: Vec<WhatsappRow>,
}

/// Every record the relay read for the whole company, before it is split by business.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct CustomerFacts {
    /// Prospect heads.
    pub prospects: Vec<SourceProspect>,
    /// Client heads.
    pub clients: Vec<SourceClient>,
    /// Proposal versions.
    pub proposals: Vec<SourceProposalVersion>,
    /// Proposal acceptances.
    pub acceptances: Vec<SourceAcceptance>,
    /// Payment evidence.
    pub payments: Vec<SourcePayment>,
    /// Invoice heads.
    pub invoices: Vec<InvoiceHead>,
    /// Playbook running copies.
    pub jobs: Vec<PlaybookJobHead>,
}

/// Everything the records say about one business.
#[derive(Debug, Clone, PartialEq)]
pub struct CustomerSource {
    /// The party id.
    pub party_id: String,
    /// The prospect, when the business was found as a lead.
    pub prospect: Option<SourceProspect>,
    /// Client records of this party.
    pub clients: Vec<SourceClient>,
    /// Proposal versions for this party.
    pub proposals: Vec<SourceProposalVersion>,
    /// Acceptances of those proposals.
    pub acceptances: Vec<SourceAcceptance>,
    /// Payments of the party's clients.
    pub payments: Vec<SourcePayment>,
    /// Invoices of the party's clients.
    pub invoices: Vec<InvoiceHead>,
    /// Running copies titled with the business's name.
    pub jobs: Vec<PlaybookJobHead>,
    /// The WhatsApp chat, filled in by the relay from its own tables.
    pub whatsapp: Option<SourceWhatsapp>,
}

impl CustomerFacts {
    /// Every party that has a prospect or a client record, in id order.
    pub fn party_ids(&self) -> Vec<String> {
        let mut ids = BTreeSet::new();
        for prospect in &self.prospects {
            ids.insert(
                prospect
                    .head
                    .prospect
                    .party
                    .party_id
                    .hyphenated()
                    .to_string(),
            );
        }
        for client in &self.clients {
            ids.insert(client.head.party_id.hyphenated().to_string());
        }
        ids.into_iter().collect()
    }

    /// Split out what the records say about one business, or `None` when the
    /// party has neither a prospect nor a client record.
    pub fn source_for(&self, party_id: &str) -> Option<CustomerSource> {
        let party: Uuid = Uuid::parse_str(party_id).ok()?;
        let prospect = self
            .prospects
            .iter()
            .filter(|prospect| prospect.head.prospect.party.party_id == party)
            .min_by_key(|prospect| (prospect.created_at, prospect.head.prospect_id))
            .cloned();
        let clients = self
            .clients
            .iter()
            .filter(|client| client.head.party_id == party)
            .cloned()
            .collect::<Vec<_>>();
        if prospect.is_none() && clients.is_empty() {
            return None;
        }
        let proposals = self
            .proposals
            .iter()
            .filter(|proposal| proposal.version.prospect_party_id == party)
            .cloned()
            .collect::<Vec<_>>();
        let proposal_ids = proposals
            .iter()
            .map(|proposal| proposal.version.proposal_id)
            .collect::<BTreeSet<_>>();
        let acceptances = self
            .acceptances
            .iter()
            .filter(|acceptance| proposal_ids.contains(&acceptance.acceptance.proposal_id))
            .cloned()
            .collect::<Vec<_>>();
        let mut client_ids = clients
            .iter()
            .map(|client| client.head.client_id)
            .collect::<BTreeSet<_>>();
        client_ids.extend(
            acceptances
                .iter()
                .map(|acceptance| acceptance.acceptance.client_id),
        );
        let payments = self
            .payments
            .iter()
            .filter(|payment| client_ids.contains(&payment.payment.client_id))
            .cloned()
            .collect::<Vec<_>>();
        let invoices = self
            .invoices
            .iter()
            .filter(|invoice| {
                client_ids.contains(&invoice.client_id)
                    || proposal_ids.contains(&invoice.proposal_id)
            })
            .cloned()
            .collect::<Vec<_>>();
        let mut names = BTreeSet::new();
        if let Some(prospect) = &prospect {
            names.insert(norm_name(&prospect.head.prospect.party.display_name));
        }
        for client in &clients {
            names.insert(norm_name(&client.head.display_name));
        }
        let jobs = self
            .jobs
            .iter()
            .filter(|job| names.contains(&norm_name(&job.title)))
            .cloned()
            .collect::<Vec<_>>();
        Some(CustomerSource {
            party_id: party_id.to_owned(),
            prospect,
            clients,
            proposals,
            acceptances,
            payments,
            invoices,
            jobs,
            whatsapp: None,
        })
    }
}

// ───────────────────────────── compute ─────────────────────────────

fn entry(
    kind: TimelineKind,
    title: impl Into<String>,
    detail: Option<String>,
    at: i64,
    actor_pubkey: Option<String>,
    source_kind: &str,
    source_id: &str,
) -> TimelineEntry {
    TimelineEntry {
        entry_id: format!("{source_kind}:{source_id}:{}", kind.as_str()),
        kind,
        title: title.into(),
        detail,
        at,
        actor_pubkey,
        source: SourceRef {
            kind: source_kind.to_owned(),
            id: source_id.to_owned(),
        },
    }
}

fn search_label(source: &SourceProspect) -> Option<String> {
    let prospect = &source.head.prospect;
    let picked = prospect
        .discovery_tags
        .iter()
        .filter(|tag| tag.picked)
        .map(|tag| tag.tag.trim())
        .filter(|tag| !tag.is_empty())
        .collect::<Vec<_>>();
    let mut label = if picked.is_empty() {
        let vertical = prospect.vertical.trim();
        let location = prospect.location.as_deref().map(str::trim).unwrap_or("");
        match (vertical.is_empty(), location.is_empty()) {
            (false, false) => format!("{vertical} in {location}"),
            (false, true) => vertical.to_owned(),
            (true, false) => location.to_owned(),
            (true, true) => String::new(),
        }
    } else {
        picked.join(", ")
    };
    label = clip(&label, MAX_CUSTOMER_TEXT_CHARS);
    (!label.is_empty()).then_some(label)
}

fn is_sent_state(state: &str) -> bool {
    matches!(state, "sent" | "delivered" | "read")
}

fn whatsapp_entries(
    whatsapp: &SourceWhatsapp,
    entries: &mut Vec<TimelineEntry>,
) -> WhatsappSummary {
    let mut rows = whatsapp.rows.iter().collect::<Vec<_>>();
    rows.sort_by(|a, b| (a.at, &a.id).cmp(&(b.at, &b.id)));
    let outbound = rows.iter().filter(|row| row.outbound).count();
    let summary = WhatsappSummary {
        wa_id: whatsapp.wa_id.clone(),
        message_count: rows.len() as u32,
        inbound_count: (rows.len() - outbound) as u32,
        outbound_count: outbound as u32,
        last_message_at: rows.iter().map(|row| row.at).max(),
        opted_out: whatsapp.opted_out,
    };
    let opener = rows
        .iter()
        .find(|row| row.outbound && row.new_chat)
        .or_else(|| rows.iter().find(|row| row.outbound));
    let Some(opener) = opener else {
        return summary;
    };
    let name = opener
        .template_title
        .as_deref()
        .or(opener.template_name.as_deref())
        .map(str::trim)
        .filter(|name| !name.is_empty());
    let detail = name.map(|name| clip(&format!("Template: {name}"), MAX_CUSTOMER_TEXT_CHARS));
    if let Some(batch) = opener.batch.as_ref().filter(|batch| batch.approved) {
        if let Some(decided_at) = batch.decided_at {
            entries.push(entry(
                TimelineKind::OpenerApproved,
                "WhatsApp opener approved",
                detail.clone(),
                decided_at,
                batch.decided_by_pubkey.clone(),
                "whatsapp_batch",
                &batch.id,
            ));
        }
    }
    if !is_sent_state(&opener.state) {
        return summary;
    }
    entries.push(entry(
        TimelineKind::OpenerSent,
        "WhatsApp opener sent",
        detail,
        opener.at,
        opener.sender_pubkey.clone(),
        "whatsapp_message",
        &opener.id,
    ));
    if let Some(reply) = rows.iter().find(|row| !row.outbound && row.at >= opener.at) {
        let text = clip(&reply.body, REPLY_DETAIL_CHARS);
        entries.push(entry(
            TimelineKind::Replied,
            "Replied",
            (!text.is_empty()).then_some(text),
            reply.at,
            None,
            "whatsapp_message",
            &reply.id,
        ));
    }
    summary
}

struct QuoteView {
    summary: QuoteSummary,
    /// Total of the accepted version, when the proposal was accepted.
    accepted_total_minor: Option<Option<i64>>,
    accepted_currency: Option<String>,
    latest_at: i64,
}

fn quote_views(source: &CustomerSource, entries: &mut Vec<TimelineEntry>) -> Vec<QuoteView> {
    let mut by_proposal: BTreeMap<Uuid, Vec<&SourceProposalVersion>> = BTreeMap::new();
    for version in &source.proposals {
        by_proposal
            .entry(version.version.proposal_id)
            .or_default()
            .push(version);
    }
    let mut views = Vec::new();
    for (proposal_id, mut versions) in by_proposal {
        versions.sort_by_key(|v| (v.version.revision, v.at));
        let first = versions[0];
        let latest = versions[versions.len() - 1];
        for version in &versions {
            let revised = version.version.revision > 1;
            entries.push(entry(
                TimelineKind::QuoteDrafted,
                if revised {
                    "Quote revised"
                } else {
                    "Quote drafted"
                },
                Some(format!("Version {}", version.version.revision)),
                version.at,
                Some(version.author_pubkey.clone()),
                "proposal_version",
                &version.event_id,
            ));
        }
        let acceptance = source
            .acceptances
            .iter()
            .filter(|a| a.acceptance.proposal_id == proposal_id)
            .min_by_key(|a| (a.at, a.event_id.clone()));
        let mut accepted_total_minor = None;
        let mut accepted_currency = None;
        if let Some(acceptance) = acceptance {
            let accepted_version = versions
                .iter()
                .find(|v| v.event_id == acceptance.acceptance.proposal_version_event_id);
            let detail = match (&acceptance.acceptance.evidence, accepted_version) {
                (Some(evidence), _) if !evidence.accepted_by_name.trim().is_empty() => Some(clip(
                    &format!("Accepted by {}", evidence.accepted_by_name),
                    MAX_CUSTOMER_TEXT_CHARS,
                )),
                (_, Some(version)) => Some(format!("Version {}", version.version.revision)),
                _ => None,
            };
            entries.push(entry(
                TimelineKind::QuoteAccepted,
                "Quote accepted",
                detail,
                acceptance.at,
                Some(acceptance.actor_pubkey.clone()),
                "proposal_acceptance",
                &acceptance.event_id,
            ));
            if let Some(version) = accepted_version {
                accepted_total_minor = Some(invoice_lines_total_minor(&version.version.lines));
                accepted_currency = Some(version.version.currency.clone());
            }
        }
        views.push(QuoteView {
            summary: QuoteSummary {
                proposal_id: proposal_id.hyphenated().to_string(),
                revision: latest.version.revision,
                total_minor: invoice_lines_total_minor(&latest.version.lines),
                currency: latest.version.currency.clone(),
                drafted_at: first.at,
                accepted_at: acceptance.map(|a| a.at),
            },
            accepted_total_minor,
            accepted_currency,
            latest_at: latest.at,
        });
    }
    views.sort_by_key(|view| (view.summary.drafted_at, view.summary.proposal_id.clone()));
    views
}

/// The Money panel and the invoice and payment entries.
fn money_panel(
    source: &CustomerSource,
    quotes: &[QuoteView],
    entries: &mut Vec<TimelineEntry>,
) -> Money {
    // The quote the money is stated against: the accepted one, else the latest.
    let accepted = quotes
        .iter()
        .filter(|view| view.accepted_total_minor.is_some())
        .max_by_key(|view| (view.summary.accepted_at, view.latest_at));
    let chosen = accepted.or_else(|| quotes.iter().max_by_key(|view| view.latest_at));

    let mut seen_payments: HashMap<Uuid, &SourcePayment> = HashMap::new();
    for payment in &source.payments {
        seen_payments
            .entry(payment.payment.payment_id)
            .or_insert(payment);
    }
    let mut payments = seen_payments.into_values().collect::<Vec<_>>();
    payments.sort_by_key(|p| (p.payment.occurred_at, p.payment.payment_id));
    let issued = source
        .invoices
        .iter()
        .filter(|invoice| invoice.status == InvoiceStatus::Issued)
        .collect::<Vec<_>>();

    let currency = chosen
        .map(|view| {
            view.accepted_currency
                .clone()
                .unwrap_or_else(|| view.summary.currency.clone())
        })
        .or_else(|| issued.first().map(|invoice| invoice.currency.clone()))
        .or_else(|| payments.first().map(|p| p.payment.currency.clone()));
    let Some(currency) = currency else {
        return Money {
            currency: None,
            quote_total_minor: None,
            deposit_paid_minor: None,
            deposit_paid_at: None,
            balance_due_minor: None,
            other_currency_facts: 0,
        };
    };

    let quote_total_minor = chosen.and_then(|view| match &view.accepted_total_minor {
        Some(total) => *total,
        None => view.summary.total_minor,
    });

    let mut other_currency_facts = 0_u32;
    let mut paid_total = 0_i64;
    let mut first_paid_at = None;
    let mut paid_any = false;
    for (index, payment) in payments.iter().enumerate() {
        let detail = if payment.payment.provider == "manual" {
            "Recorded by hand".to_owned()
        } else {
            clip(&payment.payment.provider, MAX_CUSTOMER_TEXT_CHARS)
        };
        entries.push(entry(
            TimelineKind::PaymentReceived,
            if index == 0 {
                "Deposit paid"
            } else {
                "Payment received"
            },
            Some(detail),
            payment.payment.occurred_at,
            Some(payment.recorded_by_pubkey.clone()),
            "payment",
            &payment.payment.payment_id.hyphenated().to_string(),
        ));
        if payment.payment.currency != currency {
            other_currency_facts += 1;
            continue;
        }
        paid_any = true;
        paid_total = paid_total.saturating_add(payment.payment.amount_minor);
        first_paid_at.get_or_insert(payment.payment.occurred_at);
    }

    let mut owed = 0_i64;
    let mut issued_in_currency = 0_u32;
    for invoice in &issued {
        if let Some(at) = invoice.issued_at {
            entries.push(entry(
                TimelineKind::InvoiceIssued,
                "Invoice issued",
                Some(format!("Version {}", invoice.version)),
                at,
                None,
                "invoice",
                &invoice.invoice_id.hyphenated().to_string(),
            ));
        }
        if invoice.currency != currency {
            other_currency_facts += 1;
            continue;
        }
        issued_in_currency += 1;
        owed = owed.saturating_add(invoice.outstanding_minor.max(0));
    }

    let deposit_paid_minor = if paid_any {
        Some(paid_total)
    } else if issued_in_currency > 0 {
        // An issued invoice with no payment evidence: nothing has been paid.
        Some(0)
    } else {
        None
    };
    let balance_due_minor = if issued_in_currency > 0 {
        Some(owed)
    } else {
        match (quote_total_minor, paid_any) {
            (Some(total), true) => Some(total.saturating_sub(paid_total).max(0)),
            _ => None,
        }
    };
    Money {
        currency: Some(currency),
        quote_total_minor,
        deposit_paid_minor,
        deposit_paid_at: first_paid_at,
        balance_due_minor,
        other_currency_facts,
    }
}

fn project_panel(
    source: &CustomerSource,
    entries: &mut Vec<TimelineEntry>,
) -> Option<ProjectSummary> {
    for job in &source.jobs {
        let step_count = job.steps.len() as u32;
        let current = job
            .current_step_id
            .as_deref()
            .and_then(|id| job.steps.iter().position(|step| step.step_id == id));
        let detail = match (current, job.status) {
            (Some(index), JobStatus::Running) => format!(
                "{} \u{00b7} step {} of {}: {}",
                job.playbook_title,
                index + 1,
                step_count,
                job.steps[index].title
            ),
            _ => job.playbook_title.clone(),
        };
        entries.push(entry(
            TimelineKind::ProjectStarted,
            "Project in Operations",
            Some(clip(&detail, MAX_CUSTOMER_TEXT_CHARS)),
            job.started_at,
            Some(job.started_by_pubkey.clone()),
            "playbook_job",
            &job.job_id,
        ));
        if let (JobStatus::Completed, Some(ended_at)) = (job.status, job.ended_at) {
            entries.push(entry(
                TimelineKind::ProjectCompleted,
                "Project finished",
                Some(clip(&job.playbook_title, MAX_CUSTOMER_TEXT_CHARS)),
                ended_at,
                Some(job.updated_by_pubkey.clone()),
                "playbook_job",
                &job.job_id,
            ));
        }
    }
    // The running copy wins; otherwise the most recently started one.
    let job = source
        .jobs
        .iter()
        .max_by_key(|job| (job.status.is_open(), job.started_at, job.job_id.clone()))?;
    let current = job
        .current_step_id
        .as_deref()
        .and_then(|id| job.steps.iter().position(|step| step.step_id == id));
    Some(ProjectSummary {
        job_id: job.job_id.clone(),
        title: job.title.clone(),
        playbook_title: job.playbook_title.clone(),
        status: job.status,
        current_step_id: job.current_step_id.clone(),
        current_step_title: current.map(|index| job.steps[index].title.clone()),
        step_number: current.map(|index| index as u32 + 1),
        step_count: job.steps.len() as u32,
        started_at: job.started_at,
        ended_at: job.ended_at,
    })
}

fn detect_conversion(source: &CustomerSource) -> Option<Conversion> {
    let client = source.clients.iter().min_by_key(|c| c.at)?;
    let at = source
        .acceptances
        .iter()
        .map(|a| a.at)
        .min()
        .map_or(client.at, |accepted| accepted.min(client.at));
    Some(Conversion {
        at,
        source: ConversionSource::ClientRecord,
        by_pubkey: None,
        action_event_id: None,
    })
}

fn prospect_was_found_in_discovery(prospect: &SourceProspect) -> bool {
    match prospect.head.prospect.origin {
        Some(crate::business_records::ProspectOrigin::Manual) => false,
        Some(crate::business_records::ProspectOrigin::Discovery) => true,
        None => {
            let input = &prospect.head.prospect;
            input.fit_score.is_some()
                || !input.discovery_tags.is_empty()
                || !input.evidence.is_empty()
        }
    }
}

/// Build the customer record head from the company's own records and what the
/// record already carries. The relay's `updated_by` and `action_event_id` stamp
/// the write; every other field comes from the records.
pub fn compute_customer_record(
    source: &CustomerSource,
    manual: &ManualState,
    now: i64,
    updated_by_pubkey: &str,
    action_event_id: Option<&str>,
) -> CustomerRecordHead {
    let mut entries: Vec<TimelineEntry> = Vec::new();

    let display_name = source
        .clients
        .iter()
        .min_by_key(|client| client.at)
        .map(|client| client.head.display_name.clone())
        .or_else(|| {
            source
                .prospect
                .as_ref()
                .map(|prospect| prospect.head.prospect.party.display_name.clone())
        })
        .unwrap_or_default();

    let found = source
        .prospect
        .as_ref()
        .filter(|prospect| prospect_was_found_in_discovery(prospect))
        .map(|prospect| {
            let search = search_label(prospect);
            entries.push(entry(
                TimelineKind::Found,
                "Found in Discovery",
                search.clone(),
                prospect.created_at,
                prospect.created_by_pubkey.clone(),
                "prospect",
                &prospect.head.prospect_id.hyphenated().to_string(),
            ));
            Found {
                at: prospect.created_at,
                by_pubkey: prospect.created_by_pubkey.clone(),
                search,
            }
        });

    let whatsapp = source
        .whatsapp
        .as_ref()
        .map(|whatsapp| whatsapp_entries(whatsapp, &mut entries));

    let quotes = quote_views(source, &mut entries);
    let money = money_panel(source, &quotes, &mut entries);
    let project = project_panel(source, &mut entries);

    let conversion = manual
        .conversion
        .clone()
        .or_else(|| detect_conversion(source));
    if let Some(conversion) = &conversion {
        let (detail, actor) = match conversion.source {
            ConversionSource::ClientRecord => (None, None),
            ConversionSource::OwnerAction => (
                Some("Recorded by hand".to_owned()),
                conversion.by_pubkey.clone(),
            ),
        };
        entries.push(entry(
            TimelineKind::Converted,
            "Became a client",
            detail,
            conversion.at,
            actor,
            "client",
            &source.party_id,
        ));
    }

    for milestone in &manual.milestones {
        let (kind, title) = match milestone.kind {
            MilestoneKind::CallBooked => (TimelineKind::CallBooked, "Call booked".to_owned()),
            MilestoneKind::AgreementSigned => {
                (TimelineKind::AgreementSigned, "Agreement signed".to_owned())
            }
            MilestoneKind::Other => (
                TimelineKind::Milestone,
                milestone
                    .title
                    .clone()
                    .unwrap_or_else(|| "Milestone".to_owned()),
            ),
        };
        entries.push(entry(
            kind,
            title,
            milestone.note.clone(),
            milestone.at,
            Some(milestone.by_pubkey.clone()),
            "milestone",
            &milestone.milestone_id,
        ));
    }
    let mut email_count = 0_u32;
    for attachment in &manual.attachments {
        match (&attachment.kind, &attachment.email) {
            (AttachmentKind::Email, Some(email)) => {
                email_count += 1;
                entries.push(entry(
                    TimelineKind::Email,
                    format!("Email from {}", clip(&email.from, 120)),
                    Some(clip(&email.subject, MAX_CUSTOMER_TEXT_CHARS)),
                    email.date,
                    Some(attachment.added_by_pubkey.clone()),
                    "attachment",
                    &attachment.attachment_id,
                ));
            }
            _ => entries.push(entry(
                TimelineKind::File,
                clip(&attachment.label, MAX_CUSTOMER_TEXT_CHARS),
                None,
                attachment.added_at,
                Some(attachment.added_by_pubkey.clone()),
                "attachment",
                &attachment.attachment_id,
            )),
        }
    }

    entries.sort_by(|a, b| {
        (a.at, a.kind.rank(), &a.entry_id).cmp(&(b.at, b.kind.rank(), &b.entry_id))
    });
    if entries.len() > MAX_TIMELINE_ENTRIES {
        let drop = entries.len() - MAX_TIMELINE_ENTRIES;
        entries.drain(..drop);
    }

    let client = source
        .clients
        .iter()
        .find(|client| client.head.status == "active")
        .or_else(|| source.clients.iter().min_by_key(|client| client.at));
    let mut milestones = manual.milestones.clone();
    milestones.sort_by(|a, b| (a.at, &a.milestone_id).cmp(&(b.at, &b.milestone_id)));
    let mut attachments = manual.attachments.clone();
    attachments.sort_by(|a, b| (a.added_at, &a.attachment_id).cmp(&(b.added_at, &b.attachment_id)));

    CustomerRecordHead {
        schema_version: COMPANY_RECORD_SCHEMA_VERSION,
        party_id: source.party_id.clone(),
        display_name,
        stage: if conversion.is_some() {
            CustomerStage::Client
        } else {
            CustomerStage::Lead
        },
        lead_id: source
            .prospect
            .as_ref()
            .map(|prospect| prospect.head.prospect_id.hyphenated().to_string()),
        client_id: client.map(|client| client.head.client_id.hyphenated().to_string()),
        client_status: client.map(|client| client.head.status.clone()),
        found,
        conversion,
        money,
        quotes: quotes.into_iter().map(|view| view.summary).collect(),
        chats: Chats {
            whatsapp,
            email_count,
        },
        project,
        milestones,
        attachments,
        timeline: entries,
        computed_at: now,
        updated_by_pubkey: updated_by_pubkey.to_owned(),
        updated_at: now,
        source_action_event_id: action_event_id.map(str::to_owned),
    }
}

/// Structural checks on a head the relay is about to write or has read.
pub fn validate_customer_head(head: &CustomerRecordHead) -> Result<(), CompanyRecordError> {
    if head.schema_version != COMPANY_RECORD_SCHEMA_VERSION {
        return Err(CompanyRecordError::UnsupportedSchemaVersion);
    }
    customer_d_tag(&head.party_id)?;
    if head.milestones.len() > MAX_MILESTONES {
        return Err(CompanyRecordError::Invalid("too many milestones"));
    }
    if head.attachments.len() > MAX_ATTACHMENTS {
        return Err(CompanyRecordError::Invalid("too many attachments"));
    }
    if head.timeline.len() > MAX_TIMELINE_ENTRIES {
        return Err(CompanyRecordError::Invalid("too many timeline entries"));
    }
    let mut seen = BTreeSet::new();
    for entry in &head.timeline {
        if !seen.insert(entry.entry_id.as_str()) {
            return Err(CompanyRecordError::Invalid(
                "timeline entry ids must be unique",
            ));
        }
    }
    for milestone in &head.milestones {
        if !is_hex_id(&milestone.milestone_id) || !is_hex_id(&milestone.by_pubkey) {
            return Err(CompanyRecordError::Invalid("milestone ids must be hex"));
        }
    }
    for attachment in &head.attachments {
        if !is_hex_id(&attachment.attachment_id) || !is_hex_id(&attachment.added_by_pubkey) {
            return Err(CompanyRecordError::Invalid("attachment ids must be hex"));
        }
    }
    Ok(())
}

/// `true` while a record is younger than a minute and `refresh` leaves it alone.
pub fn customer_refresh_is_fresh(head: &CustomerRecordHead, now: i64) -> bool {
    now - head.computed_at < CUSTOMER_REFRESH_FRESH_SECS
}

// ───────────────────────────── actions ─────────────────────────────

/// What a customer record write does.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CustomerOp {
    /// Ask the relay to recompute the record (at most once a minute).
    Refresh,
    /// Record that the lead is a client (owner, admin or the Sales lead).
    ConvertToClient,
    /// Record a milestone: a call booked, an agreement signed or anything else.
    RecordMilestone,
    /// Copy an email from Gmail onto the record.
    AttachEmail,
    /// Attach a file by its Blossom reference.
    AttachFile,
    /// Remove an email or file from the record.
    RemoveAttachment,
}

/// Member-signed customer record write (kind 47069). Each op takes only the
/// fields it names; anything else is refused.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct CustomerAction {
    /// Schema version.
    pub schema_version: u8,
    /// The business the record is about.
    pub party_id: String,
    /// What to do.
    pub op: CustomerOp,
    /// What the milestone is, for `record_milestone`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub milestone_kind: Option<MilestoneKind>,
    /// Unix seconds the milestone happened, for `record_milestone`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub at: Option<i64>,
    /// The title, for a milestone of kind `other`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// A note, for `record_milestone` and `convert_to_client`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    /// A Blossom file reference, for `record_milestone` and `attach_file`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file_ref: Option<String>,
    /// The email's sender, for `attach_email`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from: Option<String>,
    /// The email's subject, for `attach_email`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subject: Option<String>,
    /// Unix seconds the email was sent, for `attach_email`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub date: Option<i64>,
    /// A short summary of the email, for `attach_email`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    /// Blossom references of the email's attachments, for `attach_email`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file_refs: Option<Vec<String>>,
    /// The file's label, for `attach_file`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    /// The attachment to remove, for `remove_attachment`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attachment_id: Option<String>,
    /// Exact current head event id; required once a head exists.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_head_event_id: Option<String>,
}

impl CustomerAction {
    /// The d-tag of the record this action changes.
    pub fn d_tag(&self) -> Result<String, CompanyRecordError> {
        customer_d_tag(&self.party_id)
    }
}

fn present_fields(action: &CustomerAction) -> Vec<&'static str> {
    let mut fields = Vec::new();
    let mut note = |present: bool, name: &'static str| {
        if present {
            fields.push(name);
        }
    };
    note(action.milestone_kind.is_some(), "milestoneKind");
    note(action.at.is_some(), "at");
    note(action.title.is_some(), "title");
    note(action.note.is_some(), "note");
    note(action.file_ref.is_some(), "fileRef");
    note(action.from.is_some(), "from");
    note(action.subject.is_some(), "subject");
    note(action.date.is_some(), "date");
    note(action.summary.is_some(), "summary");
    note(action.file_refs.is_some(), "fileRefs");
    note(action.label.is_some(), "label");
    note(action.attachment_id.is_some(), "attachmentId");
    note(
        action.expected_head_event_id.is_some(),
        "expectedHeadEventId",
    );
    fields
}

/// `(required, optional)` fields of each op.
fn op_fields(op: CustomerOp) -> (&'static [&'static str], &'static [&'static str]) {
    match op {
        CustomerOp::Refresh => (&[], &["expectedHeadEventId"]),
        CustomerOp::ConvertToClient => (&[], &["note", "expectedHeadEventId"]),
        CustomerOp::RecordMilestone => (
            &["milestoneKind", "at"],
            &["title", "note", "fileRef", "expectedHeadEventId"],
        ),
        CustomerOp::AttachEmail => (
            &["from", "subject", "date", "summary"],
            &["fileRefs", "expectedHeadEventId"],
        ),
        CustomerOp::AttachFile => (&["label", "fileRef"], &["expectedHeadEventId"]),
        CustomerOp::RemoveAttachment => (&["attachmentId"], &["expectedHeadEventId"]),
    }
}

fn is_file_ref(value: &str) -> bool {
    !value.is_empty()
        && value.chars().count() <= MAX_FILE_REF_CHARS
        && !value.chars().any(char::is_whitespace)
}

/// Shape rules of one action, before any record is read.
pub fn validate_customer_action(action: &CustomerAction) -> Result<(), CompanyRecordError> {
    use CompanyRecordError::Invalid;
    if action.schema_version != COMPANY_RECORD_SCHEMA_VERSION {
        return Err(CompanyRecordError::UnsupportedSchemaVersion);
    }
    customer_d_tag(&action.party_id)?;
    let (required, optional) = op_fields(action.op);
    let present = present_fields(action);
    if required.iter().any(|name| !present.contains(name)) {
        return Err(Invalid("this op is missing a field it needs"));
    }
    if present
        .iter()
        .any(|name| !required.contains(name) && !optional.contains(name))
    {
        return Err(Invalid("this op does not take one of the fields given"));
    }
    if let Some(expected) = &action.expected_head_event_id {
        if !is_hex_id(expected) {
            return Err(Invalid("expectedHeadEventId must be a 64 character hex id"));
        }
    }
    for note in [&action.note, &action.title].into_iter().flatten() {
        if !text_within(note, MAX_CUSTOMER_TEXT_CHARS) {
            return Err(Invalid("title and note must be 1 to 300 characters"));
        }
    }
    match action.op {
        CustomerOp::Refresh | CustomerOp::ConvertToClient => {}
        CustomerOp::RecordMilestone => {
            let kind = action.milestone_kind.unwrap_or(MilestoneKind::Other);
            match (kind, action.title.is_some()) {
                (MilestoneKind::Other, false) => {
                    return Err(Invalid("a milestone of kind other needs a title"))
                }
                (MilestoneKind::CallBooked | MilestoneKind::AgreementSigned, true) => {
                    return Err(Invalid("only a milestone of kind other takes a title"))
                }
                _ => {}
            }
            if action.at.is_some_and(|at| at < MIN_MILESTONE_AT) {
                return Err(Invalid("at must be a Unix time in seconds"));
            }
            if action.file_ref.as_deref().is_some_and(|r| !is_file_ref(r)) {
                return Err(Invalid("fileRef must be a reference without spaces"));
            }
        }
        CustomerOp::AttachEmail => {
            for text in [&action.from, &action.subject].into_iter().flatten() {
                if !text_within(text, MAX_CUSTOMER_TEXT_CHARS) {
                    return Err(Invalid("from and subject must be 1 to 300 characters"));
                }
            }
            if action.date.is_some_and(|date| date < MIN_MILESTONE_AT) {
                return Err(Invalid("date must be a Unix time in seconds"));
            }
            let summary = action.summary.as_deref().unwrap_or_default();
            if summary.trim().is_empty() {
                return Err(Invalid("summary must not be empty"));
            }
            if summary.len() > MAX_EMAIL_SUMMARY_BYTES {
                return Err(Invalid(
                    "summary must be a short summary of at most 4 KB, not the whole email",
                ));
            }
            if let Some(refs) = &action.file_refs {
                if refs.len() > MAX_EMAIL_FILE_REFS || refs.iter().any(|r| !is_file_ref(r)) {
                    return Err(Invalid(
                        "fileRefs holds at most 5 references without spaces",
                    ));
                }
            }
        }
        CustomerOp::AttachFile => {
            if !action
                .label
                .as_deref()
                .is_some_and(|label| text_within(label, MAX_CUSTOMER_TEXT_CHARS))
            {
                return Err(Invalid("label must be 1 to 300 characters"));
            }
            if !action.file_ref.as_deref().is_some_and(is_file_ref) {
                return Err(Invalid("fileRef must be a reference without spaces"));
            }
        }
        CustomerOp::RemoveAttachment => {
            if !action.attachment_id.as_deref().is_some_and(is_hex_id) {
                return Err(Invalid("attachmentId must be a 64 character hex id"));
            }
        }
    }
    Ok(())
}

fn error(kind: PlanErrorKind, message: impl Into<String>) -> PlanError {
    PlanError {
        kind,
        message: message.into(),
    }
}

/// Who may write. Owners and admins always; a managed agent (Scout) for every op
/// except converting; a member only as the lead of the Sales or Customer
/// Success slot. Converting is for the owner, an admin or the Sales lead. A
/// person who is none of these, and any stranger, is refused. Co-founder mode
/// must be on.
pub fn authorize_customer_action(
    action: &CustomerAction,
    actor: &PlanActor,
    context: &PlanContext,
) -> Result<(), PlanError> {
    if !context.cofounder_enabled {
        return Err(error(
            PlanErrorKind::Forbidden,
            "co-founder mode is off for this company, so the customer record is not changed",
        ));
    }
    if actor.role == PlanActorRole::Admin {
        return Ok(());
    }
    let leads = |function: CompanyFunction| actor.led_functions.contains(&function);
    if action.op == CustomerOp::ConvertToClient {
        return if leads(CompanyFunction::Sales) {
            Ok(())
        } else {
            Err(error(
                PlanErrorKind::Forbidden,
                "only the owner, an admin or the Sales lead can convert a lead to a client",
            ))
        };
    }
    if actor.role == PlanActorRole::ManagedAgent
        || leads(CompanyFunction::Sales)
        || leads(CompanyFunction::CustomerSuccess)
    {
        return Ok(());
    }
    Err(error(
        PlanErrorKind::Forbidden,
        "only the owner, an admin, a Sales or Customer Success lead or Scout can change a customer record",
    ))
}

/// What an action did to the record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CustomerChange {
    /// Recompute the record from the company's records, with nothing new carried.
    Refresh,
    /// The action changed nothing, with the reason the caller is told.
    Unchanged(&'static str),
    /// The record carries this now; recompute it.
    Changed(ManualState),
}

/// Apply one action to what the record carries. `auto_client` says a client
/// record already exists for the party, so converting has nothing to do.
pub fn apply_customer_action(
    manual: &ManualState,
    auto_client: bool,
    action: &CustomerAction,
    actor: &PlanActor,
    context: &PlanContext,
) -> Result<CustomerChange, PlanError> {
    validate_customer_action(action).map_err(|e| error(PlanErrorKind::Invalid, e.to_string()))?;
    authorize_customer_action(action, actor, context)?;
    let id = context.action_event_id.clone();
    let mut next = manual.clone();
    match action.op {
        CustomerOp::Refresh => return Ok(CustomerChange::Refresh),
        CustomerOp::ConvertToClient => {
            if manual.conversion.is_some() || auto_client {
                return Ok(CustomerChange::Unchanged("already a client"));
            }
            next.conversion = Some(Conversion {
                at: context.now,
                source: ConversionSource::OwnerAction,
                by_pubkey: Some(actor.pubkey.clone()),
                action_event_id: Some(id),
            });
        }
        CustomerOp::RecordMilestone => {
            let at = action.at.unwrap_or_default();
            if at > context.now + MILESTONE_SKEW_SECS {
                return Err(error(
                    PlanErrorKind::Invalid,
                    "a milestone cannot be in the future; record it when it happened",
                ));
            }
            if next.milestones.len() >= MAX_MILESTONES {
                return Err(error(
                    PlanErrorKind::Conflict,
                    "this record already holds the most milestones it can keep",
                ));
            }
            let kind = action.milestone_kind.unwrap_or(MilestoneKind::Other);
            let same = next.milestones.iter().any(|m| {
                m.kind == kind
                    && m.at == at
                    && m.title == action.title.as_ref().map(|t| t.trim().to_owned())
            });
            if same {
                return Ok(CustomerChange::Unchanged(
                    "that milestone is already recorded",
                ));
            }
            next.milestones.push(Milestone {
                milestone_id: id,
                kind,
                title: action.title.as_ref().map(|t| t.trim().to_owned()),
                at,
                note: action.note.as_ref().map(|n| n.trim().to_owned()),
                file_ref: action.file_ref.clone(),
                by_pubkey: actor.pubkey.clone(),
                recorded_at: context.now,
            });
        }
        CustomerOp::AttachEmail => {
            if next.attachments.len() >= MAX_ATTACHMENTS {
                return Err(error(
                    PlanErrorKind::Conflict,
                    "this record already holds the most attachments it can keep",
                ));
            }
            let email = EmailInfo {
                from: action.from.clone().unwrap_or_default().trim().to_owned(),
                subject: action.subject.clone().unwrap_or_default().trim().to_owned(),
                date: action.date.unwrap_or_default(),
                summary: action.summary.clone().unwrap_or_default().trim().to_owned(),
            };
            let same = next.attachments.iter().any(|a| {
                a.email.as_ref().is_some_and(|known| {
                    known.from == email.from
                        && known.subject == email.subject
                        && known.date == email.date
                })
            });
            if same {
                return Ok(CustomerChange::Unchanged(
                    "that email is already on the record",
                ));
            }
            next.attachments.push(Attachment {
                attachment_id: id,
                kind: AttachmentKind::Email,
                label: email.subject.clone(),
                file_refs: action.file_refs.clone().unwrap_or_default(),
                email: Some(email),
                added_by_pubkey: actor.pubkey.clone(),
                added_at: context.now,
            });
        }
        CustomerOp::AttachFile => {
            if next.attachments.len() >= MAX_ATTACHMENTS {
                return Err(error(
                    PlanErrorKind::Conflict,
                    "this record already holds the most attachments it can keep",
                ));
            }
            let file_ref = action.file_ref.clone().unwrap_or_default();
            let same = next
                .attachments
                .iter()
                .any(|a| a.kind == AttachmentKind::File && a.file_refs == [file_ref.clone()]);
            if same {
                return Ok(CustomerChange::Unchanged(
                    "that file is already on the record",
                ));
            }
            next.attachments.push(Attachment {
                attachment_id: id,
                kind: AttachmentKind::File,
                label: action.label.clone().unwrap_or_default().trim().to_owned(),
                file_refs: vec![file_ref],
                email: None,
                added_by_pubkey: actor.pubkey.clone(),
                added_at: context.now,
            });
        }
        CustomerOp::RemoveAttachment => {
            let target = action.attachment_id.as_deref().unwrap_or_default();
            let Some(position) = next
                .attachments
                .iter()
                .position(|a| a.attachment_id == target)
            else {
                return Err(error(
                    PlanErrorKind::Conflict,
                    "that attachment is not on this record",
                ));
            };
            if actor.role != PlanActorRole::Admin
                && next.attachments[position].added_by_pubkey != actor.pubkey
            {
                return Err(error(
                    PlanErrorKind::Forbidden,
                    "only the owner, an admin or whoever added an attachment can remove it",
                ));
            }
            next.attachments.remove(position);
        }
    }
    Ok(CustomerChange::Changed(next))
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    const OWNER: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    const AYA: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
    const SCOUT: &str = "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc";
    const STRANGER: &str = "dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd";
    const HEX: &str = "eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee";

    fn id(n: u128) -> Uuid {
        Uuid::from_u128(n)
    }

    fn party() -> String {
        id(100).hyphenated().to_string()
    }

    fn prospect(created_at: i64) -> SourceProspect {
        let head = serde_json::from_value::<ProspectHead>(json!({
            "schemaVersion": 1,
            "prospectId": id(1),
            "status": "active",
            "prospect": {
                "prospectId": id(1),
                "party": {
                    "partyId": id(100),
                    "partyType": "organization",
                    "displayName": "Plane Tree Guesthouse",
                    "externalIds": []
                },
                "industry": "Hospitality",
                "vertical": "guesthouses",
                "fitScore": 87,
                "discoveryTags": [],
                "website": null,
                "location": "Parkhurst",
                "email": null,
                "phone": "+27 82 555 7135",
                "evidence": [],
                "lastVerifiedAt": null,
                "qualification": "qualified",
                "saved": true,
                "stage": "qualified",
                "lostReason": null
            },
            "activities": [],
            "sourceActionEventId": HEX
        }))
        .expect("prospect head");
        SourceProspect {
            head,
            created_at,
            created_by_pubkey: Some(AYA.into()),
        }
    }

    fn client(at: i64) -> SourceClient {
        let head = serde_json::from_value::<ClientHead>(json!({
            "schemaVersion": 1,
            "clientId": id(200),
            "partyId": id(100),
            "displayName": "Plane Tree Guesthouse",
            "approverPubkeys": [],
            "status": "active",
            "sourceActionEventId": HEX
        }))
        .expect("client head");
        SourceClient { head, at }
    }

    fn proposal_version(revision: u32, at: i64, event: &str, unit: i64) -> SourceProposalVersion {
        let version = serde_json::from_value::<ProposalVersion>(json!({
            "schemaVersion": 1,
            "proposalId": id(300),
            "prospectPartyId": id(100),
            "namedAcceptorPubkey": OWNER,
            "revision": revision,
            "previousVersionEventId": null,
            "expiresAt": null,
            "currency": "ZAR",
            "lines": [{
                "serviceId": null,
                "description": "Website build",
                "quantityHundredths": 100,
                "unitAmountMinor": unit
            }],
            "terms": "50 percent deposit"
        }))
        .expect("proposal version");
        SourceProposalVersion {
            event_id: event.into(),
            at,
            author_pubkey: AYA.into(),
            version,
        }
    }

    fn acceptance(at: i64, version_event: &str) -> SourceAcceptance {
        let acceptance = serde_json::from_value::<ProposalAcceptance>(json!({
            "schemaVersion": 1,
            "proposalId": id(300),
            "proposalVersionEventId": version_event,
            "proposalVersionDigest": HEX,
            "conversionId": id(400),
            "clientId": id(200),
            "workItemId": id(500),
            "draftInvoiceId": id(600),
            "evidence": null
        }))
        .expect("acceptance");
        SourceAcceptance {
            event_id: "f".repeat(64),
            at,
            actor_pubkey: OWNER.into(),
            acceptance,
        }
    }

    fn payment(n: u128, amount: i64, currency: &str, at: i64) -> SourcePayment {
        let payment = serde_json::from_value::<PaymentEvidence>(json!({
            "schemaVersion": 1,
            "clientId": id(200),
            "invoiceId": id(600),
            "paymentId": id(n),
            "provider": "payfast",
            "providerReference": null,
            "amountMinor": amount,
            "currency": currency,
            "occurredAt": at,
            "evidenceRef": "link",
            "expectedInvoiceHeadEventId": HEX
        }))
        .expect("payment");
        SourcePayment {
            event_id: format!("{n:064x}"),
            recorded_by_pubkey: OWNER.into(),
            payment,
        }
    }

    fn invoice(status: &str, total: i64, outstanding: i64, issued_at: i64) -> InvoiceHead {
        serde_json::from_value(json!({
            "schemaVersion": 1,
            "clientId": id(200),
            "invoiceId": id(600),
            "proposalId": id(300),
            "proposalVersionEventId": HEX,
            "currency": "ZAR",
            "lines": [],
            "totalMinor": total,
            "creditedMinor": 0,
            "writtenOffMinor": 0,
            "collectedMinor": total - outstanding,
            "outstandingMinor": outstanding,
            "paymentEvidenceCount": 1,
            "version": 1,
            "currentVersionEventId": HEX,
            "status": status,
            "dueAt": null,
            "issuedAt": issued_at,
            "sourceEventId": HEX
        }))
        .expect("invoice")
    }

    fn job() -> PlaybookJobHead {
        let step = |n: u32, status: &str| {
            json!({
                "stepId": format!("s{n}"),
                "title": format!("Step {n}"),
                "owner": {"kind": "owner"},
                "status": status,
                "statusAt": 0
            })
        };
        let mut steps = Vec::new();
        for n in 1..=7 {
            steps.push(step(n, if n < 3 { "done" } else { "pending" }));
        }
        serde_json::from_value(json!({
            "schemaVersion": 1,
            "jobId": id(700).hyphenated().to_string(),
            "playbookId": id(800).hyphenated().to_string(),
            "playbookVersion": 1,
            "playbookTitle": "Delivery playbook",
            "title": "Plane Tree Guesthouse",
            "status": "running",
            "currentStepId": "s3",
            "steps": steps,
            "approvals": [],
            "handoffs": [],
            "startedByPubkey": AYA,
            "startedAt": 9_000,
            "updatedByPubkey": AYA,
            "updatedAt": 9_000,
            "sourceActionEventId": HEX
        }))
        .expect("job")
    }

    fn row(
        n: u128,
        outbound: bool,
        state: &str,
        body: &str,
        at: i64,
        batch: Option<WhatsappBatchFact>,
    ) -> WhatsappRow {
        WhatsappRow {
            id: id(n).hyphenated().to_string(),
            outbound,
            state: state.into(),
            body: body.into(),
            template_name: outbound.then(|| "has_website".to_owned()),
            template_title: outbound.then(|| "Has a website: open to a chat?".to_owned()),
            new_chat: outbound,
            at,
            sender_pubkey: outbound.then(|| AYA.to_owned()),
            batch,
        }
    }

    /// Plane Tree Guesthouse, from lead found to a project at step 3 of 7.
    fn plane_tree_source() -> CustomerSource {
        let v1 = "1".repeat(64);
        let mut messages = vec![
            row(
                10,
                true,
                "delivered",
                "Hi, do you have a website?",
                2_000,
                Some(WhatsappBatchFact {
                    id: id(11).hyphenated().to_string(),
                    approved: true,
                    decided_at: Some(1_500),
                    decided_by_pubkey: Some(OWNER.into()),
                }),
            ),
            row(12, false, "received", "Yes, tell me more", 3_000, None),
        ];
        // Fourteen more chat messages after the reply: 16 in all.
        for n in 0..14 {
            messages.push(row(
                20 + n,
                n % 2 == 0,
                "delivered",
                "ok",
                3_100 + n as i64,
                None,
            ));
        }
        let facts = CustomerFacts {
            prospects: vec![prospect(1_000)],
            clients: vec![client(6_000)],
            proposals: vec![proposal_version(1, 4_000, &v1, 900_000)],
            acceptances: vec![acceptance(6_000, &v1)],
            payments: vec![payment(900, 450_000, "ZAR", 8_000)],
            invoices: vec![invoice("issued", 900_000, 450_000, 7_500)],
            jobs: vec![job()],
        };
        let mut source = facts.source_for(&party()).expect("source");
        source.whatsapp = Some(SourceWhatsapp {
            wa_id: "27825557135".into(),
            opted_out: false,
            rows: messages,
        });
        source
    }

    fn manual_with_milestones() -> ManualState {
        ManualState {
            milestones: vec![
                Milestone {
                    milestone_id: "1".repeat(64),
                    kind: MilestoneKind::CallBooked,
                    title: None,
                    at: 3_500,
                    note: Some("Google Calendar, Fri 30 Oct 14:00".into()),
                    file_ref: None,
                    by_pubkey: AYA.into(),
                    recorded_at: 3_500,
                },
                Milestone {
                    milestone_id: "2".repeat(64),
                    kind: MilestoneKind::AgreementSigned,
                    title: None,
                    at: 7_000,
                    note: Some("E-sign, signed by Thandeka".into()),
                    file_ref: Some("blossom-agreement".into()),
                    by_pubkey: AYA.into(),
                    recorded_at: 7_000,
                },
            ],
            attachments: Vec::new(),
            conversion: None,
        }
    }

    fn kinds(head: &CustomerRecordHead) -> Vec<TimelineKind> {
        head.timeline.iter().map(|entry| entry.kind).collect()
    }

    #[test]
    fn plane_tree_timeline_is_in_the_order_of_the_screen_with_exact_money() {
        let source = plane_tree_source();
        let head = compute_customer_record(&source, &manual_with_milestones(), 10_000, SCOUT, None);

        assert_eq!(
            kinds(&head),
            vec![
                TimelineKind::Found,
                TimelineKind::OpenerApproved,
                TimelineKind::OpenerSent,
                TimelineKind::Replied,
                TimelineKind::CallBooked,
                TimelineKind::QuoteDrafted,
                TimelineKind::QuoteAccepted,
                TimelineKind::Converted,
                TimelineKind::AgreementSigned,
                TimelineKind::InvoiceIssued,
                TimelineKind::PaymentReceived,
                TimelineKind::ProjectStarted,
            ]
        );
        let ats = head.timeline.iter().map(|e| e.at).collect::<Vec<_>>();
        assert_eq!(
            ats,
            vec![
                1_000, 1_500, 2_000, 3_000, 3_500, 4_000, 6_000, 6_000, 7_000, 7_500, 8_000, 9_000
            ]
        );
        assert_eq!(head.timeline[0].title, "Found in Discovery");
        assert_eq!(
            head.timeline[0].detail.as_deref(),
            Some("guesthouses in Parkhurst")
        );
        assert_eq!(head.timeline[0].actor_pubkey.as_deref(), Some(AYA));
        assert_eq!(
            head.timeline[1].detail.as_deref(),
            Some("Template: Has a website: open to a chat?")
        );
        assert_eq!(head.timeline[1].actor_pubkey.as_deref(), Some(OWNER));
        assert_eq!(
            head.timeline[3].detail.as_deref(),
            Some("Yes, tell me more")
        );
        assert_eq!(head.timeline[10].title, "Deposit paid");
        assert_eq!(
            head.timeline[11].detail.as_deref(),
            Some("Delivery playbook \u{00b7} step 3 of 7: Step 3")
        );

        assert_eq!(head.money.currency.as_deref(), Some("ZAR"));
        assert_eq!(head.money.quote_total_minor, Some(900_000));
        assert_eq!(head.money.deposit_paid_minor, Some(450_000));
        assert_eq!(head.money.deposit_paid_at, Some(8_000));
        assert_eq!(head.money.balance_due_minor, Some(450_000));
        assert_eq!(head.money.other_currency_facts, 0);

        assert_eq!(head.stage, CustomerStage::Client);
        assert_eq!(head.display_name, "Plane Tree Guesthouse");
        let project = head.project.as_ref().expect("project");
        assert_eq!(project.step_number, Some(3));
        assert_eq!(project.step_count, 7);
        let chats = head.chats.whatsapp.as_ref().expect("whatsapp");
        assert_eq!(chats.message_count, 16);
        assert_eq!(chats.last_message_at, Some(3_113));
        validate_customer_head(&head).expect("valid head");
        // Computing again from the same records gives the same record.
        let again =
            compute_customer_record(&source, &ManualState::from_head(&head), 10_000, SCOUT, None);
        assert_eq!(head, again);
    }

    #[test]
    fn a_manually_added_prospect_has_no_discovery_history() {
        let mut source = plane_tree_source();
        let prospect = source.prospect.as_mut().expect("prospect");
        prospect.head.prospect.origin = Some(crate::business_records::ProspectOrigin::Manual);
        prospect.head.prospect.fit_score = None;
        prospect.head.prospect.discovery_tags.clear();
        prospect.head.prospect.evidence.clear();

        let head = compute_customer_record(&source, &ManualState::default(), 10_000, SCOUT, None);

        assert_eq!(head.found, None);
        assert!(!head
            .timeline
            .iter()
            .any(|entry| entry.kind == TimelineKind::Found));
    }

    #[test]
    fn money_is_null_when_unknown_and_never_zero() {
        let facts = CustomerFacts {
            prospects: vec![prospect(1_000)],
            ..CustomerFacts::default()
        };
        let source = facts.source_for(&party()).expect("source");
        let head = compute_customer_record(&source, &ManualState::default(), 5_000, SCOUT, None);
        assert_eq!(head.money.currency, None);
        assert_eq!(head.money.quote_total_minor, None);
        assert_eq!(head.money.deposit_paid_minor, None);
        assert_eq!(head.money.balance_due_minor, None);
        assert_eq!(head.stage, CustomerStage::Lead);
        assert_eq!(kinds(&head), vec![TimelineKind::Found]);

        // A quote with nothing paid and no invoice states only the quote.
        let v1 = "1".repeat(64);
        let facts = CustomerFacts {
            prospects: vec![prospect(1_000)],
            proposals: vec![proposal_version(1, 2_000, &v1, 250_000)],
            ..CustomerFacts::default()
        };
        let source = facts.source_for(&party()).expect("source");
        let head = compute_customer_record(&source, &ManualState::default(), 5_000, SCOUT, None);
        assert_eq!(head.money.quote_total_minor, Some(250_000));
        assert_eq!(head.money.deposit_paid_minor, None);
        assert_eq!(head.money.balance_due_minor, None);
    }

    #[test]
    fn a_payment_in_another_currency_is_left_out_and_counted() {
        let v1 = "1".repeat(64);
        let facts = CustomerFacts {
            prospects: vec![prospect(1_000)],
            clients: vec![client(3_000)],
            proposals: vec![proposal_version(1, 2_000, &v1, 100_000)],
            payments: vec![
                payment(1, 25_000, "ZAR", 4_000),
                payment(2, 900, "USD", 5_000),
            ],
            ..CustomerFacts::default()
        };
        let source = facts.source_for(&party()).expect("source");
        let head = compute_customer_record(&source, &ManualState::default(), 9_000, SCOUT, None);
        assert_eq!(head.money.deposit_paid_minor, Some(25_000));
        assert_eq!(head.money.balance_due_minor, Some(75_000));
        assert_eq!(head.money.other_currency_facts, 1);
    }

    #[test]
    fn converting_keeps_every_entry_and_converting_twice_is_a_no_op() {
        // The lead before conversion: found, opener, reply, call booked.
        let mut source = plane_tree_source();
        source.clients.clear();
        source.acceptances.clear();
        source.payments.clear();
        source.invoices.clear();
        source.jobs.clear();
        let manual = manual_with_milestones();
        let before = compute_customer_record(&source, &manual, 5_000, SCOUT, None);
        assert_eq!(before.stage, CustomerStage::Lead);
        assert_eq!(before.conversion, None);

        let owner = actor(OWNER, PlanActorRole::Admin, &[]);
        let ctx = context(5_000, "9".repeat(64));
        let convert = action(CustomerOp::ConvertToClient);
        let CustomerChange::Changed(converted) = apply_customer_action(
            &ManualState::from_head(&before),
            false,
            &convert,
            &owner,
            &ctx,
        )
        .expect("owner converts") else {
            panic!("converting a lead changes the record");
        };
        let after = compute_customer_record(
            &source,
            &converted,
            5_000,
            OWNER,
            Some(&ctx.action_event_id),
        );
        assert_eq!(after.stage, CustomerStage::Client);
        assert_eq!(
            after.party_id, before.party_id,
            "one party id across lead and client"
        );
        assert_eq!(after.lead_id, before.lead_id);
        for entry in &before.timeline {
            assert!(
                after.timeline.contains(entry),
                "conversion dropped the entry {}",
                entry.entry_id
            );
        }
        assert_eq!(after.timeline.len(), before.timeline.len() + 1);
        let converted_entry = after
            .timeline
            .iter()
            .find(|entry| entry.kind == TimelineKind::Converted)
            .expect("converted entry");
        assert_eq!(converted_entry.actor_pubkey.as_deref(), Some(OWNER));

        // Converting again changes nothing.
        let again = apply_customer_action(
            &ManualState::from_head(&after),
            false,
            &convert,
            &owner,
            &ctx,
        )
        .expect("second convert");
        assert_eq!(again, CustomerChange::Unchanged("already a client"));

        // The client record appearing later keeps the first conversion, so the
        // lead's history and the conversion time never move.
        let mut with_client = plane_tree_source();
        with_client.payments.clear();
        let carried = compute_customer_record(
            &with_client,
            &ManualState::from_head(&after),
            9_000,
            SCOUT,
            None,
        );
        assert_eq!(carried.conversion, after.conversion);
        assert!(after.timeline.iter().all(|e| carried.timeline.contains(e)));
    }

    #[test]
    fn a_client_created_directly_still_gets_a_record() {
        let facts = CustomerFacts {
            clients: vec![client(3_000)],
            ..CustomerFacts::default()
        };
        assert_eq!(facts.party_ids(), vec![party()]);
        let source = facts.source_for(&party()).expect("source");
        let head = compute_customer_record(&source, &ManualState::default(), 4_000, SCOUT, None);
        assert_eq!(head.stage, CustomerStage::Client);
        assert_eq!(head.lead_id, None);
        assert_eq!(head.found, None);
        assert_eq!(
            head.client_id.as_deref(),
            Some(id(200).hyphenated().to_string().as_str())
        );
        assert_eq!(kinds(&head), vec![TimelineKind::Converted]);
        assert_eq!(head.timeline[0].at, 3_000);
        assert_eq!(
            head.conversion.as_ref().map(|c| c.source),
            Some(ConversionSource::ClientRecord)
        );
        // A party with no record at all has no customer.
        assert!(facts
            .source_for(&id(999).hyphenated().to_string())
            .is_none());
    }

    #[test]
    fn the_whole_company_is_split_by_business() {
        let mut other = prospect(2_000);
        other.head.prospect.party.party_id = id(101);
        let facts = CustomerFacts {
            prospects: vec![prospect(1_000), other],
            ..CustomerFacts::default()
        };
        assert_eq!(
            facts.party_ids(),
            vec![
                id(100).hyphenated().to_string(),
                id(101).hyphenated().to_string()
            ]
        );
        let source = facts
            .source_for(&id(101).hyphenated().to_string())
            .expect("source");
        assert_eq!(
            source
                .prospect
                .expect("prospect")
                .head
                .prospect
                .party
                .party_id,
            id(101)
        );
    }

    fn actor(pubkey: &str, role: PlanActorRole, led: &[CompanyFunction]) -> PlanActor {
        PlanActor {
            pubkey: pubkey.into(),
            role,
            led_functions: led.to_vec(),
        }
    }

    fn context(now: i64, action_event_id: String) -> PlanContext {
        PlanContext {
            now,
            cofounder_enabled: true,
            action_event_id,
        }
    }

    fn action(op: CustomerOp) -> CustomerAction {
        CustomerAction {
            schema_version: COMPANY_RECORD_SCHEMA_VERSION,
            party_id: party(),
            op,
            milestone_kind: None,
            at: None,
            title: None,
            note: None,
            file_ref: None,
            from: None,
            subject: None,
            date: None,
            summary: None,
            file_refs: None,
            label: None,
            attachment_id: None,
            expected_head_event_id: None,
        }
    }

    fn email_action() -> CustomerAction {
        CustomerAction {
            from: Some("Thandeka <thandeka@planetree.example>".into()),
            subject: Some("Logo and photos".into()),
            date: Some(1_793_000_000),
            summary: Some("Sent the logo and 14 photos for the site.".into()),
            ..action(CustomerOp::AttachEmail)
        }
    }

    #[test]
    fn a_stranger_cannot_write_and_each_role_gets_only_its_ops() {
        let ctx = context(1_793_000_100, "7".repeat(64));
        let member = actor(STRANGER, PlanActorRole::Member, &[]);
        let sales_lead = actor(AYA, PlanActorRole::ManagedAgent, &[CompanyFunction::Sales]);
        let cs_lead = actor(
            AYA,
            PlanActorRole::Member,
            &[CompanyFunction::CustomerSuccess],
        );
        let scout = actor(SCOUT, PlanActorRole::ManagedAgent, &[]);
        let owner = actor(OWNER, PlanActorRole::Admin, &[]);
        let ops = [
            CustomerOp::Refresh,
            CustomerOp::ConvertToClient,
            CustomerOp::RecordMilestone,
            CustomerOp::AttachEmail,
            CustomerOp::AttachFile,
            CustomerOp::RemoveAttachment,
        ];
        for op in ops {
            let probe = action(op);
            let refused = authorize_customer_action(&probe, &member, &ctx)
                .expect_err("a member with no lead slot is refused");
            assert_eq!(refused.kind, PlanErrorKind::Forbidden, "{op:?}");
            authorize_customer_action(&probe, &owner, &ctx).expect("owner");
            assert!(
                authorize_customer_action(&probe, &sales_lead, &ctx).is_ok(),
                "{op:?}"
            );
        }
        // Scout and a Customer Success lead write everything but a conversion.
        for op in [
            CustomerOp::Refresh,
            CustomerOp::RecordMilestone,
            CustomerOp::AttachEmail,
            CustomerOp::AttachFile,
            CustomerOp::RemoveAttachment,
        ] {
            let probe = action(op);
            assert!(
                authorize_customer_action(&probe, &scout, &ctx).is_ok(),
                "{op:?}"
            );
            assert!(
                authorize_customer_action(&probe, &cs_lead, &ctx).is_ok(),
                "{op:?}"
            );
        }
        let convert = action(CustomerOp::ConvertToClient);
        for who in [&scout, &cs_lead] {
            let refused =
                authorize_customer_action(&convert, who, &ctx).expect_err("not the Sales lead");
            assert!(
                refused.message.contains("Sales lead"),
                "{}",
                refused.message
            );
        }
        // Co-founder mode off stops everyone, the owner included.
        let off = PlanContext {
            cofounder_enabled: false,
            ..ctx
        };
        let refused = authorize_customer_action(&action(CustomerOp::Refresh), &owner, &off)
            .expect_err("mode off");
        assert!(refused.message.contains("co-founder mode is off"));
    }

    #[test]
    fn milestones_are_recorded_deduplicated_and_shaped_for_a_provider() {
        let ctx = context(1_793_000_100, "7".repeat(64));
        let scout = actor(SCOUT, PlanActorRole::ManagedAgent, &[]);
        let call = CustomerAction {
            milestone_kind: Some(MilestoneKind::CallBooked),
            at: Some(1_793_000_000),
            note: Some("Google Calendar, Fri 30 Oct 14:00".into()),
            ..action(CustomerOp::RecordMilestone)
        };
        let CustomerChange::Changed(state) =
            apply_customer_action(&ManualState::default(), false, &call, &scout, &ctx)
                .expect("record")
        else {
            panic!("changes the record");
        };
        assert_eq!(state.milestones.len(), 1);
        assert_eq!(state.milestones[0].milestone_id, ctx.action_event_id);
        assert_eq!(state.milestones[0].by_pubkey, SCOUT);
        // The same call recorded again (a provider retry) changes nothing.
        let again = apply_customer_action(&state, false, &call, &scout, &ctx).expect("again");
        assert_eq!(
            again,
            CustomerChange::Unchanged("that milestone is already recorded")
        );

        let future = CustomerAction {
            at: Some(1_793_000_100 + 3_600),
            ..call.clone()
        };
        let refused = apply_customer_action(&ManualState::default(), false, &future, &scout, &ctx)
            .expect_err("future");
        assert!(refused.message.contains("future"));

        let other = CustomerAction {
            milestone_kind: Some(MilestoneKind::Other),
            title: Some("Quote approved and sent".into()),
            ..call.clone()
        };
        validate_customer_action(&other).expect("other with title");
        let untitled = CustomerAction {
            title: None,
            ..other.clone()
        };
        assert!(validate_customer_action(&untitled).is_err());
        let titled_call = CustomerAction {
            title: Some("x".into()),
            ..call
        };
        assert!(validate_customer_action(&titled_call).is_err());
    }

    #[test]
    fn emails_are_short_summaries_and_removal_belongs_to_the_adder_or_the_owner() {
        let ctx = context(1_793_000_100, "7".repeat(64));
        let scout = actor(SCOUT, PlanActorRole::ManagedAgent, &[]);
        let owner = actor(OWNER, PlanActorRole::Admin, &[]);
        let cs = actor(AYA, PlanActorRole::ManagedAgent, &[]);

        let long = CustomerAction {
            summary: Some("x".repeat(MAX_EMAIL_SUMMARY_BYTES + 1)),
            ..email_action()
        };
        let refused = validate_customer_action(&long)
            .expect_err("over 4 KB")
            .to_string();
        assert!(refused.contains("4 KB"), "{refused}");
        let at_limit = CustomerAction {
            summary: Some("x".repeat(MAX_EMAIL_SUMMARY_BYTES)),
            ..email_action()
        };
        validate_customer_action(&at_limit).expect("exactly 4 KB is allowed");

        let CustomerChange::Changed(state) = apply_customer_action(
            &ManualState::default(),
            false,
            &email_action(),
            &scout,
            &ctx,
        )
        .expect("attach email") else {
            panic!("changes the record");
        };
        assert_eq!(state.attachments.len(), 1);
        assert_eq!(state.attachments[0].kind, AttachmentKind::Email);
        assert_eq!(
            apply_customer_action(&state, false, &email_action(), &scout, &ctx).expect("again"),
            CustomerChange::Unchanged("that email is already on the record")
        );

        let remove = CustomerAction {
            attachment_id: Some(ctx.action_event_id.clone()),
            ..action(CustomerOp::RemoveAttachment)
        };
        let refused = apply_customer_action(&state, false, &remove, &cs, &ctx)
            .expect_err("another agent cannot remove it");
        assert_eq!(refused.kind, PlanErrorKind::Forbidden);
        assert!(matches!(
            apply_customer_action(&state, false, &remove, &scout, &ctx).expect("adder removes"),
            CustomerChange::Changed(next) if next.attachments.is_empty()
        ));
        assert!(matches!(
            apply_customer_action(&state, false, &remove, &owner, &ctx).expect("owner removes"),
            CustomerChange::Changed(next) if next.attachments.is_empty()
        ));
        let missing = CustomerAction {
            attachment_id: Some("0".repeat(64)),
            ..remove
        };
        let refused =
            apply_customer_action(&state, false, &missing, &owner, &ctx).expect_err("missing");
        assert_eq!(refused.kind, PlanErrorKind::Conflict);
    }

    #[test]
    fn attachments_show_on_the_timeline_and_in_the_chats_panel() {
        let ctx = context(1_793_000_100, "7".repeat(64));
        let scout = actor(SCOUT, PlanActorRole::ManagedAgent, &[]);
        let file = CustomerAction {
            label: Some("Agreement, signed".into()),
            file_ref: Some("blossom-sha256-abc".into()),
            ..action(CustomerOp::AttachFile)
        };
        let CustomerChange::Changed(state) =
            apply_customer_action(&ManualState::default(), false, &file, &scout, &ctx)
                .expect("file")
        else {
            panic!("changes the record");
        };
        let CustomerChange::Changed(state) =
            apply_customer_action(&state, false, &email_action(), &scout, &ctx_with("8"))
                .expect("email")
        else {
            panic!("changes the record");
        };
        let facts = CustomerFacts {
            prospects: vec![prospect(1_000)],
            ..CustomerFacts::default()
        };
        let source = facts.source_for(&party()).expect("source");
        let head = compute_customer_record(&source, &state, 1_793_000_200, SCOUT, None);
        assert_eq!(head.chats.email_count, 1);
        assert_eq!(head.attachments.len(), 2);
        assert!(kinds(&head).contains(&TimelineKind::Email));
        assert!(kinds(&head).contains(&TimelineKind::File));
        validate_customer_head(&head).expect("valid");
    }

    fn ctx_with(digit: &str) -> PlanContext {
        context(1_793_000_100, digit.repeat(64))
    }

    #[test]
    fn each_op_takes_only_its_own_fields() {
        let mut stray = action(CustomerOp::Refresh);
        stray.label = Some("nope".into());
        assert!(validate_customer_action(&stray).is_err());
        let mut missing = action(CustomerOp::RecordMilestone);
        missing.milestone_kind = Some(MilestoneKind::CallBooked);
        assert!(
            validate_customer_action(&missing).is_err(),
            "at is required"
        );
        let mut bad_head = action(CustomerOp::Refresh);
        bad_head.expected_head_event_id = Some("zz".into());
        assert!(validate_customer_action(&bad_head).is_err());
        let mut bad_party = action(CustomerOp::Refresh);
        bad_party.party_id = "not-a-uuid".into();
        assert!(validate_customer_action(&bad_party).is_err());
        let json = serde_json::to_string(&action(CustomerOp::Refresh)).expect("json");
        assert!(json.contains("\"op\":\"refresh\""), "{json}");
        assert!(
            serde_json::from_str::<CustomerAction>(&json.replace("}", ",\"extra\":1}")).is_err()
        );
    }

    #[test]
    fn d_tags_round_trip_and_refresh_is_throttled() {
        let d_tag = customer_d_tag(&party()).expect("d tag");
        assert_eq!(d_tag, format!("customer:{}", party()));
        assert_eq!(parse_customer_d_tag(&d_tag).expect("party"), party());
        assert!(parse_customer_d_tag("company:scorecard").is_err());
        // A party id with letters in it must be lowercase.
        let lettered = id(0xabc).hyphenated().to_string();
        customer_d_tag(&lettered).expect("lowercase is accepted");
        assert!(customer_d_tag(&lettered.to_uppercase()).is_err());

        let source = plane_tree_source();
        let head = compute_customer_record(&source, &ManualState::default(), 10_000, SCOUT, None);
        assert!(customer_refresh_is_fresh(&head, 10_059));
        assert!(!customer_refresh_is_fresh(&head, 10_060));
    }

    #[test]
    fn a_running_job_and_an_unsent_opener_read_honestly() {
        let mut source = plane_tree_source();
        // The opener is approved but still queued: approved shows, sent and reply do not.
        source.whatsapp = Some(SourceWhatsapp {
            wa_id: "27825557135".into(),
            opted_out: false,
            rows: vec![row(
                10,
                true,
                "queued",
                "Hi",
                2_000,
                Some(WhatsappBatchFact {
                    id: id(11).hyphenated().to_string(),
                    approved: true,
                    decided_at: Some(1_500),
                    decided_by_pubkey: Some(OWNER.into()),
                }),
            )],
        });
        source.jobs.clear();
        let head = compute_customer_record(&source, &ManualState::default(), 10_000, SCOUT, None);
        assert!(kinds(&head).contains(&TimelineKind::OpenerApproved));
        assert!(!kinds(&head).contains(&TimelineKind::OpenerSent));
        assert!(!kinds(&head).contains(&TimelineKind::Replied));
        assert_eq!(head.project, None);
    }
}
