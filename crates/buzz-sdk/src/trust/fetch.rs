//! The shared fetch-and-verify core of NIP-FP verification and work trails.
//!
//! Callers ask for events with a semantic NIP-01 filter. The core sends an
//! origin only the constraints the Buzz relay applies before its `LIMIT`, and
//! matches the rest (`#E`, `#P`, `#a`, `#t`, multi-value `#p`, …) itself after
//! fetching, so a constraint applied after the limit can never make a cut-off
//! page look complete. Discovery pages backwards with an inclusive `until`
//! within a page budget. Every event is checked byte for byte; nothing here
//! decides what an event means.

use std::collections::{BTreeMap, BTreeSet};

use nostr::filter::MatchEventOptions;
use nostr::{Event, EventId, Filter, PublicKey, Timestamp};

use super::evidence;
use super::{CoverageRecord, OwnerLink, Query, Request, Response};
use crate::SdkError;

/// `limit` on every discovery page. Below the Buzz relay's page clamp, so a
/// full page is detectable.
pub const QUERY_LIMIT: usize = 500;

/// Pages one discovery walk may fetch before it is reported as truncated.
pub const MAX_PAGES: usize = 8;

/// Which step of a run a coverage record belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Step {
    RelayInfo,
    Selected,
    Parents,
    Outcomes,
    Attention,
    BySubject,
    ByIssuer,
    Repo,
    RefState,
    Targets,
    /// Typed messages and code contributions by an actor, or the entry event.
    Seed,
    /// Replies to known requests and updates of known proposals.
    Linked,
}

impl Step {
    /// Steps whose events may be outcomes.
    pub(super) fn finds_outcomes(self) -> bool {
        matches!(self, Self::Outcomes | Self::BySubject | Self::ByIssuer)
    }
}

/// One fetched event, kept with every coverage record that returned it.
pub(super) struct Stored {
    pub(super) event: Event,
    pub(super) valid: bool,
    pub(super) origin: String,
    pub(super) scopes: Vec<usize>,
}

/// One page handed out as a [`Query`].
struct Sent {
    record: usize,
    filter: Option<Filter>,
    answered: bool,
}

/// A paged discovery read for one coverage record.
struct Walk {
    pushed: Filter,
    local: Filter,
    seen: BTreeSet<EventId>,
    pages: usize,
}

/// Queries, coverage, and verified events shared by every run.
#[derive(Default)]
pub(super) struct Fetch {
    sent: Vec<Sent>,
    steps: Vec<Step>,
    walks: BTreeMap<usize, Walk>,
    pending: Vec<(usize, Filter)>,
    pub(super) coverage: Vec<CoverageRecord>,
    pub(super) store: BTreeMap<EventId, Stored>,
    pub(super) relay_self: BTreeMap<String, PublicKey>,
}

impl Fetch {
    /// Record every query of the last round left unanswered as failed, then
    /// hand out the next pages of unfinished walks.
    pub(super) fn next_pages(&mut self) -> Vec<Query> {
        for sent in self.sent.iter_mut().filter(|s| !s.answered) {
            sent.answered = true;
            self.coverage[sent.record].error = Some("not answered".into());
            self.walks.remove(&sent.record);
        }
        let pending = std::mem::take(&mut self.pending);
        pending
            .into_iter()
            .map(|(record, filter)| self.page(record, filter))
            .collect()
    }

    /// Ask `origin` for its NIP-11 `self` key.
    pub(super) fn relay_info(&mut self, origin: &str, step: Step) -> Query {
        let record = self.record(origin, step, serde_json::json!("nip11"), None, None);
        self.dispatch(record, None)
    }

    /// Fetch exact events by id; `None` when `ids` is empty.
    pub(super) fn ids(
        &mut self,
        origin: &str,
        step: Step,
        ids: BTreeSet<EventId>,
    ) -> Option<Query> {
        if ids.is_empty() {
            return None;
        }
        let filter = Filter::new().limit(ids.len()).ids(ids);
        let record = self.record(origin, step, to_json(&filter), None, None);
        Some(self.dispatch(record, Some(filter)))
    }

    /// Start a paged discovery read for `filter`. Pages go backwards from its
    /// `until` (the cutoff) until a short page, the page budget, or a page
    /// boundary that cannot be crossed without possibly skipping events.
    pub(super) fn walk(&mut self, origin: &str, step: Step, filter: Filter) -> Query {
        let (pushed, local) = split(filter);
        let pushed = pushed.limit(QUERY_LIMIT);
        let local_json = (!local.generic_tags.is_empty()).then(|| to_json(&local));
        let cutoff = pushed.until.map(|t| t.as_secs());
        let record = self.record(origin, step, to_json(&pushed), local_json, cutoff);
        self.walks.insert(
            record,
            Walk {
                pushed: pushed.clone(),
                local,
                seen: BTreeSet::new(),
                pages: 0,
            },
        );
        self.dispatch(record, Some(pushed))
    }

