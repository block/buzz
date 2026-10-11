//! The NIP-FP verifier report.
//!
//! A report has three sections that are never merged: what the subject
//! [`Selection`] lists, what the viewer [`Observed`] by fetching events itself,
//! and the [`Interpretation`] under a named policy. Nothing here is a score.

use serde::Serialize;

/// A complete verifier report (NIP-FP §Verifier report).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Report {
    /// Always `"NIP-FP"`.
    pub nip: &'static str,
    /// The `until` bound of every discovery query, in Unix seconds.
    pub cutoff: u64,
    /// The fingerprint as signed.
    pub selection: Selection,
    /// Everything the verifier fetched, and from where.
    pub observed: Observed,
    /// The viewer's reading of the observed events under a named policy.
    pub interpretation: Interpretation,
}

/// The `selection` section: the fingerprint exactly as signed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Selection {
    /// `"transport bundle (pre-registration)"` when the fingerprint arrived in
    /// a transport bundle. A bundle is not a ledger entry or a grant.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub transport: Option<&'static str>,
    /// Whether the fingerprint is a validly signed, well-formed `30340` event.
    pub fingerprint: Validity,
    /// Why the fingerprint is invalid, when it is.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub invalid_reason: Option<String>,
    /// Fingerprint event id, as claimed.
    pub id: Option<String>,
    /// Signing pubkey, as claimed.
    pub subject: Option<String>,
    /// The `d` tag.
    pub d: Option<String>,
    /// `created_at`, as claimed.
    pub created_at: Option<u64>,
    /// Always `"selected"`, whatever the fingerprint declares.
    pub coverage: &'static str,
    /// The subject's owner link on the fingerprint.
    pub owner_link: OwnerLink,
    /// The selection tags, in order, up to the limit.
    pub items: Vec<SelectionItem>,
    /// Selection tags beyond the limit. They are reported as gaps.
    pub excess: usize,
}

/// One `e` selection tag.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SelectionItem {
    /// Selected event id.
    pub id: String,
    /// Origin to fetch it from; `None` when the tag names none.
    pub origin: Option<String>,
}

/// Whether bytes are a validly signed event.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Validity {
    /// Id and signature verify.
    Valid,
    /// Id or signature does not verify, or the event is malformed.
    Invalid,
}

/// The `observed` section: coverage records and every reported event.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Observed {
    /// One record per query sent, indexed by [`CoverageRecord::scope`].
    pub coverage: Vec<CoverageRecord>,
    /// Every reported event, selected or discovered.
    pub items: Vec<Item>,
}

/// What one query asked, of whom, and what came back.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CoverageRecord {
    /// Index of this record; items cite it in [`Item::scope`].
    pub scope: usize,
    /// The origin queried.
    pub origin: String,
    /// The NIP-01 filter sent, or `"nip11"` for the relay information document.
    pub request: serde_json::Value,
    /// The cutoff applied, when the filter has one.
    pub cutoff: Option<u64>,
    /// Events returned that matched the filter.
    pub returned: usize,
    /// Events returned that did not match the filter or did not parse. They
    /// are discarded.
    pub discarded: usize,
    /// Whether the result reached the filter's `limit`.
    pub truncated: bool,
    /// Why the query failed, when it did.
    pub error: Option<String>,
}

/// One reported event.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Item {
    /// Event id (as selected or as claimed).
    pub id: String,
    /// Origin it was fetched from, or would have been.
    pub origin: Option<String>,
    /// Kind, when fetched.
    pub kind: Option<u16>,
    /// Signer, when fetched.
    pub pubkey: Option<String>,
    /// `created_at` as asserted by the signer, when fetched.
    pub created_at: Option<u64>,
    /// Coverage records that returned it.
    pub scope: Vec<usize>,
    /// What the event is to this report.
    pub role: Role,
    /// For work events: whether the fingerprint lists it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub selected: Option<bool>,
    /// For outcomes: the work event it targets.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target: Option<String>,
    /// For outcomes: whether the fingerprint lists the target.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target_selected: Option<bool>,
    /// For outcomes: the claim, from the outcome vocabulary.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub claim: Option<&'static str>,
    /// For outcomes: the claim's polarity.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub polarity: Option<Polarity>,
    /// For outcomes: the signer's authority for the claim at the time.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub authority: Option<Authority>,
    /// For merge statuses: whether a trusted ref state corroborates it.
    #[serde(rename = "ref", skip_serializing_if = "Option::is_none")]
    pub ref_state: Option<RefCheck>,
    /// The event's own owner link, when fetched and valid.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub owner_link: Option<OwnerLink>,
    /// For outcomes: the signer's relationship to the subject.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub independence: Option<Independence>,
    /// For selected work: replies, mentions and reactions found. Attention is
    /// not an outcome.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub attention: Option<usize>,
    /// The bucket this item lands in.
    pub bucket: Bucket,
    /// Why it landed there.
    pub reason: String,
}

