//! Stdout access log for security investigations: who read what, and who
//! tried to authenticate.
//!
//! Every line goes to the [`TARGET`] tracing target (stdout, never the
//! `audit_log` table) and carries an `event` field to filter on. Search text,
//! event content, tokens, and assertions are never recorded: filters are
//! reduced to a [`FilterSummary`], and failures to fixed reason codes.
//!
//! `event=auth` lines cover every Nostr-key login: WebSocket AUTH (root and
//! audio), NIP-98 signed HTTP requests, enterprise assertions, and Blossom
//! uploads. Each attempt writes exactly one line, except a successful root
//! WebSocket login, which the connection's `close` line carries instead. On
//! the HTTP bridge (`/query`, `/count`, `/events`) the `auth` line is the
//! request's only line and also records what the request did. Three machine-to-machine
//! checks are not Nostr-key logins and write no auth line: the command-token
//! disconnect endpoint (`/api/nip-fi/disconnect`), the workflow webhook
//! secret, and the git server's internal signed callback.

use std::net::SocketAddr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use buzz_core::CommunityId;
use nostr::{Alphabet, Filter, PublicKey, SingleLetterTag};
use tracing::Level;
use uuid::Uuid;

/// The tracing target for every access line.
pub(crate) const TARGET: &str = "buzz_relay::access";

/// Most identifiers listed per field; the matching `*_count` field is exact.
const LIST_CAP: usize = 10;

macro_rules! emit {
    ($level:expr, $($fields:tt)+) => {
        if $level == Level::WARN {
            tracing::warn!(target: TARGET, $($fields)+)
        } else {
            tracing::info!(target: TARGET, $($fields)+)
        }
    };
}

/// What a REQ or COUNT asked for, merged across its filters. List fields hold
/// the first [`LIST_CAP`] distinct values; their `*_count` fields sum the
/// per-filter set sizes, so a value repeated in two filters counts twice. A
/// list field is `*` when any filter leaves that dimension open, because that
/// filter reads every value; `since`/`until`/`limit` likewise take the widest
/// bound and are absent when any filter leaves them open. No field understates
/// what the read covered. Only well-formed channel ids and pubkeys are listed,
/// so client-chosen tag text never reaches the log.
#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct FilterSummary {
    filters: usize,
    kinds: Option<String>,
    kind_count: usize,
    channel_ids: Option<String>,
    channel_count: usize,
    authors: Option<String>,
    author_count: usize,
    p_tags: Option<String>,
    p_count: usize,
    ids: usize,
    since: Option<u64>,
    until: Option<u64>,
    limit: Option<usize>,
    search: bool,
}

/// The value logged for a list dimension that some filter leaves open.
const OPEN: &str = "*";

impl FilterSummary {
    pub(crate) fn new(filters: &[Filter]) -> Self {
        let h = SingleLetterTag::lowercase(Alphabet::H);
        let p = SingleLetterTag::lowercase(Alphabet::P);
        let (mut kinds, mut channels, mut authors, mut p_tags) = (
            Listed::default(),
            Listed::default(),
            Listed::default(),
            Listed::default(),
        );
        let mut ids = 0;
        for filter in filters {
            for k in filter.kinds.iter().flatten() {
                kinds.offer(k.as_u16());
            }
            for c in filter.generic_tags.get(&h).into_iter().flatten() {
                channels.count(Uuid::parse_str(c).is_ok().then_some(c));
            }
            for a in filter.authors.iter().flatten() {
                authors.offer(*a);
            }
            for v in filter.generic_tags.get(&p).into_iter().flatten() {
                p_tags.count(PublicKey::from_hex(v).is_ok().then_some(v));
            }
            ids += filter.ids.as_ref().map_or(0, |s| s.len());
        }
        let open = |bounded: &dyn Fn(&Filter) -> bool| filters.iter().any(|f| !bounded(f));
        let bound = |pick: fn(&Filter) -> Option<u64>, widest: fn(u64, u64) -> u64| {
            let values: Option<Vec<u64>> = filters.iter().map(pick).collect();
            values?.into_iter().reduce(widest)
        };
        let (kind_count, kinds) = kinds.finish(open(&|f| f.kinds.is_some()), u16::to_string);
        let (channel_count, channel_ids) = channels
            .finish(open(&|f| f.generic_tags.contains_key(&h)), |c| {
                c.to_string()
            });
        let (author_count, authors) =
            authors.finish(open(&|f| f.authors.is_some()), PublicKey::to_hex);
        let (p_count, p_tags) = p_tags.finish(open(&|f| f.generic_tags.contains_key(&p)), |v| {
            v.to_string()
        });
        Self {
            filters: filters.len(),
            kinds,
            kind_count,
            channel_ids,
            channel_count,
            authors,
            author_count,
            p_tags,
            p_count,
            ids,
            since: bound(|f| f.since.map(|t| t.as_secs()), u64::min),
            until: bound(|f| f.until.map(|t| t.as_secs()), u64::max),
            limit: bound(|f| f.limit.map(|l| l as u64), u64::max).map(|l| l as usize),
            search: filters.iter().any(|f| f.search.is_some()),
        }
    }
}

/// The first [`LIST_CAP`] distinct values offered, in order, and how many
/// values were seen. Working state never exceeds the cap, however long the
/// filter lists are.
struct Listed<T> {
    listed: Vec<T>,
    seen: usize,
}

impl<T> Default for Listed<T> {
    fn default() -> Self {
        Self {
            listed: Vec::new(),
            seen: 0,
        }
    }
}

impl<T: PartialEq> Listed<T> {
    fn offer(&mut self, value: T) {
        self.count(Some(value));
    }

    /// Count one value, listing it when it is `Some` (well-formed).
    fn count(&mut self, value: Option<T>) {
        self.seen += 1;
        if let Some(value) = value {
            if self.listed.len() < LIST_CAP && !self.listed.contains(&value) {
                self.listed.push(value);
            }
        }
    }

    /// The count, and the comma-joined list: [`OPEN`] when a filter leaves
    /// the dimension open, `None` when nothing was listed.
    fn finish(self, open: bool, show: impl Fn(&T) -> String) -> (usize, Option<String>) {
        let shown = if open {
            Some(OPEN.to_owned())
        } else {
            (!self.listed.is_empty())
                .then(|| self.listed.iter().map(show).collect::<Vec<_>>().join(","))
        };
        (self.seen, shown)
    }
}

/// Lines a connection may write for client-repeatable refusals before the
/// rest are only counted.
const REFUSAL_LINES: u64 = 16;

/// Per-connection bound on the access lines a client can trigger without
/// being served: reads refused or denied, and AUTH frames after the login was
/// decided. The first [`REFUSAL_LINES`] of each are logged; the rest are
/// counted and reported on the close line. Served reads are never capped.
#[derive(Debug, Default)]
pub(crate) struct RefusalBudget {
    reads: AtomicU64,
    auths: AtomicU64,
}

