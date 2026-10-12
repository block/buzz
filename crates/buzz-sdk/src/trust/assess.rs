//! Turning observed events into the NIP-FP report (NIP-FP §Verifier report).
//!
//! Authority is judged as of each outcome's own `created_at`, independence is
//! never stronger than "unknown", and nothing is combined into a total.

use std::collections::{BTreeMap, BTreeSet};

use nostr::{Event, EventId, PublicKey};

use super::evidence::{self, CHANGE_KINDS};
use super::fetch::{Step, Stored};
use super::judge::{authority, Judge};
use super::{
    Authority, AuthorityStatus as A, Bucket, CoverageRecord, Gap, Independence, Interpretation,
    Item, Observed, Polarity, PolicyRef, Report, Role, Rung, Verifier,
};

/// Who may make a claim (NIP-FP §Outcome vocabulary).
#[derive(Clone, Copy)]
enum Rule {
    Repo,
    AuthorOrRepo,
    Requester,
    Opinion,
    Author,
}

struct Work<'a> {
    id: EventId,
    selected: bool,
    event: Option<&'a Event>,
}

struct Assessor<'a> {
    v: &'a Verifier,
    j: Judge<'a>,
    attention: BTreeMap<EventId, usize>,
    selected: BTreeSet<EventId>,
    reported: BTreeSet<EventId>,
    items: Vec<Item>,
    gaps: Vec<Gap>,
}

impl Verifier {
    /// Finish verification and produce the three-section report.
    pub fn finish(self) -> Report {
        let mut assessor = Assessor {
            j: Judge {
                fetch: &self.fetch,
                owners: self.owners(),
            },
            attention: self.attention(),
            selected: self.work.iter().map(|(id, _)| *id).collect(),
            reported: BTreeSet::new(),
            items: Vec::new(),
            gaps: self.gaps.clone(),
            v: &self,
        };
        let work = assessor.work_items();
        assessor.requests(&work);
        assessor.outcomes(&work);
        assessor.repo_items();
        let rungs = assessor.rungs(&work);
        let omitted_contrary = assessor
            .items
            .iter()
            .filter(|i| i.bucket == Bucket::Contrary && i.target_selected == Some(false))
            .map(|i| i.id.clone())
            .collect();
        let Assessor { items, gaps, .. } = assessor;
        Report {
            nip: "NIP-FP",
            cutoff: self.cutoff,
            interpretation: Interpretation {
                policy: PolicyRef {
                    name: self.policy.name.clone(),
                    version: self.policy.version,
                },
                rungs,
                omitted_contrary,
                gaps,
                coverage: self.fetch.coverage.iter().map(wording).collect(),
            },
            observed: Observed {
                coverage: self.fetch.coverage.clone(),
                items,
            },
            selection: self.selection.clone(),
        }
    }
}

