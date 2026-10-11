//! The sans-IO NIP-FP discovery state machine (NIP-FP §Discovery).

use std::collections::{BTreeMap, BTreeSet};

use nostr::filter::MatchEventOptions;
use nostr::{Alphabet, Event, EventId, Filter, Kind, PublicKey, SingleLetterTag, Timestamp};

use super::evidence::{
    self, Coord, ATTENTION_KINDS, KEYED_OUTCOME_KINDS, OUTCOME_KINDS, STATUS_KINDS,
};
use super::{
    fingerprint_json, CoverageRecord, Gap, OwnerLink, Policy, Query, Request, Response, Selection,
    SelectionItem, Validity, BUNDLE_LABEL, FINGERPRINT_KIND, MAX_SELECTIONS,
};
use crate::SdkError;

/// `limit` on every discovery query. Below the Buzz relay's page clamp, so a
/// full page is detectable as truncation.
pub const QUERY_LIMIT: usize = 500;

/// Which discovery step a coverage record belongs to.
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
}

impl Step {
    /// Steps whose events may be outcomes.
    pub(super) fn finds_outcomes(self) -> bool {
        matches!(self, Self::Outcomes | Self::BySubject | Self::ByIssuer)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    Start,
    Discover,
    Targets,
    Context,
    Done,
}

pub(super) struct Sent {
    pub(super) step: Step,
    filter: Option<Filter>,
    answered: bool,
}

/// One fetched event, kept with every scope that returned it.
pub(super) struct Stored {
    pub(super) event: Event,
    pub(super) valid: bool,
    pub(super) origin: String,
    pub(super) scopes: Vec<usize>,
}

/// Verifies one work fingerprint. Sans-IO: call [`Verifier::next_queries`],
/// answer each [`Query`] with [`Verifier::ingest`], and repeat until no
/// queries come back; then call [`Verifier::finish`].
///
/// Queries left unanswered when `next_queries` is called again are recorded
/// as failed, so everything they would have found stays unknown.
pub struct Verifier {
    pub(super) policy: Policy,
    pub(super) cutoff: u64,
    pub(super) selection: Selection,
    pub(super) subject: Option<PublicKey>,
    /// Processable selections, deduplicated, in order.
    pub(super) work: Vec<(EventId, String)>,
    pub(super) gaps: Vec<Gap>,
    pub(super) sent: Vec<Sent>,
    pub(super) coverage: Vec<CoverageRecord>,
    pub(super) store: BTreeMap<EventId, Stored>,
    pub(super) relay_self: BTreeMap<String, PublicKey>,
    pub(super) attention: BTreeMap<EventId, usize>,
    phase: Phase,
    repos: BTreeSet<(String, Coord)>,
}

impl Verifier {
    /// Start verifying a transport bundle or a bare fingerprint event.
    ///
    /// `cutoff` bounds every discovery query (`until`), in Unix seconds.
    ///
    /// # Errors
    ///
    /// Returns [`SdkError::InvalidInput`] for a bundle of another type or
    /// version. A malformed or badly signed fingerprint is not an error: the
    /// report marks it invalid and nothing is fetched.
    pub fn new(input: &serde_json::Value, policy: Policy, cutoff: u64) -> Result<Self, SdkError> {
        let transport = input.get("type").is_some().then_some(BUNDLE_LABEL);
        let fingerprint = fingerprint_json(input)?;
        let mut verifier = Self {
            policy,
            cutoff,
            selection: Selection {
                transport,
                fingerprint: Validity::Invalid,
                invalid_reason: None,
                id: None,
                subject: None,
                d: None,
                created_at: None,
                coverage: "selected",
                owner_link: OwnerLink::None,
                items: Vec::new(),
                excess: 0,
            },
            subject: None,
            work: Vec::new(),
            gaps: Vec::new(),
            sent: Vec::new(),
            coverage: Vec::new(),
            store: BTreeMap::new(),
            relay_self: BTreeMap::new(),
            attention: BTreeMap::new(),
            phase: Phase::Start,
            repos: BTreeSet::new(),
        };
        verifier.read_fingerprint(fingerprint);
        Ok(verifier)
    }