impl RefusalBudget {
    /// Whether this refused read may write its line.
    pub(crate) fn read(&self) -> bool {
        self.reads.fetch_add(1, Ordering::Relaxed) < REFUSAL_LINES
    }

    /// Whether this post-decision AUTH frame may write its line.
    pub(crate) fn auth(&self) -> bool {
        self.auths.fetch_add(1, Ordering::Relaxed) < REFUSAL_LINES
    }

    fn suppressed(seen: &AtomicU64) -> u64 {
        seen.load(Ordering::Relaxed).saturating_sub(REFUSAL_LINES)
    }
}

/// How a read ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ReadOutcome {
    /// Dropped before a terminal frame: connection teardown or session expiry.
    Aborted,
    Served,
    Denied(&'static str),
    Error(&'static str),
}

/// One access line per WebSocket REQ or COUNT, written when the handler
/// finishes, including when its future is dropped mid-read.
pub(crate) struct ReadLog {
    event: &'static str,
    conn_id: Uuid,
    community: CommunityId,
    pubkey: Option<String>,
    sub_id: String,
    summary: FilterSummary,
    outcome: ReadOutcome,
    /// REQ: stored events handed to the connection's writer so far.
    /// COUNT: the total returned.
    results: u64,
    /// Charged if the read ends denied; `None` once already charged.
    budget: Option<Arc<RefusalBudget>>,
}

impl ReadLog {
    pub(crate) fn req(
        conn: &crate::connection::ConnectionState,
        sub_id: &str,
        filters: &[Filter],
    ) -> Self {
        Self::new("req", conn, sub_id, filters)
    }

    pub(crate) fn count(
        conn: &crate::connection::ConnectionState,
        sub_id: &str,
        filters: &[Filter],
    ) -> Self {
        Self::new("count", conn, sub_id, filters)
    }

    /// Log a read refused before its handler ran; other frames log nothing.
    pub(crate) fn refused(
        msg: &crate::protocol::ClientMessage,
        conn: &crate::connection::ConnectionState,
        reason: &'static str,
    ) {
        use crate::protocol::ClientMessage as M;
        match msg {
            M::Req {
                sub_id, filters, ..
            } => Self::refused_read("req", conn, sub_id, filters, reason),
            M::Count { sub_id, filters } => {
                Self::refused_read("count", conn, sub_id, filters, reason)
            }
            // Admission refuses only reads and EVENTs.
            _ => {}
        }
    }

    /// Log a `event` ("req" or "count") refused before its handler ran.
    pub(crate) fn refused_read(
        event: &'static str,
        conn: &crate::connection::ConnectionState,
        sub_id: &str,
        filters: &[Filter],
        reason: &'static str,
    ) {
        if conn.refusal_budget.read() {
            let mut log = Self::new(event, conn, sub_id, filters);
            log.budget = None;
            log.deny(reason);
        }
    }

    fn new(
        event: &'static str,
        conn: &crate::connection::ConnectionState,
        sub_id: &str,
        filters: &[Filter],
    ) -> Self {
        let pubkey = match conn.auth_state_snapshot() {
            crate::connection::AuthState::Authenticated(ctx) => Some(ctx.pubkey.to_hex()),
            _ => None,
        };
        Self {
            event,
            conn_id: conn.conn_id,
            community: conn.tenant.community(),
            pubkey,
            sub_id: sub_id.to_owned(),
            // The summary is the costly part; skip it when the line is off.
            summary: if tracing::enabled!(target: TARGET, Level::INFO) {
                FilterSummary::new(filters)
            } else {
                FilterSummary::default()
            },
            outcome: ReadOutcome::Aborted,
            results: 0,
            budget: Some(Arc::clone(&conn.refusal_budget)),
        }
    }

    /// Refused before any stored event was read.
    pub(crate) fn deny(&mut self, reason: &'static str) {
        self.outcome = ReadOutcome::Denied(reason);
    }

    /// Ended by a server-side failure.
    pub(crate) fn error(&mut self, reason: &'static str) {
        self.outcome = ReadOutcome::Error(reason);
    }

    /// Ended by a failed database read.
    pub(crate) fn db_failed(&mut self, e: &buzz_db::DbError) {
        self.error(if e.is_statement_cancelled() {
            "query_timed_out"
        } else {
            "database_error"
        });
    }

    /// One stored event was handed to the writer. Counted as it happens, so
    /// a read that later fails or is cancelled still logs its progress.
    pub(crate) fn queued_one(&mut self) {
        self.results += 1;
    }

    /// Completed: every stored event was queued before EOSE.
    pub(crate) fn served(&mut self) {
        self.outcome = ReadOutcome::Served;
    }

    /// A COUNT completed with `total`.
    pub(crate) fn counted(&mut self, total: u64) {
        self.outcome = ReadOutcome::Served;
        self.results = total;
    }
}

impl Drop for ReadLog {
    fn drop(&mut self) {
        if let (ReadOutcome::Denied(_), Some(budget)) = (self.outcome, &self.budget) {
            if !budget.read() {
                return;
            }
        }
        let (outcome, reason) = match self.outcome {
            ReadOutcome::Aborted => ("aborted", None),
            ReadOutcome::Served => ("served", None),
            ReadOutcome::Denied(r) => ("denied", Some(r)),
            ReadOutcome::Error(r) => ("error", Some(r)),
        };
        let s = &self.summary;
        let (events_queued, count) = match (self.event, self.outcome) {
            ("count", ReadOutcome::Served) => (None, Some(self.results)),
            ("count", _) => (None, None),
            _ => (Some(self.results), None),
        };
        emit!(
            Level::INFO,
            event = self.event,
            conn_id = %self.conn_id,
            community = %self.community,
            pubkey = self.pubkey.as_deref(),
            sub_id = self.sub_id.as_str(),
            filters = s.filters,
            kinds = s.kinds.as_deref(),
            kind_count = s.kind_count,
            channel_ids = s.channel_ids.as_deref(),
            channel_count = s.channel_count,
            authors = s.authors.as_deref(),
            author_count = s.author_count,
            p_tags = s.p_tags.as_deref(),
            p_count = s.p_count,
            ids = s.ids,
            since = s.since,
            until = s.until,
            limit = s.limit,
            search = s.search,
            outcome,
            reason,
            events_queued,
            count,
            "access"
        );
    }
}

/// Which credential an auth attempt presented.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum AuthMethod {
    /// WebSocket login (NIP-42).
    Nip42,
    /// HTTP signed request (NIP-98).
    Nip98,
    /// Enterprise identity assertion (NIP-FI).
    NipFi,
    /// Blossom upload authorization (kind 24242).
    Blossom,
    /// Dev-mode `X-Pubkey` header: a claimed key that nothing signed.
    XPubkey,
}

