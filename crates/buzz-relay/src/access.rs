//! Stdout access log for security investigations: who read what, and who
//! tried to authenticate.
//!
//! Every line goes to the [`TARGET`] tracing target (stdout, never the
//! `audit_log` table) and carries an `event` field to filter on. Search text,
//! event content, tokens, and assertions are never recorded: filters are
//! reduced to a [`FilterSummary`], and failures to fixed reason codes.
//!
//! `event=auth` lines cover WebSocket AUTH (root and audio): one line per
//! attempt; root lines carry the TCP `peer_addr`. `event=http` lines come from
//! [`http_access_log`]: one per HTTP request, with what the handler proved and
//! decided recorded through its [`AccessSlot`]. Every matched route is logged
//! except public documents and probes; a request that matched no route is
//! logged only at status 400 or above, as `route="<unmatched>"`. `route` is the
//! matched template, never the raw path.
//! A successful bridge read that returned nothing (`result_count=0`) logs at
//! DEBUG: polling clients make most of them. WebSocket `req` lines stay at
//! INFO, because an empty subscription can still deliver live events.
//!
//! With `BUZZ_CLIENT_IP_HEADER` set, `http`, `auth` and `close` lines also
//! carry `client_ip`: the rightmost entry of that request header, only when it
//! parses as an IP address. It never replaces `peer_addr`.

use std::net::{IpAddr, SocketAddr};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use buzz_core::CommunityId;
use nostr::{Alphabet, Filter, PublicKey, SingleLetterTag};
use tracing::Level;
use uuid::Uuid;

/// The tracing target for every access line.
pub(crate) const TARGET: &str = "buzz_relay::access";

/// Most identifiers listed per field; a longer list adds its `*_count`.
const LIST_CAP: usize = 10;

macro_rules! emit {
    ($level:expr, $($fields:tt)+) => {
        match $level {
            Level::WARN => tracing::warn!(target: TARGET, $($fields)+),
            Level::INFO => tracing::info!(target: TARGET, $($fields)+),
            _ => tracing::debug!(target: TARGET, $($fields)+),
        }
    };
}

/// What a REQ or COUNT asked for, merged across its filters. List fields hold
/// the first [`LIST_CAP`] distinct values; their `*_count` fields sum the
/// per-filter set sizes, so a value repeated in two filters counts twice, and
/// are absent when the list already shows that many values. A list field is
/// `*` when any filter leaves that dimension open, because that filter reads
/// every value; `since`/`until`/`limit` likewise take the widest
/// bound and are absent when any filter leaves them open. No field understates
/// what the read covered. Only well-formed channel ids and pubkeys are listed,
/// so client-chosen tag text never reaches the log.
#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct FilterSummary {
    filters: usize,
    kinds: Option<String>,
    kind_count: Option<usize>,
    channel_ids: Option<String>,
    channel_count: Option<usize>,
    authors: Option<String>,
    author_count: Option<usize>,
    p_tags: Option<String>,
    p_count: Option<usize>,
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

    /// The count, unless the list shows exactly that many values, and the
    /// comma-joined list: [`OPEN`] when a filter leaves the dimension open,
    /// `None` when nothing was listed.
    fn finish(self, open: bool, show: impl Fn(&T) -> String) -> (Option<usize>, Option<String>) {
        let count = (open || self.seen != self.listed.len()).then_some(self.seen);
        let shown = if open {
            Some(OPEN.to_owned())
        } else {
            (!self.listed.is_empty())
                .then(|| self.listed.iter().map(show).collect::<Vec<_>>().join(","))
        };
        (count, shown)
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
        Self::charge(&self.reads)
    }

    /// Whether this post-decision AUTH frame may write its line.
    pub(crate) fn auth(&self) -> bool {
        Self::charge(&self.auths)
    }

    /// A charge logs while under budget, or once `close` has been written.
    fn charge(seen: &AtomicU64) -> bool {
        let prev = seen.fetch_add(1, Ordering::AcqRel);
        !(REFUSAL_LINES..CLOSED).contains(&prev)
    }

    /// Ends counting for `close` and returns how many were suppressed.
    /// Swapping in [`CLOSED`] and every charge's `fetch_add` are
    /// read-modify-writes on one atomic, so each charge lands either before
    /// the swap (in `seen`) or after it (sees `CLOSED` and logs its own
    /// line): none is both counted and logged, and none is lost.
    fn close(seen: &AtomicU64) -> u64 {
        seen.swap(CLOSED, Ordering::AcqRel)
            .saturating_sub(REFUSAL_LINES)
    }
}