    fn read_fingerprint(&mut self, value: &serde_json::Value) {
        let event: Event = match serde_json::from_value(value.clone()) {
            Ok(event) => event,
            Err(e) => {
                self.selection.invalid_reason = Some(format!("not an event: {e}"));
                return;
            }
        };
        let s = &mut self.selection;
        s.id = Some(event.id.to_hex());
        s.subject = Some(event.pubkey.to_hex());
        s.created_at = Some(event.created_at.as_secs());
        s.d = evidence::tag_value(&event, "d").map(str::to_owned);
        let d_tags = evidence::tags(&event, "d").count();
        s.invalid_reason = if event.verify().is_err() {
            Some("id or signature does not verify".into())
        } else if event.kind.as_u16() != FINGERPRINT_KIND {
            Some(format!(
                "kind {} is not {FINGERPRINT_KIND}",
                event.kind.as_u16()
            ))
        } else if d_tags != 1 {
            Some(format!("{d_tags} d tags; exactly one is required"))
        } else {
            None
        };
        if s.invalid_reason.is_some() {
            return;
        }
        s.fingerprint = Validity::Valid;
        s.owner_link = evidence::owner_link(&event);
        self.subject = Some(event.pubkey);

        let mut seen = BTreeSet::new();
        for (index, parts) in evidence::tags(&event, "e").enumerate() {
            let raw_id = parts.get(1).cloned().unwrap_or_default();
            let origin = parts.get(2).filter(|o| !o.is_empty()).cloned();
            if index >= MAX_SELECTIONS {
                self.selection.excess += 1;
                self.gaps.push(Gap {
                    id: Some(raw_id),
                    reason: format!("beyond the {MAX_SELECTIONS}-selection limit; unknown"),
                });
                continue;
            }
            self.selection.items.push(SelectionItem {
                id: raw_id.clone(),
                origin: origin.clone(),
            });
            match (EventId::from_hex(&raw_id), origin) {
                (Ok(id), Some(origin)) => {
                    if seen.insert(id) {
                        self.work.push((id, origin));
                    }
                }
                (Err(_), _) => self.gaps.push(Gap {
                    id: Some(raw_id),
                    reason: "selection id is not an event id; unknown".into(),
                }),
                (Ok(_), None) => {}
            }
        }
    }

    /// The queries for the next discovery round; empty when discovery is done.
    pub fn next_queries(&mut self) -> Vec<Query> {
        for (sent, record) in self.sent.iter_mut().zip(&mut self.coverage) {
            if !sent.answered {
                sent.answered = true;
                record.error = Some("not answered".into());
            }
        }
        let Some(subject) = self.subject else {
            self.phase = Phase::Done;
            return Vec::new();
        };
        loop {
            let queries = match self.phase {
                Phase::Start => {
                    self.phase = Phase::Discover;
                    self.plan_start()
                }
                Phase::Discover => {
                    self.phase = Phase::Targets;
                    self.plan_discover(subject)
                }
                Phase::Targets => {
                    self.phase = Phase::Context;
                    self.plan_targets()
                }
                Phase::Context => {
                    self.phase = Phase::Done;
                    self.plan_context()
                }
                Phase::Done => return Vec::new(),
            };
            if !queries.is_empty() {
                return queries;
            }
        }
    }

    /// Record an origin's answer to the query with this `scope`.
    ///
    /// Events that do not parse or do not match the filter sent are discarded
    /// and counted.
    ///
    /// # Errors
    ///
    /// Returns [`SdkError::InvalidInput`] for an unknown or already answered
    /// scope.
    pub fn ingest(&mut self, scope: usize, response: Response) -> Result<(), SdkError> {
        let (Some(sent), Some(record)) = (self.sent.get_mut(scope), self.coverage.get_mut(scope))
        else {
            return Err(SdkError::InvalidInput(format!("unknown scope {scope}")));
        };
        if sent.answered {
            return Err(SdkError::InvalidInput(format!(
                "scope {scope} already answered"
            )));
        }
        sent.answered = true;
        let step = sent.step;
        match (response, &sent.filter) {
            (Response::Failed(reason), _) => record.error = Some(reason),
            (Response::RelayInfo(pk), None) => {
                record.returned = usize::from(pk.is_some());
                if let Some(pk) = pk {
                    self.relay_self.insert(record.origin.clone(), pk);
                }
            }
            (Response::Events(values), Some(filter)) => {
                let filter = filter.clone();
                let origin = record.origin.clone();
                record.truncated =
                    filter.ids.is_none() && filter.limit.is_some_and(|limit| values.len() >= limit);
                let mut accepted = Vec::new();
                for value in values {
                    match serde_json::from_value::<Event>(value) {
                        Ok(event) if filter.match_event(&event, MatchEventOptions::new()) => {
                            accepted.push(event);
                        }
                        _ => record.discarded += 1,
                    }
                }
                record.returned = accepted.len();
                for event in accepted {
                    self.accept(scope, step, &origin, event);
                }
            }
            (_, _) => record.error = Some("response does not answer this request".into()),
        }
        Ok(())
    }