impl AuthMethod {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Nip42 => "nip42",
            Self::Nip98 => "nip98",
            Self::NipFi => "nip_fi",
            Self::Blossom => "blossom",
            Self::XPubkey => "x_pubkey",
        }
    }
}

/// Why a proven identity was refused. One code per meaning on every
/// transport (WebSocket, HTTP, git, audio), so a single filter finds them all.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum AuthReason {
    NotRelayMember,
    MembershipUnavailable,
    Banned,
    BanCheckUnavailable,
    AllowlistDenied,
    AllowlistUnavailable,
    QuotaExceeded,
    AdmissionUnavailable,
    ConcurrencyLimit,
    ReplayCheckUnavailable,
    AuthorizationUnavailable,
    VerifierUnavailable,
    KeyMismatch,
    DenySet,
    SessionExpired,
    RosterUnavailable,
    OriginMismatch,
    AdminHostMismatch,
    NotStaff,
    NotOperator,
    NotOperatorListener,
    NotModerator,
    NipOaOwnerUnlinked,
    ChannelMembershipDenied,
    AlreadyAuthenticated,
    AlreadyFailed,
    Timeout,
    Disconnect,
    Shutdown,
}

impl AuthReason {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::NotRelayMember => "not_relay_member",
            Self::MembershipUnavailable => "membership_unavailable",
            Self::Banned => "banned",
            Self::BanCheckUnavailable => "ban_check_unavailable",
            Self::AllowlistDenied => "allowlist_denied",
            Self::AllowlistUnavailable => "allowlist_unavailable",
            Self::QuotaExceeded => "quota_exceeded",
            Self::AdmissionUnavailable => "admission_unavailable",
            Self::ConcurrencyLimit => "concurrency_limit",
            Self::ReplayCheckUnavailable => "replay_check_unavailable",
            Self::AuthorizationUnavailable => "authorization_unavailable",
            Self::VerifierUnavailable => "verifier_unavailable",
            Self::KeyMismatch => "key_mismatch",
            Self::DenySet => "deny_set",
            Self::SessionExpired => "session_expired",
            Self::RosterUnavailable => "roster_unavailable",
            Self::OriginMismatch => "origin_mismatch",
            Self::AdminHostMismatch => "admin_host_mismatch",
            Self::NotStaff => "not_staff",
            Self::NotOperator => "not_operator",
            Self::NotOperatorListener => "not_operator_listener",
            Self::NotModerator => "not_moderator",
            Self::NipOaOwnerUnlinked => "nip_oa_owner_unlinked",
            Self::ChannelMembershipDenied => "channel_membership_denied",
            Self::AlreadyAuthenticated => "already_authenticated",
            Self::AlreadyFailed => "already_failed",
            Self::Timeout => "timeout",
            Self::Disconnect => "disconnect",
            Self::Shutdown => "shutdown",
        }
    }
}

/// How an auth attempt ended. `Denied` means the identity was proven but not
/// admitted; `Failed` means the proof itself was rejected, with the code of
/// that proof type's single mapper. `Challenged` is a request that carried no
/// credentials and was answered with a challenge, the normal first step of a
/// git exchange. Reasons are fixed codes, never raw error text.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum AuthResult {
    Success,
    Denied(AuthReason),
    Challenged,
    Failed(&'static str),
    /// Cut off before a decision (connection cancelled or session expired).
    Aborted(&'static str),
}

impl AuthResult {
    /// The line's level, `outcome`, and `reason`.
    fn parts(self) -> (Level, &'static str, Option<&'static str>) {
        match self {
            Self::Success => (Level::INFO, "success", None),
            Self::Denied(r) => (Level::WARN, "denied", Some(r.as_str())),
            Self::Challenged => (Level::INFO, "challenged", None),
            Self::Failed(r) => (Level::WARN, "failed", Some(r)),
            Self::Aborted(r) => (Level::WARN, "aborted", Some(r)),
        }
    }

    /// The access result for a WebSocket auth terminal. Only a bad proof is
    /// `Failed`; every policy refusal, including a fail-closed lookup error,
    /// is `Denied` with the metric's stable label as its reason.
    pub(crate) fn from_outcome(outcome: crate::metrics::AuthOutcome) -> Self {
        use crate::metrics::AuthOutcome as O;
        Self::Denied(match outcome {
            O::Success => return Self::Success,
            O::Invalid => return Self::Failed(outcome.as_str()),
            O::Banned => AuthReason::Banned,
            O::BanCheckError => AuthReason::BanCheckUnavailable,
            O::AllowlistCheckError => AuthReason::AllowlistUnavailable,
            O::AllowlistDenied => AuthReason::AllowlistDenied,
            O::RelayMembershipCheckError => AuthReason::MembershipUnavailable,
            O::NotRelayMember => AuthReason::NotRelayMember,
            O::PairingMismatch => AuthReason::KeyMismatch,
            O::Timeout => AuthReason::Timeout,
            O::Disconnect => AuthReason::Disconnect,
            O::Shutdown => AuthReason::Shutdown,
        })
    }
}

/// One auth attempt whose exits are spread across a long handler: it logs
/// exactly once, at [`AuthLine::finish`], or on drop as `aborted` if the
/// handler returned (cancellation, teardown, a request error before the
/// decision) without deciding.
pub(crate) struct AuthLine<'a> {
    attempt: AuthAttempt<'a>,
    abort_reason: &'static str,
    done: bool,
    /// Set for an HTTP request's line ([`AuthLine::for_request`]): the
    /// decision waits for the request's outcome, and both are written as one
    /// line when the line drops.
    request: Option<HeldRequest>,
}

/// A request line's decision, held until the request ends.
struct HeldRequest {
    decision: Option<AuthResult>,
    /// What the request did; `None` until [`AuthLine::complete`].
    fields: Option<RequestFields>,
    summary: Option<FilterSummary>,
}

/// What an admitted HTTP request did, written on the request's access line.
#[derive(Debug, Default)]
pub(crate) struct RequestFields {
    pub(crate) status: Option<u16>,
    pub(crate) result_count: Option<u64>,
    pub(crate) accepted: Option<bool>,
    pub(crate) kind: Option<u32>,
    /// The id of the event a write stored or rejected.
    pub(crate) event_id: Option<String>,
    /// A body that failed to parse: its error category and position.
    pub(crate) category: Option<&'static str>,
    pub(crate) line: Option<usize>,
    pub(crate) column: Option<usize>,
    /// An ingest rejection's fixed code, never its message.
    pub(crate) reject_reason: Option<&'static str>,
    /// A minted invite: its id, expiry, and use limit (absent: unlimited).
    /// Never its code.
    pub(crate) invite_id: Option<Uuid>,
    pub(crate) expires_at: Option<i64>,
    pub(crate) max_uses: Option<i32>,
    /// An invite claim: whether it added the member (`false`: already one).
    pub(crate) joined: Option<bool>,
    /// An admin feedback attachment read.
    pub(crate) feedback_id: Option<Uuid>,
    pub(crate) attachment_sha256: Option<String>,
}