    fn record(
        &mut self,
        origin: &str,
        step: Step,
        request: serde_json::Value,
        local: Option<serde_json::Value>,
        cutoff: Option<u64>,
    ) -> usize {
        let scope = self.coverage.len();
        self.coverage.push(CoverageRecord {
            scope,
            origin: origin.to_owned(),
            request,
            local,
            cutoff,
            pages: 0,
            returned: 0,
            discarded: 0,
            unmatched: 0,
            incomplete: None,
            error: None,
        });
        self.steps.push(step);
        scope
    }

    fn dispatch(&mut self, record: usize, filter: Option<Filter>) -> Query {
        match filter {
            Some(filter) => self.page(record, filter),
            None => self.send(record, None, Request::RelayInfo),
        }
    }

    fn page(&mut self, record: usize, filter: Filter) -> Query {
        self.send(record, Some(filter.clone()), Request::Events(filter))
    }

    fn send(&mut self, record: usize, filter: Option<Filter>, request: Request) -> Query {
        let handle = self.sent.len();
        self.sent.push(Sent {
            record,
            filter,
            answered: false,
        });
        Query {
            scope: handle,
            origin: self.coverage[record].origin.clone(),
            request,
        }
    }

    /// The step of coverage record `scope`.
    pub(super) fn step(&self, scope: usize) -> Option<Step> {
        self.steps.get(scope).copied()
    }

    /// Whether `stored` was returned by a record of `step`.
    pub(super) fn returned_by(&self, stored: &Stored, step: Step) -> bool {
        stored.scopes.iter().any(|s| self.step(*s) == Some(step))
    }

    /// The validly signed event with this id, if fetched.
    pub(super) fn valid(&self, id: &EventId) -> Option<&Event> {
        self.store.get(id).filter(|s| s.valid).map(|s| &s.event)
    }

    /// Owner keys of each signer, learned only from verified owner links on
    /// validly signed stored events that `read` admits.
    pub(super) fn owners(
        &self,
        read: impl Fn(&Stored) -> bool,
    ) -> BTreeMap<PublicKey, BTreeSet<PublicKey>> {
        let mut owners: BTreeMap<PublicKey, BTreeSet<PublicKey>> = BTreeMap::new();
        for stored in self.store.values().filter(|s| s.valid && read(s)) {
            if let OwnerLink::Verified(hex) = evidence::owner_link(&stored.event) {
                if let Ok(owner) = PublicKey::from_hex(&hex) {
                    owners.entry(stored.event.pubkey).or_default().insert(owner);
                }
            }
        }
        owners
    }

    /// Record an origin's answer to the query with handle `scope`.
    ///
    /// Events that do not parse or do not match the filter sent are discarded
    /// and counted; events the origin could not filter are matched locally.
    ///
    /// # Errors
    ///
    /// Returns [`SdkError::InvalidInput`] for an unknown or already answered
    /// handle.
    pub(super) fn ingest(&mut self, scope: usize, response: Response) -> Result<(), SdkError> {
        let Some(sent) = self.sent.get_mut(scope) else {
            return Err(SdkError::InvalidInput(format!("unknown scope {scope}")));
        };
        if sent.answered {
            return Err(SdkError::InvalidInput(format!(
                "scope {scope} already answered"
            )));
        }
        sent.answered = true;
        let record = sent.record;
        let filter = sent.filter.take();
        let nip11 = self.coverage[record].request.is_string();
        match (response, filter) {
            (Response::Failed(reason), _) => {
                self.coverage[record].error = Some(reason);
                self.walks.remove(&record);
            }
            (Response::RelayInfo(pk), _) if nip11 => {
                let coverage = &mut self.coverage[record];
                coverage.pages = 1;
                coverage.returned = usize::from(pk.is_some());
                if let Some(pk) = pk {
                    self.relay_self.insert(coverage.origin.clone(), pk);
                }
            }
            (Response::Events(values), Some(filter)) if !nip11 => {
                self.events(record, &filter, values);
            }
            _ => {
                self.coverage[record].error = Some("response does not answer this request".into());
                self.walks.remove(&record);
            }
        }
        Ok(())
    }