impl<'a> Assessor<'a> {
    fn stored(&self, id: &EventId) -> Option<&'a Stored> {
        self.v.fetch.store.get(id)
    }

    fn valid(&self, id: &EventId) -> Option<&'a Event> {
        self.stored(id).filter(|s| s.valid).map(|s| &s.event)
    }

    fn returned_by(&self, stored: &Stored, step: Step) -> bool {
        self.v.fetch.returned_by(stored, step)
    }

    fn push(
        &mut self,
        id: EventId,
        origin: Option<String>,
        role: Role,
        bucket: Bucket,
        reason: String,
    ) -> &mut Item {
        let stored = self.stored(&id);
        self.reported.insert(id);
        self.items.push(Item {
            id: id.to_hex(),
            origin: stored.map(|s| s.origin.clone()).or(origin),
            kind: stored.map(|s| s.event.kind.as_u16()),
            pubkey: stored.map(|s| s.event.pubkey.to_hex()),
            created_at: stored.map(|s| s.event.created_at.as_secs()),
            scope: stored.map(|s| s.scopes.clone()).unwrap_or_default(),
            role,
            selected: None,
            target: None,
            target_selected: None,
            claim: None,
            polarity: None,
            authority: None,
            ref_state: None,
            owner_link: stored
                .filter(|s| s.valid)
                .map(|s| evidence::owner_link(&s.event)),
            independence: None,
            attention: None,
            bucket,
            reason,
        });
        let last = self.items.len() - 1;
        &mut self.items[last]
    }

    /// Selected work in fingerprint order, then discovered work by the subject.
    fn work_items(&mut self) -> Vec<Work<'a>> {
        let mut work = Vec::new();
        for (id, origin) in self.v.work.clone() {
            let (bucket, reason) = match self.stored(&id) {
                None => (Bucket::Inaccessible, self.unserved(&origin)),
                Some(s) if !s.valid => (Bucket::Invalid, "id or signature does not verify".into()),
                Some(s) if self.v.attributed(&s.event, &self.j.owners) => {
                    (Bucket::Verifiable, attribution(&s.event, self.v.subject))
                }
                Some(_) => (
                    Bucket::Unknown,
                    "not_attributed: signer is not the subject and has no owner link to it".into(),
                ),
            };
            let attention = self.attention.get(&id).copied().unwrap_or(0);
            let asked = self.selected_scopes(&origin);
            let item = self.push(id, Some(origin), Role::Work, bucket, reason);
            if item.scope.is_empty() {
                item.scope = asked;
            }
            item.selected = Some(true);
            item.attention = Some(attention);
            work.push(Work {
                id,
                selected: true,
                event: self.valid(&id),
            });
        }
        for item in &self.v.selection.items {
            if item.origin.is_none() {
                self.gaps.push(Gap {
                    id: Some(item.id.clone()),
                    reason: "selection names no origin; unresolvable, unknown".into(),
                });
            }
        }
        let discovered: Vec<&'a Stored> = self
            .v
            .fetch
            .store
            .values()
            .filter(|s| s.valid && self.returned_by(s, Step::Targets))
            .filter(|s| !self.selected.contains(&s.event.id))
            .filter(|s| self.v.attributed(&s.event, &self.j.owners))
            .collect();
        for stored in discovered {
            let reason = attribution(&stored.event, self.v.subject);
            let item = self.push(
                stored.event.id,
                None,
                Role::Work,
                Bucket::Verifiable,
                reason,
            );
            item.selected = Some(false);
            work.push(Work {
                id: stored.event.id,
                selected: false,
                event: Some(&stored.event),
            });
        }
        for w in &work {
            if w.event.is_none() && self.stored(&w.id).is_none() {
                self.gaps.push(Gap {
                    id: Some(w.id.to_hex()),
                    reason: "inaccessible; neither passes nor fails".into(),
                });
            }
        }
        work
    }

    /// Records that asked `origin` for the selected events.
    fn selected_records<'b>(&self, origin: &'b str) -> impl Iterator<Item = &'a CoverageRecord> + 'b
    where
        'a: 'b,
    {
        let fetch = &self.v.fetch;
        fetch.coverage.iter().filter(move |record| {
            fetch.step(record.scope) == Some(Step::Selected) && record.origin == origin
        })
    }

    /// Scopes that asked `origin` for the selected events.
    fn selected_scopes(&self, origin: &str) -> Vec<usize> {
        self.selected_records(origin).map(|r| r.scope).collect()
    }

    fn unserved(&self, origin: &str) -> String {
        match self.selected_records(origin).find_map(|r| r.error.clone()) {
            Some(error) => format!("inaccessible: {error}"),
            None => format!("inaccessible: {origin} did not serve it to this viewer"),
        }
    }

    fn requests(&mut self, work: &[Work<'a>]) {
        for w in work {
            let Some(event) = w.event.filter(|e| self.v.attributed(e, &self.j.owners)) else {
                continue;
            };
            let Some(parent) = evidence::parent(event) else {
                continue;
            };
            if self.reported.contains(&parent) {
                continue;
            }
            let (bucket, reason) = match self.stored(&parent) {
                None => (
                    Bucket::Inaccessible,
                    "request not served; requester unknown".into(),
                ),
                Some(s) if !s.valid => (Bucket::Invalid, "id or signature does not verify".into()),
                Some(_) => (
                    Bucket::Verifiable,
                    format!("request answered by {}; its author is the requester", w.id),
                ),
            };
            self.push(parent, None, Role::Request, bucket, reason);
        }
    }

    fn outcomes(&mut self, work: &[Work<'a>]) {
        let by_id: BTreeMap<EventId, &Work<'a>> = work.iter().map(|w| (w.id, w)).collect();
        let candidates: Vec<&'a Stored> = self
            .v
            .fetch
            .store
            .values()
            .filter(|s| self.v.is_outcome_candidate(s))
            .collect();
        for stored in candidates {
            let event = &stored.event;
            if self.reported.contains(&event.id) {
                continue;
            }
            let target = if event.kind.as_u16() == 5 {
                evidence::e_ids(event)
                    .into_iter()
                    .find(|id| by_id.contains_key(id))
            } else {
                evidence::target(event)
            };
            let Some(target) = target else { continue };
            let Some(w) = by_id.get(&target) else {
                if stored.valid && self.stored(&target).is_none() {
                    self.gaps.push(Gap {
                        id: Some(event.id.to_hex()),
                        reason: format!(
                            "outcome targets {target}, which was not served; attribution unknown"
                        ),
                    });
                }
                continue;
            };
            if event.kind.as_u16() == 5 && w.event.is_some_and(|t| t.pubkey != event.pubkey) {
                continue;
            }
            if !stored.valid {
                self.push(
                    event.id,
                    None,
                    Role::Outcome,
                    Bucket::Invalid,
                    "id or signature does not verify".into(),
                );
                continue;
            }
            self.outcome(event, w);
        }
    }

    fn outcome(&mut self, event: &'a Event, w: &Work<'a>) {
        let (claim, polarity, rule) = self.claim(event, w);
        let authority = self.authority(rule, event, w.event);
        let ref_state = (claim == "merged").then(|| self.j.ref_check(event, w.event));
        let independence = match self.v.subject {
            Some(subject) => self.j.relationship(&event.pubkey, &subject),
            None => Independence::Unknown,
        };
        let (bucket, reason) = match (polarity, authority.status) {
            (Polarity::Contrary, status) => (
                Bucket::Contrary,
                format!(
                    "contrary claim '{claim}' ({}): {}",
                    status_name(status),
                    authority.basis
                ),
            ),
            (_, A::Authorized | A::Delegated) => (
                Bucket::Verifiable,
                format!("'{claim}' by a signer with authority: {}", authority.basis),
            ),
            _ => (
                Bucket::Unknown,
                format!("'{claim}' does not check out: {}", authority.basis),
            ),
        };
        let selected = self.selected.contains(&w.id);
        let item = self.push(event.id, None, Role::Outcome, bucket, reason);
        item.target = Some(w.id.to_hex());
        item.target_selected = Some(selected);
        item.claim = Some(claim);
        item.polarity = Some(polarity);
        item.authority = Some(authority);
        item.ref_state = ref_state;
        item.independence = Some(independence);
    }

    fn claim(&self, event: &Event, w: &Work<'_>) -> (&'static str, Polarity, Rule) {
        let target_kind = w.event.map(|t| t.kind.as_u16());
        match event.kind.as_u16() {
            1631 => match target_kind {
                Some(k) if CHANGE_KINDS.contains(&k) => ("merged", Polarity::Positive, Rule::Repo),
                Some(_) => ("resolved", Polarity::Positive, Rule::AuthorOrRepo),
                None => ("applied", Polarity::Positive, Rule::AuthorOrRepo),
            },
            1632 => ("closed", Polarity::Contrary, Rule::AuthorOrRepo),
            1630 if self.closed_before(event, w.id) => {
                ("reopened", Polarity::Contrary, Rule::AuthorOrRepo)
            }
            1630 => ("open", Polarity::Neutral, Rule::AuthorOrRepo),
            1633 => ("draft", Polarity::Neutral, Rule::AuthorOrRepo),
            5 => ("retracted", Polarity::Contrary, Rule::Author),
            _ => match evidence::label(event) {
                Some("accepted") => ("accepted", Polarity::Positive, Rule::Requester),
                Some("reused") => ("reused", Polarity::Positive, Rule::Opinion),
                Some("corrected") => ("corrected", Polarity::Contrary, Rule::Requester),
                Some("rejected") => ("rejected", Polarity::Contrary, Rule::Requester),
                _ => ("disputed", Polarity::Contrary, Rule::Requester),
            },
        }
    }

    /// Whether a valid `1631` or `1632` on `target` precedes `event`.
    fn closed_before(&self, event: &Event, target: EventId) -> bool {
        self.v.fetch.store.values().any(|s| {
            s.valid
                && matches!(s.event.kind.as_u16(), 1631 | 1632)
                && evidence::target(&s.event) == Some(target)
                && s.event.created_at < event.created_at
        })
    }

    fn authority(&self, rule: Rule, event: &Event, target: Option<&Event>) -> Authority {
        let Some(target) = target else {
            return authority(A::Unknown, "target could not be fetched or verified");
        };
        match rule {
            Rule::Opinion => authority(A::Opinion, "anyone may say this; an opinion"),
            Rule::Author => authority(A::Authorized, "author of the work event"),
            Rule::Requester => self.requester(event, target),
            Rule::AuthorOrRepo if event.pubkey == target.pubkey => {
                authority(A::Authorized, "author of the target")
            }
            Rule::AuthorOrRepo | Rule::Repo => self.j.repo_authority(event, target),
        }
    }

    fn requester(&self, event: &Event, target: &Event) -> Authority {
        let Some(parent) = evidence::parent(target) else {
            return authority(A::Opinion, "the work answers no request; an opinion");
        };
        match self.valid(&parent) {
            Some(request) if request.pubkey == event.pubkey => authority(
                A::Authorized,
                &format!("requester: author of request {parent}"),
            ),
            Some(request) => authority(
                A::Opinion,
                &format!("not the requester ({}); an opinion", request.pubkey),
            ),
            None => authority(
                A::Unknown,
                &format!("requester unknown: request {parent} not verifiable"),
            ),
        }
    }

    /// Every `merge-commit` claimed by a reported merge status, valid or not.
    fn claimed_commits(&self) -> BTreeSet<&'a str> {
        self.items
            .iter()
            .filter(|i| i.role == Role::Outcome && i.kind == Some(1631))
            .filter_map(|i| EventId::from_hex(&i.id).ok())
            .filter_map(|id| self.stored(&id))
            .filter_map(|s| evidence::tag_value(&s.event, "merge-commit"))
            .collect()
    }

    /// Repository announcements, and the ref states that bear on a reported
    /// merge. Other ref states are counted in coverage only.
    fn repo_items(&mut self) {
        let commits = self.claimed_commits();
        let repo: Vec<&'a Stored> = self.v.fetch.store.values().collect();
        for stored in repo {
            let id = stored.event.id;
            if self.reported.contains(&id) {
                continue;
            }
            let role = if self.returned_by(stored, Step::Repo) {
                Role::Authority
            } else if self.returned_by(stored, Step::RefState)
                && evidence::head_commit(&stored.event).is_some_and(|c| commits.contains(c))
            {
                Role::Ref
            } else {
                continue;
            };
            let (bucket, reason) = if !stored.valid {
                (
                    Bucket::Invalid,
                    "id or signature does not verify".to_owned(),
                )
            } else if role == Role::Authority {
                (
                    Bucket::Verifiable,
                    "repository announcement signed by its owner".to_owned(),
                )
            } else {
                self.j.ref_bucket(stored)
            };
            self.push(id, None, role, bucket, reason);
        }
    }

    fn rungs(&self, work: &[Work<'a>]) -> Vec<Rung> {
        work.iter()
            .map(|w| {
                let mut rung = Rung {
                    id: w.id.to_hex(),
                    selected: w.selected,
                    rung: None,
                    basis: None,
                    independence: None,
                    viewer_known: None,
                };
                let Some(event) = w.event else { return rung };
                if !self.v.attributed(event, &self.j.owners) {
                    rung.rung = Some(0);
                    return rung;
                }
                rung.rung = Some(1);
                let hex = w.id.to_hex();
                let support = self
                    .items
                    .iter()
                    .filter(|i| i.target.as_deref() == Some(hex.as_str()))
                    .filter(|i| i.polarity == Some(Polarity::Positive))
                    .filter(|i| {
                        i.authority
                            .as_ref()
                            .is_some_and(|a| matches!(a.status, A::Authorized | A::Delegated))
                    })
                    .filter(|i| i.independence == Some(Independence::Unknown))
                    .map(|i| (i, self.viewer_known(i)))
                    .max_by_key(|(i, known)| (*known, std::cmp::Reverse(i.created_at)));
                if let Some((item, known)) = support {
                    rung.rung = Some(2);
                    rung.basis = Some(item.id.clone());
                    rung.independence = item.independence;
                    rung.viewer_known = Some(known);
                }
                rung
            })
            .collect()
    }

    fn viewer_known(&self, item: &Item) -> bool {
        item.pubkey
            .as_deref()
            .and_then(|pk| PublicKey::from_hex(pk).ok())
            .is_some_and(|pk| self.v.policy.viewer_known.contains(&pk))
    }
}

fn status_name(status: A) -> &'static str {
    match status {
        A::Authorized => "authorized",
        A::Delegated => "delegated",
        A::SelfAsserted => "self_asserted",
        A::Opinion => "opinion",
        A::Unknown => "authority unknown",
    }
}