impl RequestFields {
    const EMPTY: Self = Self {
        status: None,
        result_count: None,
        accepted: None,
        kind: None,
        event_id: None,
        category: None,
        line: None,
        column: None,
        reject_reason: None,
        invite_id: None,
        expires_at: None,
        max_uses: None,
        joined: None,
        feedback_id: None,
        attachment_sha256: None,
    };

    /// The response status alone.
    pub(crate) fn status(status: u16) -> Self {
        Self {
            status: Some(status),
            ..Self::default()
        }
    }
}

impl<'a> AuthLine<'a> {
    pub(crate) fn new(attempt: AuthAttempt<'a>) -> Self {
        Self {
            attempt,
            abort_reason: "connection_closed",
            done: false,
            request: None,
        }
    }

    /// This line, written once when the request ends, carrying both the auth
    /// decision and what [`AuthLine::complete`] and [`AuthLine::read`]
    /// recorded about the request.
    pub(crate) fn for_request(mut self) -> Self {
        self.request = Some(HeldRequest {
            decision: None,
            fields: None,
            summary: None,
        });
        self
    }

    /// Record what the request read, for a request line.
    pub(crate) fn read(&mut self, filters: &[Filter]) {
        if let Some(held) = &mut self.request {
            if tracing::enabled!(target: TARGET, Level::INFO) {
                held.summary = Some(FilterSummary::new(filters));
            }
        }
    }

    /// Record how the request ended, for a request line.
    pub(crate) fn complete(&mut self, fields: RequestFields) {
        if let Some(held) = &mut self.request {
            held.fields = Some(fields);
        }
    }

    /// Record that the signature proved `pubkey`.
    pub(crate) fn proven(&mut self, pubkey: PublicKey) {
        self.attempt.pubkey = Some(pubkey);
        self.attempt.pubkey_verified = true;
    }

    /// Bind the line to `community`.
    pub(crate) fn in_community(&mut self, community: CommunityId) {
        self.attempt.community = Some(community);
    }

    /// Replace the undecided attempt, keeping its line.
    pub(crate) fn retarget(&mut self, attempt: AuthAttempt<'a>) {
        self.attempt = attempt;
    }

    /// The `aborted` reason logged if the line is dropped undecided.
    pub(crate) fn abort_reason(&mut self, reason: &'static str) {
        self.abort_reason = reason;
    }

    /// Log the final decision; later calls are no-ops.
    pub(crate) fn finish(&mut self, result: AuthResult) {
        if let Some(held) = &mut self.request {
            held.decision.get_or_insert(result);
        } else if !std::mem::replace(&mut self.done, true) {
            self.attempt.log(result);
        }
    }

    /// Pass `checked` through, finishing the line with `refusal`'s result
    /// when it is an error. An admission step's refusal is final.
    pub(crate) fn gate<T, E>(
        &mut self,
        checked: Result<T, E>,
        refusal: impl FnOnce(&E) -> AuthResult,
    ) -> Result<T, E> {
        if let Err(e) = &checked {
            self.finish(refusal(e));
        }
        checked
    }
}

impl Drop for AuthLine<'_> {
    fn drop(&mut self) {
        let aborted = AuthResult::Aborted(self.abort_reason);
        match self.request.take() {
            Some(held) => self.attempt.log_request(
                held.decision.unwrap_or(aborted),
                held.fields.as_ref(),
                held.summary.as_ref(),
            ),
            None => self.finish(aborted),
        }
    }
}

/// A refusal logged inside a [`refusal_scope`], waiting for the scope to
/// learn the request's route and final status.
struct HeldRefusal {
    method: AuthMethod,
    route: Option<String>,
    pubkey: Option<PublicKey>,
    pubkey_verified: bool,
    result: AuthResult,
    status: u16,
}

thread_local! {
    /// `Some` while a [`refusal_scope`] runs on this thread.
    static REFUSAL_SCOPE: std::cell::RefCell<Option<Option<HeldRefusal>>> =
        const { std::cell::RefCell::new(None) };
}

/// Run `admit`, an HTTP admission whose verifiers log their refusal before
/// the route and the final status are known (Enforce mode can remap a
/// proof's 401 to 403). A refusal logged inside is held and written once
/// when `admit` returns, carrying `route` and, on `Err`, the response's
/// actual status.
#[allow(clippy::result_large_err)] // the admission's own error type
pub(crate) fn refusal_scope<T>(
    route: &str,
    admit: impl FnOnce() -> Result<T, axum::response::Response>,
) -> Result<T, axum::response::Response> {
    let outer = REFUSAL_SCOPE.with(|s| s.borrow_mut().replace(None));
    let result = admit();
    let held = REFUSAL_SCOPE.with(|s| std::mem::replace(&mut *s.borrow_mut(), outer));
    if let Some(Some(held)) = held {
        let status = result
            .as_ref()
            .err()
            .map_or(held.status, |r| r.status().as_u16());
        let route = held.route.as_deref().unwrap_or(route);
        AuthAttempt::http(held.method, route, held.pubkey, held.pubkey_verified).log_request(
            held.result,
            Some(&RequestFields::status(status)),
            None,
        );
    }
    result
}

/// A fixed code for a rejected proof. `Nip98Invalid` and `Internal` carry
/// free text that can echo request data, so they collapse to one code each.
pub(crate) fn auth_error_code(e: &buzz_auth::AuthError) -> &'static str {
    use buzz_auth::AuthError as E;
    match e {
        E::InvalidSignature => "invalid_signature",
        E::ChallengeMismatch => "challenge_mismatch",
        E::RelayUrlMismatch => "relay_url_mismatch",
        E::EventExpired => "event_expired",
        E::Nip98Invalid(_) => "nip98_invalid",
        E::Nip98Replay => "nip98_replay",
        E::PubkeyMismatch => "pubkey_mismatch",
        E::InsufficientScope { .. } => "insufficient_scope",
        E::ChannelAccessDenied => "channel_access_denied",
        E::Internal(_) => "internal",
    }
}

/// One auth attempt. `pubkey_verified` is false when the key is only what
/// the client claimed (a failed signature, or a key never checked).
pub(crate) struct AuthAttempt<'a> {
    method: AuthMethod,
    community: Option<CommunityId>,
    conn_id: Option<Uuid>,
    route: Option<&'a str>,
    pubkey: Option<PublicKey>,
    pubkey_verified: bool,
}