/// What an event is to the report.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    /// A work event, selected or discovered.
    Work,
    /// The event a work event replies to; its author is the requester.
    Request,
    /// A status, label or retraction about a work event.
    Outcome,
    /// A `30617` repository announcement.
    Authority,
    /// A `30618` repository state.
    Ref,
}

/// The polarity of an outcome claim.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Polarity {
    /// Merged, resolved, accepted, reused.
    Positive,
    /// Closed, reopened, corrected, rejected, disputed, retracted.
    Contrary,
    /// Open, draft.
    Neutral,
}

/// A signer's authority for a claim, with its basis.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Authority {
    /// The authority status.
    pub status: AuthorityStatus,
    /// What the status rests on, e.g. "owner of `30617:<owner>:<d>`".
    pub basis: String,
}

/// Authority status of an outcome's signer at the time it was signed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthorityStatus {
    /// The signer held the authority itself.
    Authorized,
    /// The signer's owner link names a key that held the authority.
    Delegated,
    /// The signer held no authority that signed events show.
    SelfAsserted,
    /// The claim is one anyone may make; it never raises a rung.
    Opinion,
    /// The authority in force at the time cannot be determined.
    Unknown,
}

/// Ref-state corroboration of a merge status.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RefCheck {
    /// The corroboration status.
    pub status: RefStatus,
    /// Which ref state and signer, or why none applies.
    pub basis: String,
}

/// Whether a ref state corroborates a merge status.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RefStatus {
    /// A trusted `30618` shows the default branch at the merge commit.
    Corroborated,
    /// No such ref state was observed.
    Unverified,
}

/// A NIP-OA owner link on one event.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case", tag = "status", content = "owner")]
pub enum OwnerLink {
    /// One `auth` tag that verifies for this event; carries the owner.
    Verified(String),
    /// No `auth` tag. Proves nothing either way.
    None,
    /// A malformed, failing, or duplicated `auth` tag. It is no link.
    Invalid,
}

/// The relationship between an outcome's signer and the subject.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Independence {
    /// The signer is the subject.
    SameSigner,
    /// An owner link connects the signer and the subject.
    SameOwner,
    /// Anything else. No signed evidence shows two keys are distinct people.
    Unknown,
}

/// The bucket an item lands in. The first that applies wins, in this order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Bucket {
    /// Not a validly signed event; evidence of nothing.
    Invalid,
    /// Could not be fetched. Neither passes nor fails.
    Inaccessible,
    /// A valid outcome with contrary polarity, whatever its authority.
    Contrary,
    /// Fetched, valid, and its claim checks out.
    Verifiable,
    /// Fetched, but its claim does not check out or could not be checked.
    Unknown,
}

/// The `interpretation` section.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Interpretation {
    /// The policy applied.
    pub policy: PolicyRef,
    /// One rung per work item. Rungs are never combined.
    pub rungs: Vec<Rung>,
    /// Contrary outcomes on work the fingerprint does not list.
    pub omitted_contrary: Vec<String>,
    /// What the verifier could not see or check.
    pub gaps: Vec<Gap>,
    /// Every coverage record, in the wording NIP-FP requires.
    pub coverage: Vec<String>,
}

/// A named, versioned policy.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PolicyRef {
    /// Policy name, e.g. `nip-fp/default`.
    pub name: String,
    /// Policy version.
    pub version: u32,
}

/// The rung of one work item, with its basis.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Rung {
    /// Work event id.
    pub id: String,
    /// Whether the fingerprint lists it.
    pub selected: bool,
    /// 0 signed, 1 attributed, 2 outcome; `None` for inaccessible, invalid or
    /// unresolvable items.
    pub rung: Option<u8>,
    /// The outcome that supports rung 2.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub basis: Option<String>,
    /// That outcome signer's independence. Never "independent".
    #[serde(skip_serializing_if = "Option::is_none")]
    pub independence: Option<Independence>,
    /// Whether the policy marks that signer as known to the viewer. This is
    /// the viewer's interpretation, not evidence.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub viewer_known: Option<bool>,
}

/// Something the verifier could not see or check.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Gap {
    /// The event concerned, if any.
    pub id: Option<String>,
    /// What is missing.
    pub reason: String,
}
