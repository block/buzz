//! Judgments shared by every NIP-FP read: repository authority at the time
//! of a claim, ref-state corroboration of a merge, and the relationship
//! between two signers. Pure functions of fetched, verified events.

use std::collections::{BTreeMap, BTreeSet};

use nostr::{Event, PublicKey};

use super::evidence::{self, Coord};
use super::fetch::{Fetch, Step, Stored};
use super::{Authority, AuthorityStatus as A, Bucket, Independence, RefCheck, RefStatus};

/// Verified events and the owner links learned from them.
pub(super) struct Judge<'a> {
    pub(super) fetch: &'a Fetch,
    pub(super) owners: BTreeMap<PublicKey, BTreeSet<PublicKey>>,
}

impl<'a> Judge<'a> {
    /// The latest valid `30617` observed for `coord` at `origin`.
    pub(super) fn announcement(&self, coord: &Coord, origin: &str) -> Option<&'a Event> {
        self.fetch
            .store
            .values()
            .filter(|s| s.valid && s.origin == origin && self.fetch.returned_by(s, Step::Repo))
            .map(|s| &s.event)
            .filter(|e| e.pubkey == coord.owner && evidence::tag_value(e, "d") == Some(&coord.d))
            .max_by_key(|e| e.created_at)
    }

    /// Maintainers in force at `at`, or `None` when the set at the time is
    /// unknown.
    pub(super) fn maintainers_at(
        &self,
        coord: &Coord,
        origin: &str,
        at: u64,
    ) -> Option<Vec<PublicKey>> {
        self.announcement(coord, origin)
            .filter(|a| a.created_at.as_secs() <= at)
            .map(evidence::maintainers)
    }

    pub(super) fn repo_authority(&self, event: &Event, target: &Event) -> Authority {
        let Some(coord) = evidence::repo(target) else {
            return authority(
                A::SelfAsserted,
                "target names no repository; no signed authority",
            );
        };
        if event.pubkey == coord.owner {
            return authority(A::Authorized, &format!("owner of {coord}"));
        }
        let origin = self.origin_of(target);
        let at = event.created_at.as_secs();
        let in_force = self.maintainers_at(&coord, &origin, at);
        if in_force.as_ref().is_some_and(|m| m.contains(&event.pubkey)) {
            return authority(
                A::Authorized,
                &format!("maintainer of {coord} in force at the time"),
            );
        }
        let owners = self.owners.get(&event.pubkey);
        if owners.is_some_and(|o| o.contains(&coord.owner)) {
            return authority(
                A::Delegated,
                &format!("owner link names the owner of {coord}"),
            );
        }
        if let Some(m) = in_force
            .as_ref()
            .and_then(|m| m.iter().find(|k| owners.is_some_and(|o| o.contains(k))))
        {
            return authority(
                A::Delegated,
                &format!("owner link names {m}, a maintainer of {coord} in force"),
            );
        }
        match (in_force, self.announcement(&coord, &origin)) {
            (None, Some(a)) => authority(A::Unknown, &format!(
                "maintainer set at the time unknown: the latest 30617 ({}) was signed after this status",
                a.id
            )),
            (None, None) => authority(A::Unknown, "maintainer set at the time unknown: no 30617 observed"),
            (Some(_), _) => authority(A::SelfAsserted, &format!(
                "not the owner or a maintainer of {coord} in force; community push policy, if any, is not checkable from events"
            )),
        }
    }

    pub(super) fn origin_of(&self, event: &Event) -> String {
        self.fetch
            .store
            .get(&event.id)
            .map(|s| s.origin.clone())
            .unwrap_or_default()
    }

    pub(super) fn ref_check(&self, event: &Event, target: Option<&Event>) -> RefCheck {
        let unverified = |basis: &str| RefCheck {
            status: RefStatus::Unverified,
            basis: basis.into(),
        };
        let (Some(target), Some(commit)) = (target, evidence::tag_value(event, "merge-commit"))
        else {
            return unverified("no merge-commit tag");
        };
        let Some(coord) = evidence::repo(target) else {
            return unverified("target names no repository");
        };
        let origin = self.origin_of(target);
        let at = event.created_at.as_secs();
        let best = self
            .fetch
            .store
            .values()
            .filter(|s| s.valid && s.origin == origin && self.fetch.returned_by(s, Step::RefState))
            .map(|s| &s.event)
            .filter(|e| evidence::tag_value(e, "d") == Some(&coord.d))
            .filter(|e| evidence::head_commit(e) == Some(commit))
            .filter_map(|e| self.ref_signer(e, &coord, &origin, at).map(|who| (e, who)))
            .min_by_key(|(e, _)| e.created_at.as_secs().abs_diff(at));
        match best {
            Some((state, who)) => {
                let ts = state.created_at.as_secs();
                let when = if ts <= at { "before" } else { "after" };
                RefCheck {
                    status: RefStatus::Corroborated,
                    basis: format!(
                        "30618 {} signed by {who} {} shows HEAD at {commit}, {}s {when} the status; a ref state does not show who authorized the merge",
                        state.id, state.pubkey, ts.abs_diff(at)
                    ),
                }
            }
            None => unverified(&format!("no trusted 30618 observed with HEAD at {commit}")),
        }
    }

    /// Why a `30618` signer is trusted for `coord`, if it is.
    pub(super) fn ref_signer(
        &self,
        state: &Event,
        coord: &Coord,
        origin: &str,
        at: u64,
    ) -> Option<&'static str> {
        if self.fetch.relay_self.get(origin) == Some(&state.pubkey) {
            Some("the origin's NIP-11 self")
        } else if state.pubkey == coord.owner {
            Some("the repository owner")
        } else if self
            .maintainers_at(coord, origin, at)
            .is_some_and(|m| m.contains(&state.pubkey))
        {
            Some("a maintainer in force")
        } else {
            None
        }
    }

    pub(super) fn ref_bucket(&self, stored: &Stored) -> (Bucket, String) {
        let state = &stored.event;
        let at = state.created_at.as_secs();
        let d = evidence::tag_value(state, "d").unwrap_or_default();
        let relay = (self.fetch.relay_self.get(&stored.origin) == Some(&state.pubkey))
            .then_some("the origin's NIP-11 self");
        let trusted = || {
            self.fetch
                .store
                .values()
                .filter(|s| {
                    s.valid && s.origin == stored.origin && self.fetch.returned_by(s, Step::Repo)
                })
                .filter(|s| evidence::tag_value(&s.event, "d") == Some(d))
                .map(|s| Coord {
                    owner: s.event.pubkey,
                    d: d.to_owned(),
                })
                .find_map(|coord| self.ref_signer(state, &coord, &stored.origin, at))
        };
        match relay.or_else(trusted) {
            Some(who) => (Bucket::Verifiable, format!("ref state signed by {who}")),
            None => (
                Bucket::Unknown,
                "signer is not the repository owner, a maintainer in force, or the origin's NIP-11 self".into(),
            ),
        }
    }

    /// The relationship between `signer` and `author`. Never stronger than
    /// "unknown": no signed evidence shows two keys are distinct people.
    pub(super) fn relationship(&self, signer: &PublicKey, author: &PublicKey) -> Independence {
        let empty = BTreeSet::new();
        let of_signer = self.owners.get(signer).unwrap_or(&empty);
        let of_author = self.owners.get(author).unwrap_or(&empty);
        if signer == author {
            Independence::SameSigner
        } else if of_signer.contains(author)
            || of_author.contains(signer)
            || !of_signer.is_disjoint(of_author)
        {
            Independence::SameOwner
        } else {
            Independence::Unknown
        }
    }
}

pub(super) fn authority(status: A, basis: &str) -> Authority {
    Authority {
        status,
        basis: basis.to_owned(),
    }
}