/// Counter value once `close` has been written.
const CLOSED: u64 = 1 << 63;

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
            // An `error` line is WARN, so it keeps the summary at `=warn`.
            summary: if tracing::enabled!(target: TARGET, Level::WARN) {
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
        // A client closing mid-read (`aborted`) or a refusal is routine; only
        // a server-side failure is a WARN.
        let level = match self.outcome {
            ReadOutcome::Error(_) => Level::WARN,
            _ => Level::INFO,
        };
        emit!(
            level,
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
    /// Admitted by network position alone (`BUZZ_ADMIN_AUTH=disabled`):
    /// no proof and no key.
    NetworkTrusted,
}

impl AuthMethod {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Nip42 => "nip42",
            Self::Nip98 => "nip98",
            Self::NipFi => "nip_fi",
            Self::Blossom => "blossom",
            Self::XPubkey => "x_pubkey",
            Self::NetworkTrusted => "network_trusted",
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
    OwnerMismatch,
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
            Self::OwnerMismatch => "owner_mismatch",
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

/// One WebSocket auth attempt (NIP-42 or audio) whose exits are spread
/// across a long handler: it logs exactly once, at [`AuthLine::finish`], or
/// on drop as `aborted` if the handler returned (cancellation, teardown)
/// without deciding. HTTP requests use [`AccessSlot`] instead.
pub(crate) struct AuthLine<'a> {
    attempt: AuthAttempt<'a>,
    abort_reason: &'static str,
    done: bool,
}

impl<'a> AuthLine<'a> {
    pub(crate) fn new(attempt: AuthAttempt<'a>) -> Self {
        Self {
            attempt,
            abort_reason: "connection_closed",
            done: false,
        }
    }

    /// Record that the signature proved `pubkey`.
    pub(crate) fn proven(&mut self, pubkey: PublicKey) {
        self.attempt.pubkey = Some(pubkey);
        self.attempt.pubkey_verified = true;
    }

    /// The `aborted` reason logged if the line is dropped undecided.
    pub(crate) fn abort_reason(&mut self, reason: &'static str) {
        self.abort_reason = reason;
    }

    /// Log the final decision; later calls are no-ops.
    pub(crate) fn finish(&mut self, result: AuthResult) {
        if !std::mem::replace(&mut self.done, true) {
            self.attempt.log(result);
        }
    }
}

impl Drop for AuthLine<'_> {
    fn drop(&mut self) {
        self.finish(AuthResult::Aborted(self.abort_reason));
    }
}