    fn events(&mut self, record: usize, filter: &Filter, values: Vec<serde_json::Value>) {
        let served = values.len();
        let mut events = Vec::new();
        for value in values {
            match serde_json::from_value::<Event>(value) {
                Ok(event) if filter.match_event(&event, MatchEventOptions::new()) => {
                    events.push(event);
                }
                _ => self.coverage[record].discarded += 1,
            }
        }
        let origin = self.coverage[record].origin.clone();
        self.coverage[record].pages += 1;
        let Some(walk) = self.walks.get_mut(&record) else {
            self.coverage[record].returned += events.len();
            for event in events {
                store(&mut self.store, record, &origin, event);
            }
            return;
        };
        walk.pages += 1;
        let oldest = events.iter().map(|e| e.created_at).min();
        let mut keep = Vec::new();
        for event in events {
            if !walk.seen.insert(event.id) {
                continue;
            }
            if walk.local.match_event(&event, MatchEventOptions::new()) {
                keep.push(event);
            } else {
                self.coverage[record].unmatched += 1;
            }
        }
        let next = next_page(walk, filter, served, oldest);
        let coverage = &mut self.coverage[record];
        coverage.returned += keep.len();
        match next {
            Paging::Done => {
                self.walks.remove(&record);
            }
            Paging::Stop(reason) => {
                coverage.incomplete = Some(reason);
                self.walks.remove(&record);
            }
            Paging::Next(page, tie) => {
                if let Some(reason) = tie {
                    coverage.incomplete = Some(reason);
                }
                self.pending.push((record, page));
            }
        }
        for event in keep {
            store(&mut self.store, record, &origin, event);
        }
    }
}

enum Paging {
    Done,
    Stop(String),
    Next(Filter, Option<String>),
}

/// Decide the next page of a walk after one page came back. NIP-01 `until`
/// is inclusive and has no tie-breaker, so a full page inside one second
/// cannot be paged past without possibly skipping events.
fn next_page(walk: &Walk, sent: &Filter, served: usize, oldest: Option<Timestamp>) -> Paging {
    if served < QUERY_LIMIT {
        return Paging::Done;
    }
    if walk.pages >= MAX_PAGES {
        return Paging::Stop(format!(
            "page budget of {MAX_PAGES} pages of {QUERY_LIMIT} reached; older events not fetched"
        ));
    }
    let Some(oldest) = oldest else {
        return Paging::Stop("a full page held no usable events; cannot page further".into());
    };
    let (until, tie) = if sent.until == Some(oldest) {
        let secs = oldest.as_secs();
        let reason = format!(
            "at least {QUERY_LIMIT} events share second {secs}; some may not have been returned"
        );
        match secs.checked_sub(1) {
            Some(prev) => (Timestamp::from(prev), Some(reason)),
            None => return Paging::Stop(reason),
        }
    } else {
        (oldest, None)
    };
    if walk.pushed.since.is_some_and(|since| until < since) {
        return match tie {
            Some(reason) => Paging::Stop(reason),
            None => Paging::Done,
        };
    }
    Paging::Next(walk.pushed.clone().until(until), tie)
}

fn store(store: &mut BTreeMap<EventId, Stored>, record: usize, origin: &str, event: Event) {
    let valid = event.verify().is_ok();
    let entry = store.entry(event.id).or_insert_with(|| Stored {
        event: event.clone(),
        valid,
        origin: origin.to_owned(),
        scopes: Vec::new(),
    });
    if valid && !entry.valid {
        entry.event = event;
        entry.valid = true;
        entry.origin = origin.to_owned();
    }
    if !entry.scopes.contains(&record) {
        entry.scopes.push(record);
    }
}

/// Split `filter` into what the Buzz relay applies before its `LIMIT` and the
/// tag constraints it applies only afterwards (`buzz-relay`
/// `filter_fully_pushable`): kinds, authors, ids, since, until, `#h`, `#e`,
/// single-value `#p`, and `#d` on NIP-33-only kinds are pushed.
pub(super) fn split(filter: Filter) -> (Filter, Filter) {
    let nip33_only = filter.kinds.as_ref().is_some_and(|kinds| {
        !kinds.is_empty()
            && kinds
                .iter()
                .all(|k| buzz_core::kind::is_parameterized_replaceable(u32::from(k.as_u16())))
    });
    let mut pushed = filter;
    let mut local = Filter::new();
    let tags = std::mem::take(&mut pushed.generic_tags);
    for (tag, values) in tags {
        let keep = !tag.uppercase
            && match tag.character {
                nostr::Alphabet::H | nostr::Alphabet::E => true,
                nostr::Alphabet::P => values.len() == 1,
                nostr::Alphabet::D => nip33_only,
                _ => false,
            };
        if keep {
            pushed.generic_tags.insert(tag, values);
        } else {
            local.generic_tags.insert(tag, values);
        }
    }
    (pushed, local)
}

fn to_json(filter: &Filter) -> serde_json::Value {
    serde_json::to_value(filter).unwrap_or(serde_json::Value::Null)
}