    fn accept(&mut self, scope: usize, step: Step, origin: &str, event: Event) {
        let valid = event.verify().is_ok();
        if step == Step::Attention {
            if valid {
                let work: BTreeSet<EventId> = self.work.iter().map(|(id, _)| *id).collect();
                for id in evidence::e_ids(&event).into_iter().collect::<BTreeSet<_>>() {
                    if work.contains(&id) {
                        *self.attention.entry(id).or_default() += 1;
                    }
                }
            }
            return;
        }
        let entry = self.store.entry(event.id).or_insert_with(|| Stored {
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
        entry.scopes.push(scope);
    }

    fn send(&mut self, origin: &str, step: Step, request: Request) -> Query {
        let scope = self.sent.len();
        let (filter, json) = match &request {
            Request::RelayInfo => (None, serde_json::json!("nip11")),
            Request::Events(filter) => (
                Some(filter.clone()),
                serde_json::to_value(filter).unwrap_or(serde_json::Value::Null),
            ),
        };
        self.coverage.push(CoverageRecord {
            scope,
            origin: origin.to_owned(),
            cutoff: filter.as_ref().and_then(|f| f.until).map(|t| t.as_secs()),
            request: json,
            returned: 0,
            discarded: 0,
            truncated: false,
            error: None,
        });
        self.sent.push(Sent {
            step,
            filter,
            answered: false,
        });
        Query {
            scope,
            origin: origin.to_owned(),
            request,
        }
    }

    fn ids(&mut self, origin: &str, step: Step, ids: BTreeSet<EventId>) -> Option<Query> {
        if ids.is_empty() {
            return None;
        }
        let limit = ids.len();
        Some(self.send(
            origin,
            step,
            Request::Events(Filter::new().ids(ids).limit(limit)),
        ))
    }

    fn discovery(&self, kinds: &[u16]) -> Filter {
        Filter::new()
            .kinds(kinds.iter().map(|k| Kind::from(*k)))
            .until(Timestamp::from(self.cutoff))
            .limit(QUERY_LIMIT)
    }

    fn by_origin(&self) -> BTreeMap<String, BTreeSet<EventId>> {
        let mut map: BTreeMap<String, BTreeSet<EventId>> = BTreeMap::new();
        for (id, origin) in &self.work {
            map.entry(origin.clone()).or_default().insert(*id);
        }
        map
    }

    fn plan_start(&mut self) -> Vec<Query> {
        let mut queries = Vec::new();
        for (origin, ids) in self.by_origin() {
            queries.push(self.send(&origin, Step::RelayInfo, Request::RelayInfo));
            queries.extend(self.ids(&origin, Step::Selected, ids));
        }
        queries
    }

    fn plan_discover(&mut self, subject: PublicKey) -> Vec<Query> {
        let mut queries = Vec::new();
        for (origin, ids) in self.by_origin() {
            let fetched: Vec<&Event> = ids
                .iter()
                .filter_map(|id| self.store.get(id))
                .filter(|s| s.valid)
                .map(|s| &s.event)
                .collect();
            let parents: BTreeSet<EventId> = fetched
                .iter()
                .filter_map(|e| evidence::parent(e))
                .filter(|id| !self.store.contains_key(id))
                .collect();
            let repos: BTreeSet<Coord> = fetched.iter().filter_map(|e| evidence::repo(e)).collect();
            let hex: Vec<String> = ids.iter().map(EventId::to_hex).collect();
            let outcomes = self
                .discovery(&OUTCOME_KINDS)
                .custom_tags(tag(Alphabet::E), hex.clone());
            let attention = self
                .discovery(&ATTENTION_KINDS)
                .custom_tags(tag(Alphabet::E), hex);
            let keyed = self
                .discovery(&KEYED_OUTCOME_KINDS)
                .custom_tags(tag(Alphabet::P), [subject.to_hex()]);
            queries.extend(self.ids(&origin, Step::Parents, parents));
            queries.push(self.send(&origin, Step::Outcomes, Request::Events(outcomes)));
            queries.push(self.send(&origin, Step::Attention, Request::Events(attention)));
            queries.push(self.send(&origin, Step::BySubject, Request::Events(keyed)));
            for coord in repos {
                let issuer = self
                    .discovery(&STATUS_KINDS)
                    .custom_tags(tag(Alphabet::A), [coord.to_string()]);
                queries.push(self.send(&origin, Step::ByIssuer, Request::Events(issuer)));
                queries.extend(self.plan_repo(&origin, coord));
            }
        }
        queries
    }

    fn plan_repo(&mut self, origin: &str, coord: Coord) -> Vec<Query> {
        if !self.repos.insert((origin.to_owned(), coord.clone())) {
            return Vec::new();
        }
        let announcement = Filter::new()
            .kind(Kind::from(30617))
            .author(coord.owner)
            .identifier(coord.d.clone())
            .limit(QUERY_LIMIT);
        let state = self.discovery(&[30618]).identifier(coord.d);
        vec![
            self.send(origin, Step::Repo, Request::Events(announcement)),
            self.send(origin, Step::RefState, Request::Events(state)),
        ]
    }

    /// Fetch the targets of discovered outcomes, to learn their authors.
    fn plan_targets(&mut self) -> Vec<Query> {
        let mut wanted: BTreeMap<String, BTreeSet<EventId>> = BTreeMap::new();
        for stored in self.store.values() {
            if !stored.valid || !self.is_outcome_candidate(stored) {
                continue;
            }
            if let Some(target) = evidence::target(&stored.event) {
                if !self.store.contains_key(&target) {
                    wanted
                        .entry(stored.origin.clone())
                        .or_default()
                        .insert(target);
                }
            }
        }
        let mut queries = Vec::new();
        for (origin, ids) in wanted {
            queries.extend(self.ids(&origin, Step::Targets, ids));
        }
        queries
    }

    /// Fetch the parents and repositories of discovered work by the subject.
    fn plan_context(&mut self) -> Vec<Query> {
        let owners = self.owners();
        let mut parents: BTreeMap<String, BTreeSet<EventId>> = BTreeMap::new();
        let mut repos = BTreeSet::new();
        for stored in self.store.values() {
            let discovered = stored
                .scopes
                .iter()
                .any(|s| self.step(*s) == Some(Step::Targets));
            if !stored.valid || !discovered || !self.attributed(&stored.event, &owners) {
                continue;
            }
            if let Some(parent) = evidence::parent(&stored.event) {
                if !self.store.contains_key(&parent) {
                    parents
                        .entry(stored.origin.clone())
                        .or_default()
                        .insert(parent);
                }
            }
            if let Some(coord) = evidence::repo(&stored.event) {
                repos.insert((stored.origin.clone(), coord));
            }
        }
        let mut queries = Vec::new();
        for (origin, ids) in parents {
            queries.extend(self.ids(&origin, Step::Parents, ids));
        }
        for (origin, coord) in repos {
            queries.extend(self.plan_repo(&origin, coord));
        }
        queries
    }

    pub(super) fn step(&self, scope: usize) -> Option<Step> {
        self.sent.get(scope).map(|s| s.step)
    }

    /// Whether a stored event came from an outcome query and has an outcome
    /// kind. Its target decides whether it is reported.
    pub(super) fn is_outcome_candidate(&self, stored: &Stored) -> bool {
        let kind = stored.event.kind.as_u16();
        stored
            .scopes
            .iter()
            .any(|s| self.step(*s).is_some_and(Step::finds_outcomes))
            && (kind == 5
                || STATUS_KINDS.contains(&kind)
                || evidence::label(&stored.event).is_some())
    }

    /// Owner keys of each signer, learned only from verified owner links on
    /// validly signed events by that signer.
    pub(super) fn owners(&self) -> BTreeMap<PublicKey, BTreeSet<PublicKey>> {
        let mut owners: BTreeMap<PublicKey, BTreeSet<PublicKey>> = BTreeMap::new();
        let mut link = |signer: PublicKey, owner: &OwnerLink| {
            if let OwnerLink::Verified(hex) = owner {
                if let Ok(owner) = PublicKey::from_hex(hex) {
                    owners.entry(signer).or_default().insert(owner);
                }
            }
        };
        if let Some(subject) = self.subject {
            link(subject, &self.selection.owner_link);
        }
        for stored in self.store.values().filter(|s| s.valid) {
            link(stored.event.pubkey, &evidence::owner_link(&stored.event));
        }
        owners
    }

    /// Whether `event` counts toward the subject: signed by it, or by a key
    /// whose owner link names it.
    pub(super) fn attributed(
        &self,
        event: &Event,
        owners: &BTreeMap<PublicKey, BTreeSet<PublicKey>>,
    ) -> bool {
        self.subject.is_some_and(|subject| {
            event.pubkey == subject
                || owners
                    .get(&event.pubkey)
                    .is_some_and(|o| o.contains(&subject))
        })
    }
}

fn tag(letter: Alphabet) -> SingleLetterTag {
    SingleLetterTag::lowercase(letter)
}