impl AuthAttempt<'_> {
    /// A WebSocket login on `conn` by `pubkey`.
    pub(crate) fn nip42(
        conn: &crate::connection::ConnectionState,
        pubkey: PublicKey,
        pubkey_verified: bool,
    ) -> Self {
        Self {
            method: AuthMethod::Nip42,
            community: Some(conn.tenant.community()),
            conn_id: Some(conn.conn_id),
            route: None,
            pubkey: Some(pubkey),
            pubkey_verified,
        }
    }

    /// An enterprise-assertion admission check. It runs before a community
    /// id is bound, so the line carries none.
    pub(crate) fn nip_fi(pubkey: Option<PublicKey>, pubkey_verified: bool) -> Self {
        Self {
            method: AuthMethod::NipFi,
            community: None,
            conn_id: None,
            route: None,
            pubkey,
            pubkey_verified,
        }
    }

    /// An HTTP signed-request attempt on `route` (a URL path, never a query).
    pub(crate) fn http<'r>(
        method: AuthMethod,
        route: &'r str,
        pubkey: Option<PublicKey>,
        pubkey_verified: bool,
    ) -> AuthAttempt<'r> {
        AuthAttempt {
            method,
            community: None,
            conn_id: None,
            route: Some(url_path(route)),
            pubkey,
            pubkey_verified,
        }
    }

    /// This attempt, bound to `community`.
    pub(crate) fn in_community(self, community: CommunityId) -> Self {
        Self {
            community: Some(community),
            ..self
        }
    }

    /// Finish the WebSocket login with `outcome`, logging it only if this
    /// call won the connection's single auth terminal.
    pub(crate) fn reject(
        &self,
        conn: &crate::connection::ConnectionState,
        outcome: crate::metrics::AuthOutcome,
    ) -> bool {
        let won = conn.reject_auth(outcome);
        if won {
            self.log(AuthResult::from_outcome(outcome));
        }
        won
    }

    /// Log an AUTH frame that arrived after the connection's login was
    /// decided, within the connection's [`RefusalBudget`]. A frame over
    /// budget is only counted, but still settles its frame's line.
    pub(crate) fn log_after_decision(
        &self,
        conn: &crate::connection::ConnectionState,
        result: AuthResult,
    ) {
        if conn.refusal_budget.auth() {
            self.log(result);
        } else {
            crate::connection::note_auth_line_written();
        }
    }

    /// Log a refusal that ends the request with `status`. Inside a
    /// [`refusal_scope`] the line is held so the scope can stamp the route
    /// and the status actually sent.
    pub(crate) fn log_refusal(&self, result: AuthResult, status: u16) {
        let held = REFUSAL_SCOPE.with(|s| {
            let mut scope = s.borrow_mut();
            let slot = scope.as_mut()?;
            slot.get_or_insert(HeldRefusal {
                method: self.method,
                route: self.route.map(str::to_owned),
                pubkey: self.pubkey,
                pubkey_verified: self.pubkey_verified,
                result,
                status,
            });
            Some(())
        });
        if held.is_none() {
            self.log_request(result, Some(&RequestFields::status(status)), None);
        }
    }

    pub(crate) fn log(&self, result: AuthResult) {
        crate::connection::note_auth_line_written();
        let (level, outcome, reason) = result.parts();
        let pubkey = self.pubkey.as_ref().map(PublicKey::to_hex);
        emit!(
            level,
            event = "auth",
            method = self.method.as_str(),
            conn_id = self.conn_id.map(tracing::field::display),
            community = self.community.map(tracing::field::display),
            route = self.route,
            pubkey = pubkey.as_deref(),
            pubkey_verified = pubkey.is_some().then_some(self.pubkey_verified),
            outcome,
            reason,
            "access"
        );
    }

    /// The single line of an HTTP request: its auth decision and, once
    /// admitted, what the request did. An admitted request that never
    /// completed (cancelled, or a route that forgot) logs
    /// `request=aborted` and no status. WARN on a refusal, an error status,
    /// or an aborted request.
    fn log_request(
        &self,
        result: AuthResult,
        req: Option<&RequestFields>,
        s: Option<&FilterSummary>,
    ) {
        let aborted = req.is_none() && matches!(result, AuthResult::Success);
        let (level, outcome, reason) = result.parts();
        let req = req.unwrap_or(&RequestFields::EMPTY);
        let level = if aborted || req.status.is_some_and(|s| s >= 400) {
            Level::WARN
        } else {
            level
        };
        let pubkey = self.pubkey.as_ref().map(PublicKey::to_hex);
        emit!(
            level,
            event = "auth",
            method = self.method.as_str(),
            community = self.community.map(tracing::field::display),
            route = self.route,
            pubkey = pubkey.as_deref(),
            pubkey_verified = pubkey.is_some().then_some(self.pubkey_verified),
            outcome,
            reason,
            request = aborted.then_some("aborted"),
            status = req.status,
            result_count = req.result_count,
            accepted = req.accepted,
            kind = req.kind,
            event_id = req.event_id.as_deref(),
            category = req.category,
            line = req.line,
            column = req.column,
            reject_reason = req.reject_reason,
            invite_id = req.invite_id.map(tracing::field::display),
            expires_at = req.expires_at,
            max_uses = req.max_uses,
            joined = req.joined,
            feedback_id = req.feedback_id.map(tracing::field::display),
            attachment_sha256 = req.attachment_sha256.as_deref(),
            filters = s.map(|s| s.filters),
            kinds = s.and_then(|s| s.kinds.as_deref()),
            kind_count = s.map(|s| s.kind_count),
            channel_ids = s.and_then(|s| s.channel_ids.as_deref()),
            channel_count = s.map(|s| s.channel_count),
            authors = s.and_then(|s| s.authors.as_deref()),
            author_count = s.map(|s| s.author_count),
            p_tags = s.and_then(|s| s.p_tags.as_deref()),
            p_count = s.map(|s| s.p_count),
            ids = s.map(|s| s.ids),
            since = s.and_then(|s| s.since),
            until = s.and_then(|s| s.until),
            limit = s.and_then(|s| s.limit),
            search = s.map(|s| s.search),
            "access"
        );
    }
}

/// Frames the connection's writer accepted: stored events and live pushes
/// alike. A frame counts once its flush succeeds, so a frame the socket
/// rejects, or one stranded by a failed flush, is never counted. This is not
/// client delivery confirmation.
#[derive(Debug, Default)]
pub(crate) struct QueuedTotals {
    events: AtomicU64,
    bytes: AtomicU64,
}