fn attribution(event: &Event, subject: Option<PublicKey>) -> String {
    if Some(event.pubkey) == subject {
        "signed by the subject".into()
    } else {
        "signed by a key whose owner link names the subject".into()
    }
}

/// A coverage record in the wording NIP-FP requires. Never "clean" or
/// "no dispute", and a read that may have missed events says so.
pub(super) fn wording(record: &CoverageRecord) -> String {
    let local = record
        .local
        .as_ref()
        .map(|l| format!(", then matched locally on {l}"))
        .unwrap_or_default();
    let scope = format!(
        "{} {} {}{local}",
        record.scope, record.origin, record.request
    );
    let before = record
        .cutoff
        .map(|t| format!(" before cutoff {t}"))
        .unwrap_or_default();
    let discarded = if record.discarded > 0 {
        format!(
            "; {} discarded for not matching the filter",
            record.discarded
        )
    } else {
        String::new()
    };
    if let Some(error) = &record.error {
        format!("inaccessible: {error} (scope {scope})")
    } else if let Some(reason) = &record.incomplete {
        format!(
            "not exhaustive: {reason}; {} found in scope {scope}{before}{discarded}",
            record.returned
        )
    } else if record.returned == 0 {
        format!("none found in scope {scope}{before}{discarded}")
    } else {
        format!(
            "{} found in scope {scope}{before}{discarded}",
            record.returned
        )
    }
}
