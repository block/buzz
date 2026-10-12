//! The sans-IO local work trail (policy `buzz.work/v1`).
//!
//! A trail starts from a request, a contribution, or an actor, inside a
//! declared scope, and reads request → contribution(s) → outcome(s) from
//! signed events. It reuses the NIP-FP fetch, verification, authority, and
//! relationship checks; it needs no fingerprint.
//!
//! ```text
//! Trail::new(entry, scope) → loop { next_queries → fetch → ingest } → finish → WorkTrail
//! ```

use std::collections::{BTreeMap, BTreeSet};

use nostr::{Alphabet, Event, EventId, Filter, Kind, PublicKey, SingleLetterTag, Timestamp};

use super::assess::wording;
use super::evidence::{self, Coord, KEYED_OUTCOME_KINDS, OUTCOME_KINDS, STATUS_KINDS};
use super::fetch::{Fetch, Step, Stored};
use super::judge::{authority, Judge};
use super::work::{self, Message, Part, WorkRole};
use super::{
    AuthorityStatus as A, Contribution, ContributionRole, Gap, Independence, Outcome, OutcomeScope,
    OwnerLink, PolicyRef, Query, Response, Source, Task, TaskRequest, TrailScope, WorkTrail,
};
use crate::SdkError;

/// Name of the work-trail policy.
pub const TRAIL_POLICY: &str = "buzz.work/v1";

/// Rounds of exact-id follow-up fetches before the rest are reported.
const MAX_CONTEXT_ROUNDS: usize = 8;

/// Patch, pull request, and pull-request update.
const CODE_KINDS: [u16; 3] = [1617, 1618, 1619];

/// Where a work trail starts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Entry {
    /// A typed request, or a PR or patch.
    Request(EventId),
    /// One contribution.
    Contribution(EventId),
    /// Work requested or contributed by one key in the declared scope.
    Actor(PublicKey),
}

/// What the viewer declares it may read and asks about.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Scope {
    /// The origin to read.
    pub origin: String,
    /// Channels to search for an actor's typed work.
    pub channels: Vec<String>,
    /// Repository coordinates (`30617:<owner>:<d>`) to search for an
    /// actor's code work.
    pub repos: Vec<String>,
    /// Lower bound of every discovery read, in Unix seconds.
    pub since: Option<u64>,
    /// Upper bound (`until`) of every discovery read, in Unix seconds.
    pub until: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    Seed,
    Linked,
    Outcomes,
    Repo,
    Done,
}

/// Reads one work trail. Sans-IO: call [`Trail::next_queries`], answer each
/// [`Query`] with [`Trail::ingest`], and repeat until no queries come back;
/// then call [`Trail::finish`].
pub struct Trail {
    entry: Entry,
    scope: Scope,
    fetch: Fetch,
    phase: Phase,
    asked: BTreeSet<EventId>,
    rounds: usize,
    gaps: Vec<Gap>,
}

impl Trail {
    /// Start a trail read.
    pub fn new(entry: Entry, scope: Scope) -> Self {
        Self {
            entry,
            scope,
            fetch: Fetch::default(),
            phase: Phase::Seed,
            asked: BTreeSet::new(),
            rounds: 0,
            gaps: Vec::new(),
        }
    }

