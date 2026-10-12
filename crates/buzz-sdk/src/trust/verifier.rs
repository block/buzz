//! The sans-IO NIP-FP discovery state machine (NIP-FP §Discovery).

use std::collections::{BTreeMap, BTreeSet};

use nostr::{Alphabet, Event, EventId, Filter, Kind, PublicKey, SingleLetterTag, Timestamp};

use super::evidence::{
    self, Coord, ATTENTION_KINDS, KEYED_OUTCOME_KINDS, OUTCOME_KINDS, STATUS_KINDS,
};
use super::fetch::{Fetch, Step, Stored};
use super::{
    fingerprint_json, Gap, OwnerLink, Policy, Query, Response, Selection, SelectionItem, Validity,
    BUNDLE_LABEL, FINGERPRINT_KIND, MAX_SELECTIONS,
};
use crate::SdkError;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    Start,
    Discover,
    Targets,
    Context,
    Done,
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
    pub(super) fetch: Fetch,
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
            fetch: Fetch::default(),
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
    ///
    /// Queries of the previous round left unanswered are recorded as failed.
    pub fn next_queries(&mut self) -> Vec<Query> {
        let pages = self.fetch.next_pages();
        if !pages.is_empty() {
            return pages;
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
        self.fetch.ingest(scope, response)
    }

    fn discovery(&self, kinds: &[u16]) -> Filter {
        Filter::new()
            .kinds(kinds.iter().map(|k| Kind::from(*k)))
            .until(Timestamp::from(self.cutoff))
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
            queries.push(self.fetch.relay_info(&origin, Step::RelayInfo));
            queries.extend(self.fetch.ids(&origin, Step::Selected, ids));
        }
        queries
    }

    fn plan_discover(&mut self, subject: PublicKey) -> Vec<Query> {
        let mut queries = Vec::new();
        for (origin, ids) in self.by_origin() {
            let fetched: Vec<&Event> = ids.iter().filter_map(|id| self.fetch.valid(id)).collect();
            let parents: BTreeSet<EventId> = fetched
                .iter()
                .filter_map(|e| evidence::parent(e))
                .filter(|id| !self.fetch.store.contains_key(id))
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
            queries.extend(self.fetch.ids(&origin, Step::Parents, parents));
            queries.push(self.fetch.walk(&origin, Step::Outcomes, outcomes));
            queries.push(self.fetch.walk(&origin, Step::Attention, attention));
            queries.push(self.fetch.walk(&origin, Step::BySubject, keyed));
            for coord in repos {
                let issuer = self
                    .discovery(&STATUS_KINDS)
                    .custom_tags(tag(Alphabet::A), [coord.to_string()]);
                queries.push(self.fetch.walk(&origin, Step::ByIssuer, issuer));
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
            .identifier(coord.d.clone());
        let state = self.discovery(&[30618]).identifier(coord.d);
        vec![
            self.fetch.walk(origin, Step::Repo, announcement),
            self.fetch.walk(origin, Step::RefState, state),
        ]
    }

    /// Fetch the targets of discovered outcomes, to learn their authors.
    fn plan_targets(&mut self) -> Vec<Query> {
        let mut wanted: BTreeMap<String, BTreeSet<EventId>> = BTreeMap::new();
        for stored in self.fetch.store.values() {
            if !stored.valid || !self.is_outcome_candidate(stored) {
                continue;
            }
            if let Some(target) = evidence::target(&stored.event) {
                if !self.fetch.store.contains_key(&target) {
                    wanted
                        .entry(stored.origin.clone())
                        .or_default()
                        .insert(target);
                }
            }
        }
        let mut queries = Vec::new();
        for (origin, ids) in wanted {
            queries.extend(self.fetch.ids(&origin, Step::Targets, ids));
        }
        queries
    }

    /// Fetch the parents and repositories of discovered work by the subject.
    fn plan_context(&mut self) -> Vec<Query> {
        let owners = self.owners();
        let mut parents: BTreeMap<String, BTreeSet<EventId>> = BTreeMap::new();
        let mut repos = BTreeSet::new();
        for stored in self.fetch.store.values() {
            let discovered = self.fetch.returned_by(stored, Step::Targets);
            if !stored.valid || !discovered || !self.attributed(&stored.event, &owners) {
                continue;
            }
            if let Some(parent) = evidence::parent(&stored.event) {
                if !self.fetch.store.contains_key(&parent) {
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
            queries.extend(self.fetch.ids(&origin, Step::Parents, ids));
        }
        for (origin, coord) in repos {
            queries.extend(self.plan_repo(&origin, coord));
        }
        queries
    }

    /// Whether a stored event came from an outcome query and has an outcome
    /// kind. Its target decides whether it is reported.
    pub(super) fn is_outcome_candidate(&self, stored: &Stored) -> bool {
        let kind = stored.event.kind.as_u16();
        stored
            .scopes
            .iter()
            .any(|s| self.fetch.step(*s).is_some_and(Step::finds_outcomes))
            && (kind == 5
                || STATUS_KINDS.contains(&kind)
                || evidence::label(&stored.event).is_some())
    }

    /// Owner keys of each signer, learned only from verified owner links on
    /// validly signed events by that signer. Attention events are counted,
    /// not read for owner links.
    pub(super) fn owners(&self) -> BTreeMap<PublicKey, BTreeSet<PublicKey>> {
        let mut owners = self.fetch.owners(|s| {
            s.scopes
                .iter()
                .any(|scope| self.fetch.step(*scope) != Some(Step::Attention))
        });
        if let (Some(subject), OwnerLink::Verified(hex)) =
            (self.subject, &self.selection.owner_link)
        {
            if let Ok(owner) = PublicKey::from_hex(hex) {
                owners.entry(subject).or_default().insert(owner);
            }
        }
        owners
    }

    /// Replies, mentions and reactions found for each selected work event.
    pub(super) fn attention(&self) -> BTreeMap<EventId, usize> {
        let work: BTreeSet<EventId> = self.work.iter().map(|(id, _)| *id).collect();
        let mut counts: BTreeMap<EventId, usize> = BTreeMap::new();
        for stored in self.fetch.store.values() {
            if !stored.valid || !self.fetch.returned_by(stored, Step::Attention) {
                continue;
            }
            for id in evidence::e_ids(&stored.event)
                .into_iter()
                .collect::<BTreeSet<_>>()
            {
                if work.contains(&id) {
                    *counts.entry(id).or_default() += 1;
                }
            }
        }
        counts
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