/// The client IP in `header`, when configured: the rightmost entry of the
/// field's comma-separated list, across repeated fields (the one the
/// operator's proxy appended), and only if it parses as an IP address.
pub(crate) fn client_ip(
    headers: &axum::http::HeaderMap,
    header: Option<&axum::http::HeaderName>,
) -> Option<IpAddr> {
    let value = headers.get_all(header?).iter().next_back()?.to_str().ok()?;
    value.rsplit(',').next()?.trim().parse().ok()
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
    peer_addr: Option<SocketAddr>,
    client_ip: Option<IpAddr>,
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
            peer_addr: Some(conn.remote_addr),
            client_ip: conn.community_control.client_ip(),
            pubkey: Some(pubkey),
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
            route: Some(route),
            peer_addr: None,
            client_ip: None,
            pubkey,
            pubkey_verified,
        }
    }

    /// This attempt, from a request whose configured header gave `client_ip`.
    pub(crate) fn with_client_ip(self, client_ip: Option<IpAddr>) -> Self {
        Self { client_ip, ..self }
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
            peer_addr = self.peer_addr.map(tracing::field::display),
            client_ip = self.client_ip.map(tracing::field::display),
            pubkey = pubkey.as_deref(),
            pubkey_verified = pubkey.is_some().then_some(self.pubkey_verified),
            outcome,
            reason,
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
/// dropped. `pubkey` is the identity it logged in as, if any, with its
/// `method` and `pubkey_verified`; the login also wrote its own `auth` line.
pub(crate) struct CloseLine {
    conn_id: Uuid,
    community: CommunityId,
    peer_addr: SocketAddr,
    client_ip: Option<IpAddr>,
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
        client_ip: Option<IpAddr>,
    ) -> std::sync::Arc<Self> {
        std::sync::Arc::new(Self {
            conn_id,
            community,
            peer_addr,
            client_ip,
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
            client_ip = self.client_ip.map(tracing::field::display),
            refused = self.refused.get().copied(),
            pubkey = pubkey.as_deref(),
            method = pubkey.is_some().then_some(AuthMethod::Nip42.as_str()),
            pubkey_verified = pubkey.is_some().then_some(true),
            duration_ms = u64::try_from(self.opened.elapsed().as_millis()).unwrap_or(u64::MAX),
            events_queued = self.queued.events(),
            bytes_queued = self.queued.bytes(),
            reads_suppressed = RefusalBudget::close(&self.refusals.reads),
            auth_suppressed = RefusalBudget::close(&self.refusals.auths),
            "access"
        );
    }
}

/// The `event` value of every HTTP request line.
pub(crate) const HTTP_EVENT: &str = "http";

/// The `route` of a logged request that matched no route.
const UNMATCHED_ROUTE: &str = "<unmatched>";

/// Matched routes that never get a line: public documents and probes.
/// `/` is listed only for requests that are not WebSocket upgrades.
const EXCLUDED_ROUTES: [&str; 5] = [
    "/info",
    "/.well-known/nostr.json",
    "/health",
    "/_liveness",
    "/_readiness",
];

/// What one HTTP request proved and what was decided about it, written by
/// handlers through [`AccessSlot`] and logged once by [`http_access_log`].
#[derive(Default)]
struct AccessNote {
    method: Option<AuthMethod>,
    community: Option<CommunityId>,
    pubkey: Option<PublicKey>,
    pubkey_verified: bool,
    decision: Option<AuthResult>,
    summary: Option<FilterSummary>,
    detail: Detail,
}

/// What a route did, beyond its auth decision. Each route sets only its own
/// fields; invite codes, tokens and Blossom hashes have no field.
#[derive(Debug, Default)]
pub(crate) struct Detail {
    /// A bridge read: events returned, or the count a `/count` answered.
    pub(crate) result_count: Option<u64>,
    /// A bridge write: whether it was stored, and the event's kind and id.
    pub(crate) accepted: Option<bool>,
    pub(crate) kind: Option<u32>,
    pub(crate) event_id: Option<String>,
    /// A bridge body that failed to parse: its error category and position.
    pub(crate) category: Option<&'static str>,
    pub(crate) line: Option<usize>,
    pub(crate) column: Option<usize>,
    /// An ingest rejection's fixed code, never its message.
    pub(crate) reject_reason: Option<&'static str>,
    /// A minted invite: its id, expiry, and use limit (absent: unlimited).
    pub(crate) invite_id: Option<Uuid>,
    pub(crate) expires_at: Option<i64>,
    pub(crate) max_uses: Option<i32>,
    /// An invite claim: whether it added the member (`false`: already one).
    pub(crate) joined: Option<bool>,
    /// An admin feedback attachment read.
    pub(crate) feedback_id: Option<Uuid>,
    pub(crate) attachment_sha256: Option<String>,
}

/// The access note of one HTTP request. [`http_access_log`] puts it in the
/// request's extensions; handlers take it as an extractor (a detached note
/// when the layer is absent, as in handler unit tests) and pass `&AccessSlot`
/// to their helpers. A refusal is final: a later admission leaves it, and
/// a later refusal replaces an admission.
#[derive(Clone, Default)]
pub struct AccessSlot(Arc<std::sync::Mutex<AccessNote>>);

impl AccessSlot {
    fn note(&self) -> std::sync::MutexGuard<'_, AccessNote> {
        self.0.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// The credential presented and the key it claims. A proven key is
    /// never replaced by a claimed one.
    pub(crate) fn attempt(&self, method: AuthMethod, pubkey: Option<PublicKey>, verified: bool) {
        let mut note = self.note();
        note.method = Some(method);
        if verified || !note.pubkey_verified {
            note.pubkey = pubkey.or(note.pubkey);
            note.pubkey_verified = verified;
        }
    }

    /// The credential presented, keeping any key already recorded.
    pub(crate) fn method(&self, method: AuthMethod) {
        self.note().method = Some(method);
    }

    pub(crate) fn in_community(&self, community: CommunityId) {
        self.note().community = Some(community);
    }

    /// The note of the request `parts` belongs to.
    pub(crate) fn of(parts: &axum::http::request::Parts) -> Self {
        parts.extensions.get::<Self>().cloned().unwrap_or_default()
    }

    /// Record the decision. A refusal is final; a later refusal replaces an
    /// earlier admission, never the reverse.
    pub(crate) fn finish(&self, result: AuthResult) {
        let mut note = self.note();
        if matches!(note.decision, None | Some(AuthResult::Success)) {
            note.decision = Some(result);
        }
    }

    /// Pass `checked` through, finishing with `refusal`'s result when it is
    /// an error.
    pub(crate) fn gate<T, E>(
        &self,
        checked: Result<T, E>,
        refusal: impl FnOnce(&E) -> AuthResult,
    ) -> Result<T, E> {
        if let Err(e) = &checked {
            self.finish(refusal(e));
        }
        checked
    }

    /// Record the filters the request reads. Built at WARN, not INFO,
    /// because a denied, failed or aborted request logs its line at WARN.
    pub(crate) fn read(&self, filters: &[Filter]) {
        if tracing::enabled!(target: TARGET, Level::WARN) {
            self.note().summary = Some(FilterSummary::new(filters));
        }
    }

    pub(crate) fn detail(&self, detail: Detail) {
        self.note().detail = detail;
    }

    /// Write this note's line as the layer would for `route` answered with
    /// `status`.
    #[cfg(test)]
    pub(crate) fn emit(&self, route: &str, status: u16) {
        drop(HttpLine {
            slot: self.clone(),
            route: Some(route.to_owned()),
            http_method: axum::http::Method::POST,
            peer_addr: None,
            client_ip: None,
            started: std::time::Instant::now(),
            status: Some(axum::http::StatusCode::from_u16(status).unwrap()),
        });
    }

    /// Write this note's line as the layer would for a `route` request
    /// dropped before its response was ready.
    #[cfg(test)]
    pub(crate) fn abandon(&self, route: &str) {
        drop(HttpLine {
            slot: self.clone(),
            route: Some(route.to_owned()),
            http_method: axum::http::Method::GET,
            peer_addr: None,
            client_ip: None,
            started: std::time::Instant::now(),
            status: None,
        });
    }
}

impl<S: Send + Sync> axum::extract::FromRequestParts<S> for AccessSlot {
    type Rejection = std::convert::Infallible;

    async fn from_request_parts(
        parts: &mut axum::http::request::Parts,
        _: &S,
    ) -> Result<Self, Self::Rejection> {
        Ok(Self::of(parts))
    }
}

/// Middleware: one access line per HTTP request, written when the response
/// is ready, or as `request=aborted` if the request is dropped first. Public
/// documents, probes, SPA pages and static files are skipped; a request
/// that matched no route is logged only at status 400 or above.
pub(crate) async fn http_access_log(
    axum::extract::State(state): axum::extract::State<Arc<crate::state::AppState>>,
    mut req: axum::extract::Request,
    next: axum::middleware::Next,
) -> axum::response::Response {
    let route = match req.extensions().get::<axum::extract::MatchedPath>() {
        Some(m) if excluded_route(m.as_str(), &req) => return next.run(req).await,
        Some(m) => Some(m.as_str().to_owned()),
        None if crate::router::is_document_request(&state, &req) => return next.run(req).await,
        None => None,
    };
    let slot = AccessSlot::default();
    req.extensions_mut().insert(slot.clone());
    let mut line = HttpLine {
        slot,
        route,
        http_method: req.method().clone(),
        peer_addr: req
            .extensions()
            .get::<axum::extract::ConnectInfo<SocketAddr>>()
            .map(|c| c.0),
        client_ip: client_ip(req.headers(), state.config.client_ip_header.as_ref()),
        started: std::time::Instant::now(),
        status: None,
    };
    let response = next.run(req).await;
    line.status = Some(response.status());
    response
}

/// Whether a matched route is never logged.
fn excluded_route(route: &str, req: &axum::extract::Request) -> bool {
    let upgrade = req.headers().contains_key(axum::http::header::UPGRADE)
        || req.method() == axum::http::Method::CONNECT;
    EXCLUDED_ROUTES.contains(&route) || (route == "/" && !upgrade)
}

/// A request's pending line, written on drop.
struct HttpLine {
    slot: AccessSlot,
    /// The matched template; `None` for a request that matched no route.
    route: Option<String>,
    http_method: axum::http::Method,
    peer_addr: Option<SocketAddr>,
    client_ip: Option<IpAddr>,
    started: std::time::Instant,
    /// `None` until the response is ready.
    status: Option<axum::http::StatusCode>,
}

impl Drop for HttpLine {
    fn drop(&mut self) {
        let status = self.status.map(|s| s.as_u16());
        let skip = match (&self.route, status) {
            // A successful upgrade: the connection's own lines cover it.
            (_, Some(101)) => true,
            (None, Some(s)) => s < 400,
            (None, None) => true,
            (Some(_), _) => false,
        };
        if skip {
            return;
        }
        let note = self.slot.note();
        let aborted = status.is_none();
        let (level, outcome, reason) = note.decision.map_or((Level::INFO, None, None), |d| {
            let (level, outcome, reason) = d.parts();
            (level, Some(outcome), reason)
        });
        // A challenge is git's normal first step, so its 401 stays INFO.
        let challenged = matches!(note.decision, Some(AuthResult::Challenged));
        let level = if aborted || (!challenged && status.is_some_and(|s| s >= 400)) {
            Level::WARN
        } else {
            level
        };
        // A served read that returned nothing is routine polling.
        let level = if level == Level::INFO && note.detail.result_count == Some(0) {
            Level::DEBUG
        } else {
            level
        };
        let pubkey = note.pubkey.as_ref().map(PublicKey::to_hex);
        let s = note.summary.as_ref();
        let d = &note.detail;
        emit!(
            level,
            event = HTTP_EVENT,
            route = self.route.as_deref().unwrap_or(UNMATCHED_ROUTE),
            http_method = %self.http_method,
            peer_addr = self.peer_addr.map(tracing::field::display),
            client_ip = self.client_ip.map(tracing::field::display),
            method = note.method.map(AuthMethod::as_str),
            community = note.community.map(tracing::field::display),
            pubkey = pubkey.as_deref(),
            pubkey_verified = pubkey.is_some().then_some(note.pubkey_verified),
            outcome = outcome.unwrap_or("undecided"),
            reason,
            request = aborted.then_some("aborted"),
            status,
            time_to_response_ms =
                u64::try_from(self.started.elapsed().as_millis()).unwrap_or(u64::MAX),
            result_count = d.result_count,
            accepted = d.accepted,
            kind = d.kind,
            event_id = d.event_id.as_deref(),
            category = d.category,
            line = d.line,
            column = d.column,
            reject_reason = d.reject_reason,
            invite_id = d.invite_id.map(tracing::field::display),
            expires_at = d.expires_at,
            max_uses = d.max_uses,
            joined = d.joined,
            feedback_id = d.feedback_id.map(tracing::field::display),
            attachment_sha256 = d.attachment_sha256.as_deref(),
            filters = s.map(|s| s.filters),
            kinds = s.and_then(|s| s.kinds.as_deref()),
            kind_count = s.and_then(|s| s.kind_count),
            channel_ids = s.and_then(|s| s.channel_ids.as_deref()),
            channel_count = s.and_then(|s| s.channel_count),
            authors = s.and_then(|s| s.authors.as_deref()),
            author_count = s.and_then(|s| s.author_count),
            p_tags = s.and_then(|s| s.p_tags.as_deref()),
            p_count = s.and_then(|s| s.p_count),
            ids = s.map(|s| s.ids),
            since = s.and_then(|s| s.since),
            until = s.and_then(|s| s.until),
            limit = s.and_then(|s| s.limit),
            search = s.map(|s| s.search),
            "access"
        );
    }
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
                .event_format(crate::telemetry::TraceContextLookup::default().json_formatter(false))
                .with_writer(move || writer.clone()),
        )
    }

    /// Like [`capture`], with the access target filtered to WARN.
    pub(crate) fn capture_warn<R>(f: impl FnOnce() -> R) -> (R, Captured) {
        let _lock = crate::test_support::tracing_dispatch_lock();
        let out = Captured::default();
        let subscriber = json_subscriber(&out)
            .with(tracing_subscriber::filter::Targets::new().with_target(TARGET, Level::WARN));
        let r = tracing::subscriber::with_default(subscriber, f);
        (r, out)
    }

    /// Run `f` with JSON logs captured, using production's JSON formatter.
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

    /// Like [`capture_scoped`], with only the access target on, at `level`:
    /// the HTTP span (target `buzz_relay`) is filtered out.
    pub(crate) fn capture_access_only(
        level: Level,
    ) -> (tracing::subscriber::DefaultGuard, Captured) {
        let out = Captured::default();
        let subscriber = json_subscriber(&out)
            .with(tracing_subscriber::filter::Targets::new().with_target(TARGET, level));
        (tracing::subscriber::set_default(subscriber), out)
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

    /// The one line of a request dropped mid-flight: aborted at WARN, with
    /// the proven caller it had recorded and no status.
    pub(crate) fn assert_one_aborted_line(out: &Captured, pubkey: &PublicKey) -> serde_json::Value {
        let lines = out.lines();
        assert_eq!(lines.len(), 1, "exactly one access line: {lines:?}");
        let line = lines[0].clone();
        assert_eq!(line["event"], HTTP_EVENT, "{line}");
        assert_eq!(line["request"], "aborted", "{line}");
        assert_eq!(line["level"], "WARN", "{line}");
        assert_eq!(line["pubkey"], pubkey.to_hex(), "{line}");
        assert_eq!(line["pubkey_verified"], true, "{line}");
        assert!(line.get("status").is_none(), "{line}");
        line
    }

    fn filter(json: serde_json::Value) -> Filter {
        serde_json::from_value(json).unwrap()
    }

    const PK_A: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

    fn pk() -> String {
        nostr::Keys::generate().public_key().to_hex()
    }

    /// A request dropped before its response is ready keeps its login and
    /// says the request aborted, at WARN, with no status.
    #[test]
    fn admitted_request_never_completed_logs_aborted() {
        let key = nostr::Keys::generate().public_key();
        let (_, out) = capture(|| {
            let slot = AccessSlot::default();
            slot.attempt(AuthMethod::Nip98, Some(key), true);
            slot.finish(AuthResult::Success);
            slot.abandon("/query");
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
            let slot = AccessSlot::default();
            slot.attempt(AuthMethod::Nip98, None, false);
            slot.finish(AuthResult::Success);
            slot.emit("/query", 200);
        });
        let line = &out.lines()[0];
        assert!(line.get("request").is_none(), "{line}");
        assert_eq!(line["status"], 200, "{line}");
        assert_eq!(line["level"], "INFO", "{line}");
    }

    /// Only a served bridge read that returned nothing drops to DEBUG.
    #[test]
    fn empty_served_http_reads_log_at_debug() {
        let level = |result_count, result, status| {
            let (_, out) = capture(|| {
                let slot = AccessSlot::default();
                slot.attempt(AuthMethod::Nip98, None, false);
                slot.finish(result);
                slot.detail(Detail {
                    result_count,
                    ..Default::default()
                });
                match status {
                    Some(status) => slot.emit("/query", status),
                    None => slot.abandon("/query"),
                }
            });
            out.lines()[0]["level"].as_str().unwrap().to_owned()
        };
        let denied = AuthResult::Denied(AuthReason::NotRelayMember);
        assert_eq!(level(Some(0), AuthResult::Success, Some(200)), "DEBUG");
        assert_eq!(level(Some(3), AuthResult::Success, Some(200)), "INFO");
        assert_eq!(level(None, AuthResult::Success, Some(200)), "INFO");
        assert_eq!(level(Some(0), AuthResult::Success, Some(500)), "WARN");
        assert_eq!(level(Some(0), AuthResult::Success, None), "WARN");
        assert_eq!(level(Some(0), denied, Some(403)), "WARN");
    }

    #[test]
    fn summary_merges_filters_to_the_widest_bounds() {
        let s = FilterSummary::new(&[
            filter(serde_json::json!({"kinds": [9, 7], "since": 100, "until": 200, "limit": 10})),
            filter(serde_json::json!({"kinds": [7, 5], "since": 50, "until": 300, "limit": 50})),
        ]);
        assert_eq!(s.filters, 2);
        // `7` is in both filters: the list hides one value, so the count shows.
        assert_eq!((s.kinds.as_deref(), s.kind_count), (Some("7,9,5"), Some(4)));
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

    /// Lists stop at [`LIST_CAP`] distinct values, and then carry the exact
    /// count; cross-filter duplicates count twice. A list that shows every
    /// value has no count.
    #[test]
    fn summary_caps_lists_and_counts_only_what_they_hide() {
        let channels: Vec<String> = (0..14).map(|_| Uuid::new_v4().to_string()).collect();
        let authors: Vec<String> = (0..5_000).map(|_| pk()).collect();
        let kinds: Vec<u16> = (0..=u16::MAX).collect();
        let s = FilterSummary::new(&[
            filter(serde_json::json!({
                "#h": channels, "authors": authors, "kinds": kinds
            })),
            filter(serde_json::json!({"#h": [channels[0]], "authors": [authors[0]], "kinds": [1]})),
        ]);
        assert_eq!(s.channel_count, Some(15));
        assert_eq!(s.author_count, Some(5_001));
        assert_eq!(s.kind_count, Some(65_537));
        for list in [&s.channel_ids, &s.authors, &s.kinds] {
            let listed: BTreeSet<&str> = list.as_deref().unwrap().split(',').collect();
            assert_eq!(listed.len(), LIST_CAP);
        }
        let s = FilterSummary::new(&[filter(serde_json::json!({"#p": [PK_A], "ids": [PK_A]}))]);
        assert_eq!(
            (s.p_tags.as_deref(), s.p_count, s.ids),
            (Some(PK_A), None, 1)
        );
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
            (Some(1), Some(1), Some(1), Some(1))
        );
    }

    /// A read's level follows its outcome: only a server-side error is WARN.
    #[test]
    fn read_lines_warn_only_on_error() {
        let read = |outcome| ReadLog {
            event: "req",
            conn_id: Uuid::nil(),
            community: CommunityId::from_uuid(Uuid::nil()),
            pubkey: None,
            sub_id: "s".into(),
            summary: FilterSummary::default(),
            outcome,
            results: 0,
            budget: None,
        };
        let (_, out) = capture(|| {
            drop(read(ReadOutcome::Error("database_error")));
            drop(read(ReadOutcome::Aborted));
            drop(read(ReadOutcome::Denied("auth_required")));
        });
        let levels: Vec<_> = out
            .lines()
            .iter()
            .map(|l| format!("{}={}", l["outcome"], l["level"]))
            .collect();
        assert_eq!(
            levels,
            [
                r#""error"="WARN""#,
                r#""aborted"="INFO""#,
                r#""denied"="INFO""#
            ]
        );
    }

    /// Under `buzz_relay::access=warn` an `error` read still carries its
    /// filter summary, and never the search text.
    #[test]
    fn warn_only_error_lines_keep_the_filter_summary() {
        let (conn, _rx) =
            crate::connection::tests::test_conn_with_auth(crate::connection::AuthState::Failed);
        let filters = [filter(serde_json::json!({
            "kinds": [9], "#h": [Uuid::nil().to_string()], "since": 10,
            "search": "secret words",
        }))];
        let (_, out) = capture_warn(|| {
            ReadLog::req(&conn, "s", &filters).error("database_error");
            ReadLog::count(&conn, "c", &filters).error("database_error");
        });
        assert!(!out.raw().contains("secret"), "{}", out.raw());
        let lines = out.lines();
        assert_eq!(lines.len(), 2, "{lines:?}");
        for line in &lines {
            assert_eq!(line["level"], "WARN", "{line}");
            assert_eq!(line["filters"], 1, "{line}");
            assert_eq!(line["kinds"], "9", "{line}");
            assert_eq!(line["channel_ids"], Uuid::nil().to_string(), "{line}");
            assert_eq!(line["since"], 10, "{line}");
            assert_eq!(line["search"], true, "{line}");
        }
    }

    /// HTTP lines that go to WARN keep the read's summary under a
    /// WARN-only filter, without the search text.
    #[test]
    fn warn_only_http_read_lines_keep_the_filter_summary() {
        let filters = [filter(serde_json::json!({
            "kinds": [9], "#h": [Uuid::nil().to_string()], "since": 10,
            "search": "secret words",
        }))];
        let (_, out) = capture_warn(|| {
            let failed = AccessSlot::default();
            failed.read(&filters);
            failed.emit("/query", 500);
            let aborted = AccessSlot::default();
            aborted.read(&filters);
            aborted.abandon("/query");
        });
        assert!(!out.raw().contains("secret"), "{}", out.raw());
        let lines = out.lines();
        assert_eq!(lines.len(), 2, "{lines:?}");
        for line in &lines {
            assert_eq!(line["level"], "WARN", "{line}");
            assert_eq!(line["filters"], 1, "{line}");
            assert_eq!(line["kinds"], "9", "{line}");
            assert!(
                line["channel_ids"].is_string() && line.get("channel_count").is_none(),
                "{line}"
            );
            assert_eq!(line["since"], 10, "{line}");
            assert_eq!(line["search"], true, "{line}");
        }
    }

    #[test]
    fn refusal_budget_logs_sixteen_then_counts_the_rest() {
        let budget = RefusalBudget::default();
        let logged = (0..20).filter(|_| budget.read()).count();
        assert_eq!(logged, 16);
        assert_eq!(RefusalBudget::close(&budget.reads), 4);
        assert_eq!(RefusalBudget::close(&budget.auths), 0);
    }

    /// Refusals charged after `close` was written each log their own line,
    /// so every refusal is either counted on `close` or logged, never both.
    #[test]
    fn refusals_after_close_log_their_own_lines() {
        let (late, out) = capture(|| {
            let close = CloseLine::open(
                Uuid::nil(),
                CommunityId::from_uuid(Uuid::nil()),
                "127.0.0.1:1".parse().unwrap(),
                None,
            );
            let budget = Arc::clone(&close.refusals);
            for _ in 0..(REFUSAL_LINES + 4) {
                budget.read();
                budget.auth();
            }
            drop(close);
            let reads = (0..3).filter(|_| budget.read()).count();
            let auths = (0..3).filter(|_| budget.auth()).count();
            (reads, auths)
        });
        assert_eq!(late, (3, 3), "late refusals must each log");
        let lines = out.lines();
        assert_eq!(lines.len(), 1, "{lines:?}");
        assert_eq!(lines[0]["reads_suppressed"], 4, "{}", lines[0]);
        assert_eq!(lines[0]["auth_suppressed"], 4, "{}", lines[0]);
    }

    #[test]
    fn summary_never_lists_malformed_tag_text() {
        let s = FilterSummary::new(&[filter(serde_json::json!({
            "#h": ["secret plans for tuesday"], "#p": ["not a pubkey"]
        }))]);
        assert_eq!((s.channel_ids, s.channel_count), (None, Some(1)));
        assert_eq!((s.p_tags, s.p_count), (None, Some(1)));
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
                peer_addr: None,
                client_ip: None,
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
                peer_addr: None,
                client_ip: None,
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

    /// Lines the layer writes for `req` through a router with one protected
    /// template, the excluded documents, and a fallback answering `fallback`.
    async fn layer_lines(
        state: Arc<crate::state::AppState>,
        fallback: axum::http::StatusCode,
        req: axum::http::Request<axum::body::Body>,
    ) -> Vec<serde_json::Value> {
        use axum::routing::get;
        use tower::ServiceExt as _;
        let ok = || async { "ok" };
        let router = axum::Router::new()
            .route("/x/{id}", get(ok))
            .route("/", get(ok))
            .route("/info", get(ok))
            .route("/health", get(ok))
            .route("/.well-known/nostr.json", get(ok))
            .fallback(move || async move { fallback })
            .layer(axum::middleware::from_fn_with_state(state, http_access_log));
        let (guard, out) = capture_scoped();
        router.oneshot(req).await.expect("router oneshot");
        drop(guard);
        out.lines()
    }

    fn get_req(path: &str) -> axum::http::Request<axum::body::Body> {
        axum::http::Request::get(path)
            .header("host", "relay.test")
            .body(axum::body::Body::empty())
            .unwrap()
    }

    /// A matched route no handler annotated still gets a line: its template,
    /// no identity, and an undecided outcome.
    #[tokio::test(flavor = "current_thread")]
    async fn unannotated_matched_route_logs_an_empty_note() {
        let state = crate::state::tests::test_state().await;
        let lines = layer_lines(
            state,
            axum::http::StatusCode::NOT_FOUND,
            get_req("/x/secret-id"),
        )
        .await;
        assert_eq!(lines.len(), 1, "{lines:?}");
        let line = &lines[0];
        assert_eq!(line["event"], HTTP_EVENT, "{line}");
        assert_eq!(line["route"], "/x/{id}", "{line}");
        assert_eq!(line["outcome"], "undecided", "{line}");
        assert_eq!(line["status"], 200, "{line}");
        assert!(line.get("pubkey").is_none(), "{line}");
        assert!(line["time_to_response_ms"].is_u64(), "{line}");
        assert!(!line.to_string().contains("secret-id"), "{line}");
    }

    /// Public documents and probes never get a line.
    #[tokio::test(flavor = "current_thread")]
    async fn excluded_routes_log_nothing() {
        let state = crate::state::tests::test_state().await;
        for path in ["/", "/info", "/health", "/.well-known/nostr.json"] {
            let lines = layer_lines(
                state.clone(),
                axum::http::StatusCode::NOT_FOUND,
                get_req(path),
            )
            .await;
            assert!(lines.is_empty(), "{path}: {lines:?}");
        }
    }

    /// An unmatched request is logged at 400 and above as `<unmatched>`,
    /// never with its path, and skipped below 400.
    #[tokio::test(flavor = "current_thread")]
    async fn unmatched_request_logs_only_an_error_without_its_path() {
        let state = crate::state::tests::test_state().await;
        let lines = layer_lines(
            state.clone(),
            axum::http::StatusCode::NOT_FOUND,
            get_req("/invite/INVITECODE123/extra?token=TOKEN456"),
        )
        .await;
        assert_eq!(lines.len(), 1, "{lines:?}");
        assert_eq!(lines[0]["route"], UNMATCHED_ROUTE, "{}", lines[0]);
        assert_eq!(lines[0]["status"], 404, "{}", lines[0]);
        let raw = lines[0].to_string();
        assert!(
            !raw.contains("INVITECODE123") && !raw.contains("TOKEN456"),
            "{raw}"
        );

        let lines = layer_lines(state, axum::http::StatusCode::OK, get_req("/anything")).await;
        assert!(lines.is_empty(), "{lines:?}");
    }

    /// With a web bundle configured, SPA documents are skipped even when
    /// the fallback answers an error, ahead of the unmatched rule.
    #[tokio::test(flavor = "current_thread")]
    async fn spa_document_logs_nothing_at_any_status() {
        let mut state = (*crate::state::tests::test_state().await).clone();
        let mut config = (*state.config).clone();
        config.web_dir = Some(std::env::temp_dir());
        state.config = Arc::new(config);
        let state = Arc::new(state);
        for status in [
            axum::http::StatusCode::OK,
            axum::http::StatusCode::NOT_FOUND,
        ] {
            for path in ["/assets/app.js", "/invite/INVITECODE123"] {
                let lines = layer_lines(state.clone(), status, get_req(path)).await;
                assert!(lines.is_empty(), "{path} {status}: {lines:?}");
            }
        }
    }

    /// The configured header's IP rides the `http` line; an unset setting, a
    /// missing header or a non-IP value logs none, and a list or repeated
    /// fields log the rightmost entry, with no fallback past an invalid one.
    #[tokio::test(flavor = "current_thread")]
    async fn http_line_logs_the_configured_client_ip() {
        let base = crate::state::tests::test_state().await;
        async fn ip(
            base: &Arc<crate::state::AppState>,
            setting: Option<&str>,
            values: &[&str],
        ) -> Option<serde_json::Value> {
            let mut state = (**base).clone();
            Arc::make_mut(&mut state.config).client_ip_header = setting.map(|s| s.parse().unwrap());
            let mut req = get_req("/x/1");
            for value in values {
                req.headers_mut()
                    .append("x-real-ip", value.parse().unwrap());
            }
            let lines = layer_lines(Arc::new(state), axum::http::StatusCode::NOT_FOUND, req).await;
            assert_eq!(lines.len(), 1, "{lines:?}");
            lines[0].get("client_ip").cloned()
        }
        let on = Some("X-Real-IP");
        assert_eq!(
            ip(&base, on, &["203.0.113.7"]).await,
            Some("203.0.113.7".into())
        );
        assert_eq!(ip(&base, None, &["203.0.113.7"]).await, None);
        assert_eq!(ip(&base, on, &[]).await, None);
        assert_eq!(ip(&base, on, &["not-an-ip"]).await, None);
        assert_eq!(
            ip(&base, on, &["198.51.100.1, 10.0.0.2"]).await,
            Some("10.0.0.2".into())
        );
        assert_eq!(
            ip(&base, on, &["198.51.100.1", "10.0.0.2"]).await,
            Some("10.0.0.2".into())
        );
        assert_eq!(ip(&base, on, &["198.51.100.1", "not-an-ip"]).await, None);
    }

    /// A request dropped before its response is ready is logged once as
    /// `request=aborted` by the layer itself.
    #[tokio::test(flavor = "current_thread")]
    async fn layer_logs_a_dropped_request_as_aborted() {
        use axum::routing::get;
        use tower::ServiceExt as _;
        let state = crate::state::tests::test_state().await;
        let router = axum::Router::new()
            .route("/x/{id}", get(std::future::pending::<&'static str>))
            .layer(axum::middleware::from_fn_with_state(state, http_access_log));
        let (guard, out) = capture_scoped();
        let call = router.oneshot(get_req("/x/1"));
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(20), call)
                .await
                .is_err()
        );
        drop(guard);
        let lines = out.lines();
        assert_eq!(lines.len(), 1, "{lines:?}");
        assert_eq!(lines[0]["request"], "aborted", "{}", lines[0]);
        assert_eq!(lines[0]["route"], "/x/{id}", "{}", lines[0]);
        assert!(lines[0].get("status").is_none(), "{}", lines[0]);
    }

    /// A refusal is final: a later admission cannot replace it, while a
    /// later refusal replaces an admission.
    #[test]
    fn refusal_is_final_and_overrides_an_admission() {
        let slot = AccessSlot::default();
        slot.finish(AuthResult::Denied(AuthReason::Banned));
        slot.finish(AuthResult::Success);
        assert_eq!(
            slot.note().decision,
            Some(AuthResult::Denied(AuthReason::Banned))
        );

        let slot = AccessSlot::default();
        slot.finish(AuthResult::Success);
        slot.finish(AuthResult::Failed("nip98_replay"));
        assert_eq!(
            slot.note().decision,
            Some(AuthResult::Failed("nip98_replay"))
        );
    }
}