    /// The queries for the next round; empty when the read is done.
    ///
    /// Queries of the previous round left unanswered are recorded as failed.
    pub fn next_queries(&mut self) -> Vec<Query> {
        let pages = self.fetch.next_pages();
        if !pages.is_empty() {
            return pages;
        }
        loop {
            if let Some(query) = self.context() {
                return vec![query];
            }
            let queries = match self.phase {
                Phase::Seed => {
                    self.phase = Phase::Linked;
                    self.seed()
                }
                Phase::Linked => {
                    self.phase = Phase::Outcomes;
                    self.linked()
                }
                Phase::Outcomes => {
                    self.phase = Phase::Repo;
                    self.outcomes()
                }
                Phase::Repo => {
                    self.phase = Phase::Done;
                    self.repos()
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
    /// # Errors
    ///
    /// Returns [`SdkError::InvalidInput`] for an unknown or already answered
    /// scope.
    pub fn ingest(&mut self, scope: usize, response: Response) -> Result<(), SdkError> {
        self.fetch.ingest(scope, response)
    }

    fn bounded(&self, kinds: &[u16]) -> Filter {
        let filter = Filter::new()
            .kinds(kinds.iter().map(|k| Kind::from(*k)))
            .until(Timestamp::from(self.scope.until));
        match self.scope.since {
            Some(since) => filter.since(Timestamp::from(since)),
            None => filter,
        }
    }

    fn walk(&mut self, step: Step, filter: Filter) -> Query {
        let origin = self.scope.origin.clone();
        self.fetch.walk(&origin, step, filter)
    }

    fn seed(&mut self) -> Vec<Query> {
        let origin = self.scope.origin.clone();
        let mut queries = vec![self.fetch.relay_info(&origin, Step::RelayInfo)];
        match self.entry {
            Entry::Request(id) | Entry::Contribution(id) => {
                self.asked.insert(id);
                queries.extend(self.fetch.ids(&origin, Step::Seed, BTreeSet::from([id])));
            }
            Entry::Actor(actor) => {
                if self.scope.channels.is_empty() {
                    self.gap(
                        None,
                        "no channels declared; typed work by the actor was not searched",
                    );
                } else {
                    let typed = self
                        .bounded(&[9])
                        .author(actor)
                        .custom_tags(lower(Alphabet::H), self.scope.channels.clone())
                        .custom_tags(lower(Alphabet::T), WorkRole::ALL.map(WorkRole::tag));
                    queries.push(self.walk(Step::Seed, typed));
                }
                if self.scope.repos.is_empty() {
                    self.gap(
                        None,
                        "no repositories declared; code work by the actor was not searched",
                    );
                } else {
                    let code = self
                        .bounded(&CODE_KINDS)
                        .author(actor)
                        .custom_tags(lower(Alphabet::A), self.scope.repos.clone());
                    queries.push(self.walk(Step::Seed, code));
                }
                let keyed = self
                    .bounded(&KEYED_OUTCOME_KINDS)
                    .custom_tags(lower(Alphabet::P), [actor.to_hex()]);
                queries.push(self.walk(Step::BySubject, keyed));
            }
        }
        queries
    }

    /// Replies to typed requests, and updates of pull requests.
    fn linked(&mut self) -> Vec<Query> {
        let requests: Vec<String> = self
            .valid()
            .filter(|e| matches!(work::read(e), Message::Request(_)))
            .map(|e| e.id.to_hex())
            .collect();
        let roots: Vec<String> = self
            .valid()
            .filter(|e| e.kind.as_u16() == 1618)
            .map(|e| e.id.to_hex())
            .collect();
        let mut queries = Vec::new();
        if !requests.is_empty() {
            let replies = self.bounded(&[9]).custom_tags(lower(Alphabet::E), requests);
            queries.push(self.walk(Step::Linked, replies));
        }
        if !roots.is_empty() {
            let updates = self
                .bounded(&[1619])
                .custom_tags(SingleLetterTag::uppercase(Alphabet::E), roots);
            queries.push(self.walk(Step::Linked, updates));
        }
        queries
    }

    /// Outcomes on every contribution and PR.
    fn outcomes(&mut self) -> Vec<Query> {
        let ids: Vec<String> = self
            .valid()
            .filter(|e| is_contribution(e))
            .map(|e| e.id.to_hex())
            .collect();
        if ids.is_empty() {
            return Vec::new();
        }
        let filter = self
            .bounded(&OUTCOME_KINDS)
            .custom_tags(lower(Alphabet::E), ids);
        vec![self.walk(Step::Outcomes, filter)]
    }

    /// Repository authority and ref state for every repository touched.
    fn repos(&mut self) -> Vec<Query> {
        let coords: BTreeSet<Coord> = self
            .valid()
            .filter(|e| CODE_KINDS.contains(&e.kind.as_u16()))
            .filter_map(evidence::repo)
            .collect();
        let mut queries = Vec::new();
        for coord in coords {
            let announcement = Filter::new()
                .kind(Kind::from(30617))
                .author(coord.owner)
                .identifier(coord.d.clone());
            let state = self.bounded(&[30618]).identifier(coord.d);
            queries.push(self.walk(Step::Repo, announcement));
            queries.push(self.walk(Step::RefState, state));
        }
        queries
    }

    /// Fetch, by exact id, events the trail references but has not seen:
    /// requests, original PRs, previous submissions, and outcome targets.
    fn context(&mut self) -> Option<Query> {
        let missing: BTreeSet<EventId> = self
            .references()
            .into_iter()
            .filter(|id| !self.fetch.store.contains_key(id) && !self.asked.contains(id))
            .collect();
        if missing.is_empty() {
            return None;
        }
        if self.rounds >= MAX_CONTEXT_ROUNDS {
            for id in &missing {
                self.gap(
                    Some(*id),
                    &format!(
                        "not fetched: {MAX_CONTEXT_ROUNDS}-round follow-up budget reached; unknown"
                    ),
                );
            }
            self.asked.extend(missing);
            return None;
        }
        self.rounds += 1;
        self.asked.extend(missing.iter().copied());
        let origin = self.scope.origin.clone();
        self.fetch.ids(&origin, Step::Targets, missing)
    }

    fn references(&self) -> BTreeSet<EventId> {
        let mut out = BTreeSet::new();
        for stored in self.fetch.store.values().filter(|s| s.valid) {
            let event = &stored.event;
            match event.kind.as_u16() {
                9 => {
                    if let Message::Part(part) = work::read(event) {
                        out.insert(part.request);
                        out.extend(part.previous);
                    }
                }
                1619 => out.extend(single_id(event, "E")),
                1617 => out.extend(root_e(event)),
                _ if self.found_as_outcome(stored) => match work::read_label(event) {
                    Some(Ok(label)) => {
                        out.insert(label.target);
                    }
                    Some(Err(_)) => {}
                    None if STATUS_KINDS.contains(&event.kind.as_u16()) => {
                        out.extend(evidence::e_ids(event));
                    }
                    None => {}
                },
                _ => {}
            }
        }
        out
    }

    fn valid(&self) -> impl Iterator<Item = &Event> {
        self.fetch
            .store
            .values()
            .filter(|s| s.valid)
            .map(|s| &s.event)
    }

    fn found_as_outcome(&self, stored: &Stored) -> bool {
        stored
            .scopes
            .iter()
            .any(|s| self.fetch.step(*s).is_some_and(Step::finds_outcomes))
            && OUTCOME_KINDS.contains(&stored.event.kind.as_u16())
    }

    fn gap(&mut self, id: Option<EventId>, reason: &str) {
        self.gaps.push(Gap {
            id: id.map(|id| id.to_hex()),
            reason: reason.to_owned(),
        });
    }

    /// Finish the read and produce the work trail.
    pub fn finish(self) -> WorkTrail {
        let judge = Judge {
            fetch: &self.fetch,
            owners: self.fetch.owners(|_| true),
        };
        let mut reduce = Reduce {
            trail: &self,
            judge,
            tasks: BTreeMap::new(),
            index: BTreeMap::new(),
            gaps: self.gaps.clone(),
        };
        reduce.invalid();
        reduce.contributions();
        reduce.outcomes();
        reduce.conversation();
        reduce.entry();
        let Reduce { tasks, gaps, .. } = reduce;
        let mut tasks: Vec<Task> = tasks.into_values().map(TaskBuild::done).collect();
        tasks.sort_by_key(|t| {
            (
                t.request
                    .created_at
                    .or(t.contributions.first().map(|c| c.created_at)),
                t.request.id.clone(),
                t.contributions.first().map(|c| c.id.clone()),
            )
        });
        WorkTrail {
            policy: PolicyRef {
                name: TRAIL_POLICY.into(),
                version: 1,
            },
            entry: match self.entry {
                Entry::Request(id) => format!("request {id}"),
                Entry::Contribution(id) => format!("contribution {id}"),
                Entry::Actor(pk) => format!("actor {pk}"),
            },
            scope: TrailScope {
                origin: self.scope.origin.clone(),
                channels: self.scope.channels.clone(),
                repos: self.scope.repos.clone(),
                since: self.scope.since,
                cutoff: self.scope.until,
            },
            tasks,
            gaps,
            coverage: self.fetch.coverage.iter().map(wording).collect(),
            records: self.fetch.coverage.clone(),
        }
    }
}

/// A task while it is being assembled.
struct TaskBuild<'a> {
    task: Task,
    /// The verified requester of a typed request.
    requester: Option<PublicKey>,
    /// The original PR, or the root patch of a series.
    root: Option<&'a Event>,
    series: bool,
}

impl TaskBuild<'_> {
    fn done(mut self) -> Task {
        let by_time = |a: &Outcome, b: &Outcome| (a.created_at, &a.id).cmp(&(b.created_at, &b.id));
        self.task
            .contributions
            .sort_by(|a, b| (a.created_at, &a.id).cmp(&(b.created_at, &b.id)));
        for c in &mut self.task.contributions {
            c.outcomes.sort_by(by_time);
        }
        self.task.unbound.sort_by(by_time);
        self.task
            .conversation
            .sort_by(|a, b| (a.created_at, &a.id).cmp(&(b.created_at, &b.id)));
        self.task
    }
}

/// Where a contribution sits, and what outcomes need to know about it.
struct Placed<'a> {
    task: EventId,
    at: usize,
    event: &'a Event,
    role: ContributionRole,
    resolved: bool,
}

struct Reduce<'a> {
    trail: &'a Trail,
    judge: Judge<'a>,
    tasks: BTreeMap<EventId, TaskBuild<'a>>,
    index: BTreeMap<EventId, Placed<'a>>,
    gaps: Vec<Gap>,
}