impl QueuedTotals {
    pub(crate) fn events(&self) -> u64 {
        self.events.load(Ordering::Relaxed)
    }

    pub(crate) fn bytes(&self) -> u64 {
        self.bytes.load(Ordering::Relaxed)
    }
}

/// `(events, bytes)` a frame contributes to [`QueuedTotals`].
fn frame_size(frame: &axum::extract::ws::Message) -> (u64, u64) {
    use axum::extract::ws::Message;
    match frame {
        Message::Text(text) => (
            u64::from(text.as_str().starts_with("[\"EVENT\"")),
            text.len() as u64,
        ),
        Message::Binary(bytes) => (0, bytes.len() as u64),
        _ => (0, 0),
    }
}

/// A WebSocket writer that adds each frame to [`QueuedTotals`] once the
/// inner sink accepts it and a later flush succeeds.
pub(crate) struct CountingSink<S> {
    inner: S,
    totals: std::sync::Arc<QueuedTotals>,
    pending: (u64, u64),
}

impl<S> CountingSink<S> {
    pub(crate) fn new(inner: S, totals: std::sync::Arc<QueuedTotals>) -> Self {
        Self {
            inner,
            totals,
            pending: (0, 0),
        }
    }

    fn settle<E>(&mut self, flushed: &Result<(), E>) {
        let (events, bytes) = std::mem::take(&mut self.pending);
        if flushed.is_ok() {
            self.totals.events.fetch_add(events, Ordering::Relaxed);
            self.totals.bytes.fetch_add(bytes, Ordering::Relaxed);
        }
    }
}