impl<'a> Reduce<'a> {
    fn store(&self) -> impl Iterator<Item = &'a Stored> {
        self.trail.fetch.store.values()
    }

    fn valid(&self, id: &EventId) -> Option<&'a Event> {
        self.trail.fetch.valid(id)
    }

    fn gap(&mut self, id: EventId, reason: String) {
        self.gaps.push(Gap {
            id: Some(id.to_hex()),
            reason,
        });
    }

    /// Events whose bytes fail verification support nothing.
    fn invalid(&mut self) {
        let invalid: Vec<EventId> = self
            .store()
            .filter(|s| !s.valid)
            .map(|s| s.event.id)
            .collect();
        for id in invalid {
            self.gap(
                id,
                "id or signature does not verify; supports nothing".into(),
            );
        }
    }

    fn task(&mut self, key: EventId, code: bool) -> &mut TaskBuild<'a> {
        let request = (!code).then(|| self.valid(&key)).flatten();
        let fetched = self.trail.fetch.store.contains_key(&key);
        let root = code.then(|| self.valid(&key)).flatten();
        self.tasks.entry(key).or_insert_with(|| {
            let mut req = TaskRequest {
                id: None,
                requester: None,
                created_at: None,
                title: None,
                task_type: None,
                acceptance_criteria: Vec::new(),
                status: "request not linked".into(),
            };
            let mut requester = None;
            if !code {
                req.id = Some(key.to_hex());
                req.status = match request.map(|e| (e, work::read(e))) {
                    Some((e, Message::Request(body))) => {
                        requester = Some(e.pubkey);
                        req.requester = Some(e.pubkey.to_hex());
                        req.created_at = Some(e.created_at.as_secs());
                        req.title = Some(body.title);
                        req.task_type = Some(body.task_type);
                        req.acceptance_criteria = body.acceptance_criteria;
                        "typed request; its signer is the requester".into()
                    }
                    Some((_, Message::Malformed(reason))) => {
                        format!("not a typed request ({reason}); requester unknown")
                    }
                    Some(_) => "not a typed request; requester unknown".into(),
                    None if fetched => "request does not verify; requester unknown".into(),
                    None => "request unavailable to this viewer; requester unknown".into(),
                };
            }
            TaskBuild {
                task: Task {
                    request: req,
                    contributions: Vec::new(),
                    unbound: Vec::new(),
                    conversation: Vec::new(),
                },
                requester,
                root,
                series: false,
            }
        })
    }

    fn place(&mut self, key: EventId, code: bool, event: &'a Event, mut c: Contribution) {
        let task = self.task(key, code);
        if c.unresolved.is_none() {
            if let (
                Some(requester),
                ContributionRole::Commitment
                | ContributionRole::Update
                | ContributionRole::Submission,
            ) = (task.requester, c.role)
            {
                let names = evidence::tags(event, "p")
                    .filter_map(|p| p.get(1))
                    .any(|pk| *pk == requester.to_hex());
                if !names {
                    c.unresolved = Some("its p tags do not name the requester".into());
                }
            }
        }
        let role = c.role;
        let resolved = c.unresolved.is_none();
        task.task.contributions.push(c);
        let at = task.task.contributions.len() - 1;
        self.index.insert(
            event.id,
            Placed {
                task: key,
                at,
                event,
                role,
                resolved,
            },
        );
    }

    fn contributions(&mut self) {
        let events: Vec<&'a Event> = self.store().filter(|s| s.valid).map(|s| &s.event).collect();
        for event in events {
            match event.kind.as_u16() {
                9 => match work::read(event) {
                    Message::Part(part) => {
                        let c = self.part(event, &part);
                        self.place(part.request, false, event, c);
                    }
                    Message::Request(_) => {
                        self.task(event.id, false);
                    }
                    Message::Malformed(reason) if !replies_to_request(event) => {
                        self.gap(
                            event.id,
                            format!(
                                "tries to be typed work but is not ({reason}); not read as work"
                            ),
                        );
                    }
                    _ => {}
                },
                1618 => {
                    let unresolved = match (count(event, "c"), count(event, "a")) {
                        (1, 1) => None,
                        _ => Some(
                            "needs exactly one c tip commit and one a repository; revision unknown"
                                .into(),
                        ),
                    };
                    let c = code(event, ContributionRole::Proposal, unresolved, None);
                    self.place(event.id, true, event, c);
                }
                1619 => match single_id(event, "E") {
                    Some(root) => {
                        let unresolved = self.update_link(event, root).err();
                        let c = code(
                            event,
                            ContributionRole::ProposalUpdate,
                            unresolved,
                            Some(root),
                        );
                        self.place(root, true, event, c);
                    }
                    None => self.gap(
                        event.id,
                        "pull-request update names no single original PR (E); unresolved".into(),
                    ),
                },
                1617 => {
                    let root = root_e(event).unwrap_or(event.id);
                    let c = code(
                        event,
                        ContributionRole::Patch,
                        Some("patch revisions are series-scoped; revision unknown".into()),
                        (root != event.id).then_some(root),
                    );
                    self.place(root, true, event, c);
                    if let Some(task) = self.tasks.get_mut(&root) {
                        task.series = true;
                    }
                }
                _ => {}
            }
        }
    }

    fn part(&mut self, event: &'a Event, part: &Part) -> Contribution {
        let role = match part.role {
            WorkRole::Commitment => ContributionRole::Commitment,
            WorkRole::Update => ContributionRole::Update,
            _ => ContributionRole::Submission,
        };
        if let Some(previous) = part.previous {
            let same = self.valid(&previous).is_some_and(|p| {
                matches!(work::read(p), Message::Part(q)
                    if q.role == WorkRole::Submission && q.request == part.request)
            });
            if !same {
                self.gap(event.id, format!(
                    "q names {previous}, which is not a verified submission to the same request; revision link unresolved"
                ));
            }
        }
        Contribution {
            id: event.id.to_hex(),
            kind: 9,
            role,
            signer: event.pubkey.to_hex(),
            owner_link: evidence::owner_link(event),
            created_at: event.created_at.as_secs(),
            summary: Some(part.summary.clone()),
            state: part.state.clone(),
            artifact: part.artifact.clone(),
            revision: part.artifact_revision.clone(),
            previous: part.previous.map(|id| id.to_hex()),
            unresolved: None,
            outcomes: Vec::new(),
        }
    }

    /// Validate a `1619` against its original PR (engineering plan §3, code
    /// revision binding). The error says why it stays unresolved.
    fn update_link(&self, update: &Event, root: EventId) -> Result<(), String> {
        for name in ["E", "P", "a", "c"] {
            if count(update, name) != 1 {
                return Err(format!("needs exactly one {name} tag; unresolved update"));
            }
        }
        let Some(original) = self.valid(&root).filter(|e| e.kind.as_u16() == 1618) else {
            return Err(format!(
                "original PR {root} unavailable or not a verified 1618; unresolved update"
            ));
        };
        if evidence::tag_value(update, "P") != Some(original.pubkey.to_hex().as_str()) {
            return Err("P does not name the original PR's signer; unresolved update".into());
        }
        if count(original, "a") != 1
            || evidence::tag_value(update, "a") != evidence::tag_value(original, "a")
        {
            return Err(
                "names a different repository than the original PR; unresolved update".into(),
            );
        }
        let origin = |e: &Event| self.trail.fetch.store.get(&e.id).map(|s| s.origin.as_str());
        if origin(update) != origin(original) {
            return Err(
                "fetched from a different origin than the original PR; unresolved update".into(),
            );
        }
        if update.pubkey != original.pubkey {
            return Err("signed by a key other than the original PR's signer; unresolved update (delegation is not supported)".into());
        }
        Ok(())
    }

    fn outcomes(&mut self) {
        let candidates: Vec<&'a Event> = self
            .store()
            .filter(|s| s.valid && self.trail.found_as_outcome(s))
            .map(|s| &s.event)
            .collect();
        for event in candidates {
            match event.kind.as_u16() {
                1985 => self.label(event),
                5 => self.retraction(event),
                _ => self.status(event),
            }
        }
    }

    fn attach(&mut self, target: EventId, outcome: Outcome) {
        if let Some(placed) = self.index.get(&target) {
            let (task, at) = (placed.task, placed.at);
            if let Some(t) = self.tasks.get_mut(&task) {
                t.task.contributions[at].outcomes.push(outcome);
            }
        }
    }

    fn label(&mut self, event: &'a Event) {
        let label = match work::read_label(event) {
            None => return,
            Some(Err(reason)) => {
                return self.gap(event.id, format!("label not interpreted: {reason}"));
            }
            Some(Ok(label)) => label,
        };
        let Some(placed) = self.index.get(&label.target) else {
            let why = if self.trail.fetch.store.contains_key(&label.target) {
                "is not a contribution in this trail; not interpreted"
            } else {
                "was not served; unknown"
            };
            return self.gap(
                event.id,
                format!("label judges {}, which {why}", label.target),
            );
        };
        let target = placed.event;
        let problem = if !placed.resolved {
            Some("judges an unresolved contribution")
        } else if matches!(
            placed.role,
            ContributionRole::Commitment | ContributionRole::Update
        ) {
            Some("judges a commitment or update; outcomes judge a submitted revision")
        } else if label.subject != target.pubkey {
            Some("p does not name the contribution's signer")
        } else if evidence::tag_value(target, "h") != Some(label.channel.as_str())
            || count(target, "h") != 1
        {
            Some("h does not match the contribution's channel")
        } else {
            None
        };
        if let Some(problem) = problem {
            return self.gap(event.id, format!("label not interpreted: {problem}"));
        }
        let requester = self.tasks.get(&placed.task).and_then(|t| t.requester);
        let request = placed.task;
        let authority = match (label.claim, requester) {
            ("reused", _) => authority(A::Opinion, "anyone may say this; an attributed opinion"),
            (_, Some(r)) if r == event.pubkey => authority(
                A::Authorized,
                &format!("requester: signer of request {request}; for that request only"),
            ),
            (_, Some(r)) => authority(
                A::Opinion,
                &format!("not the requester ({r}); an attributed opinion, not task state"),
            ),
            (_, None) if placed.role == ContributionRole::Submission => authority(
                A::Unknown,
                &format!("requester unknown: request {request} unavailable or untyped"),
            ),
            (_, None) => authority(A::Opinion, "no typed request; an attributed opinion"),
        };
        let outcome = Outcome {
            id: event.id.to_hex(),
            kind: 1985,
            signer: event.pubkey.to_hex(),
            created_at: event.created_at.as_secs(),
            claim: label.claim,
            scope: OutcomeScope::Revision,
            note: None,
            authority,
            relationship: self.judge.relationship(&event.pubkey, &target.pubkey),
            ref_state: None,
            reason: Some(label.reason),
            criteria_checked: label.criteria_checked,
            evidence_refs: label.evidence_refs,
        };
        self.attach(label.target, outcome);
    }

    /// A deletion by a contribution's own signer withdraws it.
    fn retraction(&mut self, event: &'a Event) {
        for id in evidence::e_ids(event) {
            let own = self
                .index
                .get(&id)
                .is_some_and(|p| p.event.pubkey == event.pubkey);
            if !own {
                continue;
            }
            let outcome = Outcome {
                id: event.id.to_hex(),
                kind: 5,
                signer: event.pubkey.to_hex(),
                created_at: event.created_at.as_secs(),
                claim: "retracted",
                scope: OutcomeScope::Revision,
                note: None,
                authority: authority(A::Authorized, "the contribution's signer withdraws it"),
                relationship: Independence::SameSigner,
                ref_state: None,
                reason: None,
                criteria_checked: Vec::new(),
                evidence_refs: Vec::new(),
            };
            self.attach(id, outcome);
        }
    }

    fn status(&mut self, event: &'a Event) {
        let Some(key) = evidence::target(event) else {
            return;
        };
        let Some(task) = self
            .tasks
            .get(&key)
            .filter(|t| t.root.is_some() || t.series || t.task.request.id.is_none())
        else {
            return self.gap(event.id, format!(
                "status names {key}, which is not a PR or patch series in this trail; not interpreted"
            ));
        };
        let (root, series) = (task.root, task.series);
        let claim = match event.kind.as_u16() {
            1630 => "open",
            1631 if series => "applied",
            1631 => "merged",
            1632 => "closed",
            _ => "draft",
        };
        let authority = match root {
            None => authority(A::Unknown, "the PR or series root is unavailable"),
            Some(r) if claim != "merged" && r.pubkey == event.pubkey => {
                authority(A::Authorized, "signer of the PR or series")
            }
            Some(r) => self.judge.repo_authority(event, r),
        };
        let ref_state = (event.kind.as_u16() == 1631).then(|| self.judge.ref_check(event, root));
        let (bound, scope, note) = if series {
            (
                None,
                OutcomeScope::Series,
                Some(format!("series-level {claim} claim; revision unknown")),
            )
        } else {
            match self.revision(event, key, root) {
                Ok(revision) => (Some(revision), OutcomeScope::Revision, None),
                Err(why) => {
                    let what = if claim == "merged" { "merge" } else { claim };
                    (
                        None,
                        OutcomeScope::PullRequest,
                        Some(format!(
                            "PR-level {what} claim; judged revision unknown{why}"
                        )),
                    )
                }
            }
        };
        let judged = bound
            .and_then(|id| self.index.get(&id))
            .map(|p| p.event.pubkey)
            .or(root.map(|r| r.pubkey));
        let outcome = Outcome {
            id: event.id.to_hex(),
            kind: event.kind.as_u16(),
            signer: event.pubkey.to_hex(),
            created_at: event.created_at.as_secs(),
            claim,
            scope,
            note,
            authority,
            relationship: judged.map_or(Independence::Unknown, |pk| {
                self.judge.relationship(&event.pubkey, &pk)
            }),
            ref_state,
            reason: None,
            criteria_checked: Vec::new(),
            evidence_refs: Vec::new(),
        };
        match bound {
            Some(id) => self.attach(id, outcome),
            None => {
                if let Some(t) = self.tasks.get_mut(&key) {
                    t.task.unbound.push(outcome);
                }
            }
        }
    }

    /// The exact revision a status judges: exactly one `e` reply naming the
    /// original PR or a validated update of it. Never the newest tip. The
    /// error is a suffix explaining why the revision is unknown.
    fn revision(
        &self,
        event: &Event,
        root: EventId,
        original: Option<&Event>,
    ) -> Result<EventId, String> {
        let replies: Vec<EventId> = evidence::tags(event, "e")
            .filter(|p| p.get(3).is_some_and(|m| m == "reply"))
            .filter_map(|p| p.get(1).and_then(|id| EventId::from_hex(id).ok()))
            .collect();
        let revision = match replies.as_slice() {
            [] => return Err(String::new()),
            [one] => *one,
            _ => return Err("; names more than one revision".into()),
        };
        if let (Some(original), Some(_)) = (original, evidence::tag_value(event, "a")) {
            let names = evidence::tags(event, "a")
                .any(|p| p.get(1).map(String::as_str) == evidence::tag_value(original, "a"));
            if !names {
                return Err("; names a different repository".into());
            }
        }
        match self.index.get(&revision) {
            Some(p) if p.task == root && p.resolved => Ok(revision),
            Some(_) => Err(format!(
                "; names {revision}, which is not a validated revision of this PR"
            )),
            None => Err(format!("; names {revision}, which is unavailable")),
        }
    }

    /// Untyped and malformed replies to requests: conversation, with links.
    fn conversation(&mut self) {
        let replies: Vec<&'a Event> = self
            .store()
            .filter(|s| s.valid && s.event.kind.as_u16() == 9)
            .map(|s| &s.event)
            .collect();
        for event in replies {
            let note = match work::read(event) {
                Message::Plain => "conversation; not read as work or acceptance".to_owned(),
                Message::Malformed(reason) => {
                    format!("tries to be typed work but is not ({reason}); conversation, not work")
                }
                _ => continue,
            };
            for id in evidence::e_ids(event).into_iter().collect::<BTreeSet<_>>() {
                if let Some(task) = self
                    .tasks
                    .get_mut(&id)
                    .filter(|t| t.task.request.id.is_some())
                {
                    task.task.conversation.push(Source {
                        id: event.id.to_hex(),
                        signer: event.pubkey.to_hex(),
                        created_at: event.created_at.as_secs(),
                        note: note.clone(),
                    });
                }
            }
        }
    }

    fn entry(&mut self) {
        let id = match self.trail.entry {
            Entry::Request(id) | Entry::Contribution(id) => id,
            Entry::Actor(_) => return,
        };
        if !self.trail.fetch.store.contains_key(&id) {
            self.gap(id, "entry not served to this viewer; unknown".into());
        } else if self.valid(&id).is_some()
            && !self.tasks.contains_key(&id)
            && !self.index.contains_key(&id)
        {
            self.gap(
                id,
                format!("entry is not typed work or code work under {TRAIL_POLICY}"),
            );
        }
    }
}

fn code(
    event: &Event,
    role: ContributionRole,
    unresolved: Option<String>,
    previous: Option<EventId>,
) -> Contribution {
    Contribution {
        id: event.id.to_hex(),
        kind: event.kind.as_u16(),
        role,
        signer: event.pubkey.to_hex(),
        owner_link: match evidence::owner_link(event) {
            OwnerLink::Verified(o) => OwnerLink::Verified(o),
            other => other,
        },
        created_at: event.created_at.as_secs(),
        summary: evidence::tag_value(event, "subject").map(str::to_owned),
        state: None,
        artifact: None,
        revision: (count(event, "c") == 1)
            .then(|| evidence::tag_value(event, "c").map(str::to_owned))
            .flatten(),
        previous: previous.map(|id| id.to_hex()),
        unresolved,
        outcomes: Vec::new(),
    }
}

fn is_contribution(event: &Event) -> bool {
    CODE_KINDS.contains(&event.kind.as_u16()) || matches!(work::read(event), Message::Part(_))
}

fn replies_to_request(event: &Event) -> bool {
    evidence::tags(event, "e").next().is_some()
}

fn count(event: &Event, name: &str) -> usize {
    evidence::tags(event, name).count()
}

fn single_id(event: &Event, name: &str) -> Option<EventId> {
    let mut values = evidence::tags(event, name).filter_map(|p| p.get(1));
    match (values.next(), values.next()) {
        (Some(id), None) => EventId::from_hex(id).ok(),
        _ => None,
    }
}

fn root_e(event: &Event) -> Option<EventId> {
    evidence::tags(event, "e")
        .find(|p| p.get(3).is_some_and(|m| m == "root"))
        .and_then(|p| p.get(1))
        .and_then(|id| EventId::from_hex(id).ok())
}

fn lower(letter: Alphabet) -> SingleLetterTag {
    SingleLetterTag::lowercase(letter)
}