impl<S> futures_util::Sink<axum::extract::ws::Message> for CountingSink<S>
where
    S: futures_util::Sink<axum::extract::ws::Message> + Unpin,
{
    type Error = S::Error;

    fn poll_ready(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Result<(), Self::Error>> {
        std::pin::Pin::new(&mut self.inner).poll_ready(cx)
    }

    fn start_send(
        mut self: std::pin::Pin<&mut Self>,
        item: axum::extract::ws::Message,
    ) -> Result<(), Self::Error> {
        let (events, bytes) = frame_size(&item);
        std::pin::Pin::new(&mut self.inner).start_send(item)?;
        self.pending.0 += events;
        self.pending.1 += bytes;
        Ok(())
    }

    fn poll_flush(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Result<(), Self::Error>> {
        let flushed = futures_util::ready!(std::pin::Pin::new(&mut self.inner).poll_flush(cx));
        self.settle(&flushed);
        std::task::Poll::Ready(flushed)
    }

    fn poll_close(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Result<(), Self::Error>> {
        let closed = futures_util::ready!(std::pin::Pin::new(&mut self.inner).poll_close(cx));
        self.settle(&closed);
        std::task::Poll::Ready(closed)
    }
}

/// The connection's one line, written once when the last holder drops it:
/// after a normal close, a refusal during startup, or the handler being
/// dropped. `pubkey` is the identity it logged in as, if any; a successful
/// login writes no line of its own, so its `method` and `pubkey_verified`
/// appear here.
pub(crate) struct CloseLine {
    conn_id: Uuid,
    community: CommunityId,
    peer_addr: SocketAddr,
    opened: std::time::Instant,
    pubkey: std::sync::Mutex<Option<PublicKey>>,
    refused: std::sync::OnceLock<&'static str>,
    pub(crate) queued: std::sync::Arc<QueuedTotals>,
    pub(crate) refusals: std::sync::Arc<RefusalBudget>,
}

impl CloseLine {
    /// The line for a connection upgraded now.
    pub(crate) fn open(
        conn_id: Uuid,
        community: CommunityId,
        peer_addr: SocketAddr,
    ) -> std::sync::Arc<Self> {
        std::sync::Arc::new(Self {
            conn_id,
            community,
            peer_addr,
            opened: std::time::Instant::now(),
            pubkey: std::sync::Mutex::new(None),
            refused: std::sync::OnceLock::new(),
            queued: Default::default(),
            refusals: Default::default(),
        })
    }

    pub(crate) fn conn_id(&self) -> Uuid {
        self.conn_id
    }

    /// Record the identity the connection logged in as.
    pub(crate) fn logged_in(&self, pubkey: PublicKey) {
        *self.pubkey.lock().unwrap_or_else(|e| e.into_inner()) = Some(pubkey);
    }

    /// Record why the relay refused the connection before serving it.
    pub(crate) fn refused(&self, reason: &'static str) {
        let _ = self.refused.set(reason);
    }
}

impl Drop for CloseLine {
    fn drop(&mut self) {
        let pubkey = self
            .pubkey
            .get_mut()
            .unwrap_or_else(|e| e.into_inner())
            .as_ref()
            .map(PublicKey::to_hex);
        emit!(
            Level::INFO,
            event = "close",
            conn_id = %self.conn_id,
            community = %self.community,
            peer_addr = %self.peer_addr,
            refused = self.refused.get().copied(),
            pubkey = pubkey.as_deref(),
            method = pubkey.is_some().then_some(AuthMethod::Nip42.as_str()),
            pubkey_verified = pubkey.is_some().then_some(true),
            duration_ms = u64::try_from(self.opened.elapsed().as_millis()).unwrap_or(u64::MAX),
            events_queued = self.queued.events(),
            bytes_queued = self.queued.bytes(),
            reads_suppressed = RefusalBudget::suppressed(&self.refusals.reads),
            auth_suppressed = RefusalBudget::suppressed(&self.refusals.auths),
            "access"
        );
    }
}

/// The path of a request URL, without scheme, host, or query string. The
/// query is dropped because it can carry client-chosen text.
pub(crate) fn url_path(url: &str) -> &str {
    let after_scheme = url.split_once("://").map_or(url, |(_, rest)| rest);
    let path = after_scheme.find('/').map_or("/", |i| &after_scheme[i..]);
    path.split(['?', '#']).next().unwrap_or(path)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::collections::BTreeSet;
    use std::sync::{Arc, Mutex};
    use tracing_subscriber::prelude::*;

    #[derive(Clone, Default)]
    pub(crate) struct Captured(Arc<Mutex<Vec<u8>>>);

    impl std::io::Write for Captured {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    impl Captured {
        /// Everything written so far, on every target.
        pub(crate) fn raw(&self) -> String {
            String::from_utf8(self.0.lock().unwrap().clone()).unwrap()
        }

        /// Access lines written so far, parsed from the production JSON shape.
        pub(crate) fn lines(&self) -> Vec<serde_json::Value> {
            String::from_utf8(self.0.lock().unwrap().clone())
                .unwrap()
                .lines()
                .map(|l| serde_json::from_str(l).unwrap())
                .filter(|v: &serde_json::Value| v["target"] == TARGET)
                .collect()
        }
    }

    fn json_subscriber(out: &Captured) -> impl tracing::Subscriber + Send + Sync {
        let writer = out.clone();
        tracing_subscriber::registry().with(
            tracing_subscriber::fmt::layer()
                .json()
                .flatten_event(true)
                .with_writer(move || writer.clone()),
        )
    }

    /// Run `f` with JSON logs (flattened, as in production) captured.
    pub(crate) fn capture<R>(f: impl FnOnce() -> R) -> (R, Captured) {
        let _lock = crate::test_support::tracing_dispatch_lock();
        let out = Captured::default();
        let r = tracing::subscriber::with_default(json_subscriber(&out), f);
        (r, out)
    }

    /// Capture JSON logs on this thread until the guard drops, for async
    /// tests on a current-thread runtime that `.await` the code under test
    /// directly (spawned tasks on other threads are not captured).
    pub(crate) fn capture_scoped() -> (tracing::subscriber::DefaultGuard, Captured) {
        let out = Captured::default();
        (tracing::subscriber::set_default(json_subscriber(&out)), out)
    }

    /// A replay guard whose check never finishes, so a request parks in its
    /// replay wait; `arrived` fires once a request reaches it.
    #[derive(Default)]
    pub(crate) struct StallingReplayGuard {
        pub(crate) arrived: tokio::sync::Notify,
    }

    impl buzz_auth::Nip98ReplayGuard for StallingReplayGuard {
        fn try_mark_in_scope<'a>(
            &'a self,
            _scope: &'a str,
            _event_id: &'a nostr::EventId,
            _ttl_secs: u64,
        ) -> std::pin::Pin<
            Box<dyn std::future::Future<Output = Result<bool, buzz_auth::AuthError>> + Send + 'a>,
        > {
            self.arrived.notify_one();
            Box::pin(std::future::pending())
        }
    }

    /// Assert `out` holds exactly one auth line, the `aborted` line of a
    /// request by `pubkey` dropped before its decision, and return it.
    pub(crate) fn assert_one_aborted_auth_line(
        out: &Captured,
        pubkey: &PublicKey,
    ) -> serde_json::Value {
        let lines: Vec<_> = out
            .lines()
            .into_iter()
            .filter(|l| l["event"] == "auth")
            .collect();
        assert_eq!(lines.len(), 1, "exactly one auth line: {lines:?}");
        let line = lines[0].clone();
        assert_eq!(line["outcome"], "aborted", "{line}");
        assert_eq!(line["reason"], "request_ended", "{line}");
        assert_eq!(line["pubkey"], pubkey.to_hex(), "{line}");
        assert_eq!(line["pubkey_verified"], true, "{line}");
        line
    }

    fn filter(json: serde_json::Value) -> Filter {
        serde_json::from_value(json).unwrap()
    }

    const PK_A: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

    fn pk() -> String {
        nostr::Keys::generate().public_key().to_hex()
    }

    /// A request line admitted but never completed (its future dropped
    /// mid-request, or a route that forgot `complete`) keeps its login and
    /// says the request aborted, with no status.
    #[test]
    fn admitted_request_never_completed_logs_aborted() {
        let key = nostr::Keys::generate().public_key();
        let (_, out) = capture(|| {
            let mut line = AuthLine::new(AuthAttempt::http(
                AuthMethod::Nip98,
                "/query",
                Some(key),
                true,
            ))
            .for_request();
            line.finish(AuthResult::Success);
        });
        let lines = out.lines();
        assert_eq!(lines.len(), 1, "{lines:?}");
        let line = &lines[0];
        assert_eq!(line["outcome"], "success", "{line}");
        assert_eq!(line["pubkey"], key.to_hex(), "{line}");
        assert_eq!(line["request"], "aborted", "{line}");
        assert_eq!(line["level"], "WARN", "{line}");
        assert!(line.get("status").is_none(), "{line}");
    }

    /// The completed counterpart carries its status and no `request` field.
    #[test]
    fn admitted_request_completed_logs_no_abort() {
        let (_, out) = capture(|| {
            let mut line =
                AuthLine::new(AuthAttempt::http(AuthMethod::Nip98, "/query", None, false))
                    .for_request();
            line.finish(AuthResult::Success);
            line.complete(RequestFields::status(200));
        });
        let line = &out.lines()[0];
        assert!(line.get("request").is_none(), "{line}");
        assert_eq!(line["status"], 200, "{line}");
    }

    #[test]
    fn summary_merges_filters_to_the_widest_bounds() {
        let s = FilterSummary::new(&[
            filter(serde_json::json!({"kinds": [9, 7], "since": 100, "until": 200, "limit": 10})),
            filter(serde_json::json!({"kinds": [7, 5], "since": 50, "until": 300, "limit": 50})),
        ]);
        assert_eq!(s.filters, 2);
        assert_eq!((s.kinds.as_deref(), s.kind_count), (Some("7,9,5"), 4));
        assert_eq!((s.since, s.until, s.limit), (Some(50), Some(300), Some(50)));
        assert!(!s.search);
    }

    #[test]
    fn summary_omits_a_bound_any_filter_leaves_open() {
        let s = FilterSummary::new(&[
            filter(serde_json::json!({"since": 100, "limit": 10})),
            filter(serde_json::json!({"until": 300})),
        ]);
        assert_eq!((s.since, s.until, s.limit), (None, None, None));
    }

    #[test]
    fn summary_caps_lists_and_counts_every_value() {
        let channels: Vec<String> = (0..14).map(|_| Uuid::new_v4().to_string()).collect();
        let ps: Vec<String> = (0..12).map(|_| pk()).collect();
        let s = FilterSummary::new(&[filter(serde_json::json!({
            "#h": channels, "#p": ps, "authors": [PK_A], "ids": [PK_A]
        }))]);
        assert_eq!(s.channel_count, 14);
        assert_eq!(s.channel_ids.unwrap().split(',').count(), LIST_CAP);
        assert_eq!(s.p_count, 12);
        assert_eq!(s.p_tags.unwrap().split(',').count(), LIST_CAP);
        assert_eq!(
            (s.authors.as_deref(), s.author_count, s.ids),
            (Some(PK_A), 1, 1)
        );
    }

    #[test]
    fn summary_of_long_lists_lists_ten_distinct_and_sums_counts() {
        let authors: Vec<String> = (0..5_000).map(|_| pk()).collect();
        let shared = authors[0].clone();
        let s = FilterSummary::new(&[
            filter(serde_json::json!({"authors": authors})),
            filter(serde_json::json!({"authors": [shared]})),
        ]);
        // Cross-filter duplicates count twice.
        assert_eq!(s.author_count, 5_001);
        let listed: Vec<&str> = s.authors.as_deref().unwrap().split(',').collect();
        assert_eq!(listed.len(), LIST_CAP);
        assert_eq!(listed.iter().collect::<BTreeSet<_>>().len(), LIST_CAP);
    }

    #[test]
    fn summary_caps_kinds_like_the_identifier_lists() {
        let kinds: Vec<u16> = (0..=u16::MAX).collect();
        let s = FilterSummary::new(&[filter(serde_json::json!({"kinds": kinds}))]);
        assert_eq!(s.kinds.unwrap().split(',').count(), LIST_CAP);
        assert_eq!(s.kind_count, 65_536);
    }

    #[test]
    fn summary_marks_a_dimension_any_filter_leaves_open() {
        let s = FilterSummary::new(&[
            filter(
                serde_json::json!({"kinds": [9], "authors": [PK_A], "#h": [Uuid::nil().to_string()]}),
            ),
            filter(serde_json::json!({"#p": [PK_A]})),
        ]);
        assert_eq!(s.kinds.as_deref(), Some("*"));
        assert_eq!(s.authors.as_deref(), Some("*"));
        assert_eq!(s.channel_ids.as_deref(), Some("*"));
        assert_eq!(s.p_tags.as_deref(), Some("*"));
        assert_eq!(
            (s.kind_count, s.author_count, s.channel_count, s.p_count),
            (1, 1, 1, 1)
        );
    }

    #[test]
    fn refusal_budget_logs_sixteen_then_counts_the_rest() {
        let budget = RefusalBudget::default();
        let logged = (0..20).filter(|_| budget.read()).count();
        assert_eq!(logged, 16);
        assert_eq!(RefusalBudget::suppressed(&budget.reads), 4);
        assert_eq!(RefusalBudget::suppressed(&budget.auths), 0);
    }

    #[test]
    fn summary_never_lists_malformed_tag_text() {
        let s = FilterSummary::new(&[filter(serde_json::json!({
            "#h": ["secret plans for tuesday"], "#p": ["not a pubkey"]
        }))]);
        assert_eq!((s.channel_ids, s.channel_count), (None, 1));
        assert_eq!((s.p_tags, s.p_count), (None, 1));
    }

    #[test]
    fn req_line_records_search_use_but_never_the_search_text() {
        let needle = "zebra-crossing-7f3a";
        let filters = [filter(serde_json::json!({"kinds": [9], "search": needle}))];
        let ((), out) = capture(|| {
            let mut log = ReadLog {
                event: "req",
                conn_id: Uuid::nil(),
                community: CommunityId::from_uuid(Uuid::nil()),
                pubkey: Some(PK_A.to_owned()),
                sub_id: "sub".to_owned(),
                summary: FilterSummary::new(&filters),
                outcome: ReadOutcome::Aborted,
                results: 0,
                budget: None,
            };
            for _ in 0..3 {
                log.queued_one();
            }
            log.served();
        });
        let raw = String::from_utf8(out.0.lock().unwrap().clone()).unwrap();
        assert!(!raw.contains(needle), "search text leaked: {raw}");
        let lines = out.lines();
        assert_eq!(lines.len(), 1);
        let line = &lines[0];
        assert_eq!(line["event"], "req");
        assert_eq!(line["search"], true);
        assert_eq!(line["outcome"], "served");
        assert_eq!(line["events_queued"], 3);
        assert_eq!(line["pubkey"], PK_A);
        assert_eq!(line["kinds"], "9");
        assert!(line.get("since").is_none() && line.get("reason").is_none());
        assert!(line.get("count").is_none());
    }

    #[test]
    fn dropped_read_logs_as_aborted() {
        let ((), out) = capture(|| {
            let _log = ReadLog {
                event: "count",
                conn_id: Uuid::nil(),
                community: CommunityId::from_uuid(Uuid::nil()),
                pubkey: None,
                sub_id: "c".to_owned(),
                summary: FilterSummary::default(),
                outcome: ReadOutcome::Aborted,
                results: 0,
                budget: None,
            };
        });
        let line = &out.lines()[0];
        assert_eq!(
            (&line["event"], &line["outcome"]),
            (&"count".into(), &"aborted".into())
        );
        assert!(
            line.get("count").is_none(),
            "an uncounted COUNT has no count"
        );
        assert!(line.get("pubkey").is_none());
    }

    #[test]
    fn auth_failure_is_warn_and_marks_the_claimed_key_unverified() {
        let key = nostr::Keys::generate().public_key();
        let ((), out) = capture(|| {
            AuthAttempt {
                method: AuthMethod::Nip98,
                community: None,
                conn_id: None,
                route: Some("/query"),
                pubkey: Some(key),
                pubkey_verified: false,
            }
            .log(AuthResult::Failed("invalid_signature"));
        });
        let line = &out.lines()[0];
        assert_eq!(line["level"], "WARN");
        assert_eq!(line["event"], "auth");
        assert_eq!(line["method"], "nip98");
        assert_eq!(line["outcome"], "failed");
        assert_eq!(line["reason"], "invalid_signature");
        assert_eq!(line["pubkey"], key.to_hex());
        assert_eq!(line["pubkey_verified"], false);
        assert_eq!(line["route"], "/query");
    }

    #[test]
    fn auth_success_is_info_without_a_reason() {
        let ((), out) = capture(|| {
            AuthAttempt {
                method: AuthMethod::Nip42,
                community: Some(CommunityId::from_uuid(Uuid::nil())),
                conn_id: Some(Uuid::nil()),
                route: None,
                pubkey: Some(nostr::Keys::generate().public_key()),
                pubkey_verified: true,
            }
            .log(AuthResult::Success);
        });
        let line = &out.lines()[0];
        assert_eq!(
            (&line["level"], &line["outcome"]),
            (&"INFO".into(), &"success".into())
        );
        assert_eq!(line["pubkey_verified"], true);
        assert!(line.get("reason").is_none() && line.get("route").is_none());
    }

    #[test]
    fn auth_line_logs_once_and_aborts_when_dropped_undecided() {
        let attempt = || AuthAttempt::http(AuthMethod::Nip42, "audio", None, false);
        let ((), out) = capture(|| {
            let mut line = AuthLine::new(attempt());
            line.finish(AuthResult::Denied(crate::access::AuthReason::DenySet));
            line.finish(AuthResult::Success);
            drop(AuthLine::new(attempt()));
        });
        let outcomes: Vec<_> = out.lines().iter().map(|l| l["outcome"].clone()).collect();
        assert_eq!(outcomes, ["denied", "aborted"]);
    }

    #[test]
    fn url_path_drops_host_and_query() {
        assert_eq!(url_path("https://relay.example/query?search=hi"), "/query");
        assert_eq!(url_path("https://relay.example"), "/");
        assert_eq!(url_path("/events#x"), "/events");
    }
}
