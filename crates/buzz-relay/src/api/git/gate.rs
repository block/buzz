//! The governed decision at the land: `buzz_git_gate::decide` over inputs
//! resolved from the governance heads and evidence, read in the land's own
//! transaction after `hold()`.
//!
//! ```text
//!   floor, trusted, labels:<repo>, rules:<repo>  ─┐
//!   evidence (approvals, checks) for the new tips ─┼─► intern ─► Input ─► decide
//!   pusher's and approvers' facts, read now       ─┘
//! ```
//!
//! The announcement's `buzz-protect` rules are not translated: the caller
//! still evaluates them with the existing evaluator, and a push lands only
//! if both allow it. Anything the gate has no input for is mapped to a
//! requirement that denies, never dropped.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use buzz_core::git_perms::{Denial, RefPattern, RefUpdate, UpdateKind};
use buzz_core::governance::{
    Body, Document, DocumentId, EvidenceBody, LabelDefinition, RepoCoordinate, Requirement,
    Rule as DocRule,
};
use buzz_db::git_evidence::EvidenceRow;
use buzz_db::git_land;
use buzz_db::{AdmittedTx, DbError};
use buzz_git_gate as gate;

use super::land::Authority;

const ZERO_OID: &str = "0000000000000000000000000000000000000000";

/// Why a push whose scan was not clean is refused.
pub(super) const SECRET_DETECTED: &str = "the push brings a detected secret";
pub(super) const SCAN_INCOMPLETE: &str = "the secret scan did not complete";

/// A value and the governance head it came from (`None`: no head).
#[derive(Debug, Clone)]
pub(crate) struct Sourced<T> {
    pub value: T,
    pub head: Option<nostr::EventId>,
}

/// Governance in force for one repository.
#[derive(Debug, Clone)]
pub(crate) struct Policy {
    /// The floor's rules and protected-label schema.
    pub floor: Sourced<(Vec<DocRule>, BTreeMap<String, LabelDefinition>)>,
    /// Trusted check names and their producer keys.
    pub trusted: Sourced<BTreeMap<String, BTreeSet<String>>>,
    /// The repo's protected labels; without a head, the floor's defaults.
    pub labels: Sourced<BTreeMap<String, String>>,
    /// The repo's own rules.
    pub rules: Sourced<Vec<DocRule>>,
}

/// Read the governance in force for `repo`. Call after `hold()`. A head that
/// does not parse is an error, never an empty document.
pub(crate) async fn read_policy(
    tx: &mut AdmittedTx,
    repo: &RepoCoordinate,
) -> Result<Policy, DbError> {
    let floor = match head(tx, &DocumentId::Floor).await? {
        Some((Body::Floor { rules, labels, .. }, id)) => Sourced {
            value: (rules, labels),
            head: Some(id),
        },
        _ => Sourced {
            value: Default::default(),
            head: None,
        },
    };
    let trusted = match head(tx, &DocumentId::Trusted).await? {
        Some((Body::Trusted { checks }, id)) => Sourced {
            value: checks,
            head: Some(id),
        },
        _ => Sourced {
            value: Default::default(),
            head: None,
        },
    };
    let labels = match head(tx, &DocumentId::Labels(repo.clone())).await? {
        Some((Body::Labels { values }, id)) => Sourced {
            value: values,
            head: Some(id),
        },
        // No head: the floor's defaults apply. No labels at all would make
        // every label-scoped floor rule inapplicable, which fails open.
        _ => Sourced {
            value: floor
                .value
                .1
                .iter()
                .filter_map(|(k, d)| Some((k.clone(), d.default.clone()?)))
                .collect(),
            head: None,
        },
    };
    let rules = match head(tx, &DocumentId::Rules(repo.clone())).await? {
        Some((Body::Rules { rules }, id)) => Sourced {
            value: rules,
            head: Some(id),
        },
        _ => Sourced {
            value: Vec::new(),
            head: None,
        },
    };
    Ok(Policy {
        floor,
        trusted,
        labels,
        rules,
    })
}

async fn head(
    tx: &mut AdmittedTx,
    id: &DocumentId,
) -> Result<Option<(Body, nostr::EventId)>, DbError> {
    let Some(event) = buzz_db::governance::head(tx, id).await? else {
        return Ok(None);
    };
    let doc = Document::parse(&event.content, id)
        .map_err(|e| DbError::InvalidData(format!("unavailable: governance head {id}: {e}")))?;
    Ok(Some((doc.body, event.id)))
}

/// What one ref's decision relied on, as audit sequence numbers.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(super) struct Counted {
    /// Approvals offered to the gate for this ref and tip.
    pub evidence: BTreeSet<String>,
    /// Check results offered to the gate for this ref's required checks.
    pub checks: BTreeSet<String>,
}

/// The governed verdict for a push, with what each ref relied on.
pub(super) struct Decision {
    pub verdict: Result<(), Vec<Denial>>,
    pub counted: BTreeMap<String, Counted>,
}

/// A push's verdict: the repository's legacy protections AND governance.
/// Legacy rules stay in their own evaluator so `require-patch`, owner-only
/// pushes and tag defaults are exactly what they were.
#[allow(clippy::result_large_err)] // The land's closures return its Axum response.
pub(super) async fn push(
    tx: &mut AdmittedTx,
    authority: &Authority,
    repo: &RepoCoordinate,
    actor: &[u8],
    updates: &[RefUpdate],
    scan: gate::Scan,
    open: bool,
) -> Result<Decision, axum::response::Response> {
    let legacy = match authority.role {
        Some(role) => buzz_core::git_perms::evaluate_push(updates, role, &authority.rules),
        None => Err(updates
            .iter()
            .map(|u| Denial {
                ref_name: u.ref_name.clone(),
                reason: super::authority::NOT_A_MEMBER.into(),
            })
            .collect()),
    };
    let governed = decide(tx, authority, repo, actor, updates, scan, open)
        .await
        .map_err(|e| {
            tracing::error!(error = %e, "git land: governance unavailable");
            axum::response::IntoResponse::into_response((
                axum::http::StatusCode::SERVICE_UNAVAILABLE,
                "authorization unavailable",
            ))
        })?;
    let verdict = match (legacy, governed.verdict) {
        (Ok(()), Ok(())) => Ok(()),
        (Err(a), Ok(())) | (Ok(()), Err(a)) => Err(a),
        (Err(mut a), Err(b)) => {
            a.extend(b);
            Err(a)
        }
    };
    Ok(Decision {
        verdict,
        counted: governed.counted,
    })
}

/// Decide `updates` by `actor` under the governance read in `tx`.
/// `scan` is the secret scan of what the push brings (`super::scan`);
/// `open` is whether the community admits non-members.
pub(super) async fn decide(
    tx: &mut AdmittedTx,
    authority: &Authority,
    repo: &RepoCoordinate,
    actor: &[u8],
    updates: &[RefUpdate],
    scan: gate::Scan,
    open: bool,
) -> Result<Decision, DbError> {
    let policy = read_policy(tx, repo).await?;
    let heads: Vec<String> = updates
        .iter()
        .filter(|u| u.new_oid != ZERO_OID)
        .map(|u| u.new_oid.clone())
        .collect();
    let rows = buzz_db::git_evidence::read_in_transaction(tx, repo, &heads).await?;
    // Approvers' standing is read now, under the land's locks, not when
    // they approved.
    let mut approvers = HashMap::new();
    for row in latest_approvals(&rows) {
        if approvers.contains_key(&row.actor) {
            continue;
        }
        let key = hex::decode(&row.actor)
            .map_err(|_| DbError::InvalidData("invalid evidence producer".into()))?;
        let facts = git_land::key_facts(tx, authority.channel, &key).await?;
        approvers.insert(row.actor.clone(), facts);
    }
    let pusher = Principal {
        key: hex::encode(actor),
        facts: &authority.facts,
        granted: authority.granted,
    };
    assemble(
        &policy, repo, &pusher, updates, scan, &rows, &approvers, open,
    )
}

/// The pusher as the land read it.
pub(super) struct Principal<'a> {
    pub key: String,
    pub facts: &'a git_land::KeyFacts,
    pub granted: bool,
}

/// The latest approval statement per producer and subject, by audit
/// sequence, that still approves; a withdrawal leaves none.
fn latest_approvals(rows: &[EvidenceRow]) -> Vec<&EvidenceRow> {
    let mut latest: BTreeMap<(&str, &str, &str, &str), &EvidenceRow> = BTreeMap::new();
    for row in rows {
        let e = &row.evidence;
        if let EvidenceBody::Approval { .. } = e.body {
            let k = (&*row.actor, &*e.git_ref, &*e.head, &*e.base);
            if latest.get(&k).is_none_or(|r| r.seq < row.seq) {
                latest.insert(k, row);
            }
        }
    }
    latest
        .into_values()
        .filter(|r| matches!(r.evidence.body, EvidenceBody::Approval { approved: true }))
        .collect()
}

/// The gate's input from what the land read, and its verdict. Pure.
#[allow(clippy::too_many_arguments)] // One per gate input, each read apart.
fn assemble(
    policy: &Policy,
    repo: &RepoCoordinate,
    pusher: &Principal<'_>,
    updates: &[RefUpdate],
    scan: gate::Scan,
    rows: &[EvidenceRow],
    approvers: &HashMap<String, git_land::KeyFacts>,
    open: bool,
) -> Result<Decision, DbError> {
    let mut ids = Interner::default();
    let mut trusted = Vec::new();
    for (name, keys) in &policy.trusted.value {
        for key in keys {
            trusted.push(gate::Reporter {
                check: ids.check(name),
                key: ids.key(key),
            });
        }
    }
    let trusted_keys: BTreeSet<u64> = trusted.iter().map(|r| r.key).collect();
    let pusher = key_facts(
        &mut ids,
        &pusher.key,
        pusher.facts,
        pusher.granted,
        open,
        &trusted_keys,
    );

    let cmds: Vec<gate::Cmd> = updates
        .iter()
        .map(|u| gate::Cmd {
            git_ref: ids.git_ref(&u.ref_name),
            is_branch: u.ref_name.starts_with("refs/heads/"),
            old: ids.oid(&u.old_oid),
            // The swap is conditioned on the manifest this push was built
            // on, so the tip it replaces is the tip in force.
            current: ids.oid(&u.old_oid),
            new: ids.oid(&u.new_oid),
            fast_forward: u.kind == UpdateKind::FastForward,
            // Stricter than the truth: any new tip may bring commits, so
            // the pusher never approves its own push.
            brings: u.new_oid != ZERO_OID,
            // Not tracked yet (git_repo_brought): earlier authors can
            // still approve.
            authors: Vec::new(),
            // Not resolved yet: a codeowners requirement denies instead.
            owners: Vec::new(),
        })
        .collect();

    let (mut floor, mut labels) = (Vec::new(), Vec::new());
    for rule in &policy.floor.value.0 {
        let scope = if rule.labels.is_empty() {
            gate::Scope::All
        } else {
            let label = ids.label(&rule.labels);
            if rule
                .labels
                .iter()
                .all(|(k, v)| policy.labels.value.get(k) == Some(v))
            {
                labels.push(label);
            }
            gate::Scope::Label(label)
        };
        floor.extend(expand(&mut ids, rule, scope, updates));
    }
    let mut repo_rules = Vec::new();
    for rule in &policy.rules.value {
        repo_rules.extend(expand(&mut ids, rule, gate::Scope::All, updates));
    }

    let mut posts = Vec::new();
    for row in rows {
        let e = &row.evidence;
        if let EvidenceBody::Check { name, passed } = &e.body {
            posts.push(gate::Post {
                by: ids.key(&row.actor),
                check: ids.check(name),
                head: ids.oid(&e.head),
                pass: *passed,
                seq: u64::try_from(row.seq)
                    .map_err(|_| DbError::InvalidData("invalid evidence sequence".into()))?,
            });
        }
    }
    let mut approvals = Vec::new();
    for row in latest_approvals(rows) {
        let read = approvers
            .get(&row.actor)
            .ok_or_else(|| DbError::InvalidData("approver facts not read".into()))?;
        // An approver acts for itself; it holds no grant.
        let by = key_facts(&mut ids, &row.actor, read, false, open, &trusted_keys);
        let e = &row.evidence;
        approvals.push(gate::Approval {
            by,
            repo: ids.repo(&e.repo),
            git_ref: ids.git_ref(&e.git_ref),
            head: ids.oid(&e.head),
            base: ids.oid(&e.base),
        });
    }

    let input = gate::Input {
        repo: ids.repo(&repo.to_string()),
        pusher,
        scan,
        cmds,
        floor,
        labels,
        repo_rules,
        trusted,
        approvals,
        posts,
    };
    let counted = counted(&input, updates, rows, &mut ids);
    let verdict = if gate::decide(&input) {
        Ok(())
    } else {
        // A scan that is not clean refuses every ref; otherwise name the
        // refs whose rules are not met.
        let scan = match input.scan {
            gate::Scan::Clean => None,
            gate::Scan::Dirty => Some(SECRET_DETECTED),
            gate::Scan::Unknown => Some(SCAN_INCOMPLETE),
        };
        let denied: Vec<Denial> = updates
            .iter()
            .zip(&input.cmds)
            .filter_map(|(u, c)| {
                let reason = scan.or(
                    (!gate::ref_ok(&input, c)).then_some("governance requirements are not met")
                )?;
                Some(Denial {
                    ref_name: u.ref_name.clone(),
                    reason: reason.into(),
                })
            })
            .collect();
        Err(if denied.is_empty() {
            updates
                .iter()
                .map(|u| Denial {
                    ref_name: u.ref_name.clone(),
                    reason: "governance refused the push".into(),
                })
                .collect()
        } else {
            denied
        })
    };
    Ok(Decision { verdict, counted })
}

/// The gate's view of a key from the facts read under the land's locks.
fn key_facts(
    ids: &mut Interner,
    hex_key: &str,
    read: &git_land::KeyFacts,
    granted: bool,
    open: bool,
    trusted: &BTreeSet<u64>,
) -> gate::KeyFacts {
    let key = ids.key(hex_key);
    gate::KeyFacts {
        key,
        admitted: read.admitted(open),
        role: role(read.key.channel_role.as_deref()),
        who: match &read.owner {
            Some((owner, f)) => gate::Who::Agent(gate::Owner {
                key: ids.key(&hex::encode(owner)),
                admitted: f.admitted(open),
                role: role(f.channel_role.as_deref()),
            }),
            None => gate::Who::Person,
        },
        granted,
        check_key: trusted.contains(&key),
    }
}

/// A stored channel role as the gate ranks it; an unknown role is none.
fn role(stored: Option<&str>) -> gate::Role {
    match stored {
        Some("owner" | "admin") => gate::Role::Admin,
        Some("member" | "bot") => gate::Role::Member,
        Some("guest") => gate::Role::Guest,
        _ => gate::Role::None,
    }
}

/// One document rule as gate rules, for each pushed ref it names.
fn expand(
    ids: &mut Interner,
    rule: &DocRule,
    scope: gate::Scope,
    updates: &[RefUpdate],
) -> Vec<gate::Rule> {
    let patterns: Vec<RefPattern> = rule
        .refs
        .iter()
        .filter_map(|p| RefPattern::parse(p).ok())
        .collect();
    // Approvals are strict about the target tip unless this rule says not.
    let relaxed = rule
        .require
        .iter()
        .any(|r| matches!(r, Requirement::FreshTarget { required: false }));
    let mut out = Vec::new();
    for u in updates {
        // A pattern that no longer parses governs every ref: deny, not skip.
        if patterns.len() != rule.refs.len() || patterns.iter().any(|p| p.matches(&u.ref_name)) {
            let git_ref = ids.git_ref(&u.ref_name);
            let mk = |req, n, check| gate::Rule {
                req,
                git_ref,
                scope,
                n,
                check,
            };
            if patterns.len() != rule.refs.len() {
                out.push(mk(gate::Req::Unknown, 0, 0));
                continue;
            }
            if !relaxed {
                out.push(mk(gate::Req::FreshTarget, 0, 0));
            }
            for req in &rule.require {
                out.push(match req {
                    Requirement::AdminPush => mk(gate::Req::AdminPush, 0, 0),
                    Requirement::NoForce => mk(gate::Req::NoForce, 0, 0),
                    Requirement::NoDelete => mk(gate::Req::NoDelete, 0, 0),
                    Requirement::Review { count, group: None } => {
                        mk(gate::Req::Review, u64::from(*count), 0)
                    }
                    // No reviewer-group input yet.
                    Requirement::Review { group: Some(_), .. } => mk(gate::Req::Unknown, 0, 0),
                    Requirement::FreshTarget { .. } => continue,
                    Requirement::Check { name } => {
                        let check = ids.check(name);
                        mk(gate::Req::Check, 0, check)
                    }
                    // Owners from the old tree are not resolved yet; an
                    // empty owner list would pass vacuously.
                    Requirement::Codeowners => mk(gate::Req::Unknown, 0, 0),
                });
            }
        }
    }
    out
}

/// The evidence each ref's decision was offered: approvals naming its ref
/// and new tip, and results for the checks its rules require on that tip.
fn counted(
    input: &gate::Input,
    updates: &[RefUpdate],
    rows: &[EvidenceRow],
    ids: &mut Interner,
) -> BTreeMap<String, Counted> {
    let mut out = BTreeMap::new();
    for (u, c) in updates.iter().zip(&input.cmds) {
        let required: BTreeSet<u64> = input
            .floor
            .iter()
            .chain(&input.repo_rules)
            .filter(|r| r.git_ref == c.git_ref && r.req == gate::Req::Check)
            .map(|r| r.check)
            .collect();
        let mut counted = Counted::default();
        for row in rows {
            let e = &row.evidence;
            if e.head != u.new_oid {
                continue;
            }
            match &e.body {
                EvidenceBody::Approval { approved: true } if e.git_ref == u.ref_name => {
                    counted.evidence.insert(row.seq.to_string());
                }
                EvidenceBody::Check { name, .. } if required.contains(&ids.check(name)) => {
                    counted.checks.insert(row.seq.to_string());
                }
                _ => {}
            }
        }
        out.insert(u.ref_name.clone(), counted);
    }
    out
}

/// Strings to the gate's opaque ids, one namespace per kind. Id 0 is
/// `NO_COMMIT`, the zero object id.
#[derive(Default)]
struct Interner(HashMap<String, u64>);

impl Interner {
    fn id(&mut self, s: String) -> u64 {
        let next = self.0.len() as u64 + 1;
        *self.0.entry(s).or_insert(next)
    }
    fn oid(&mut self, oid: &str) -> u64 {
        if oid == ZERO_OID {
            gate::NO_COMMIT
        } else {
            self.id(format!("oid:{oid}"))
        }
    }
    fn key(&mut self, hex: &str) -> u64 {
        self.id(format!("key:{}", hex.to_ascii_lowercase()))
    }
    fn git_ref(&mut self, name: &str) -> u64 {
        self.id(format!("ref:{name}"))
    }
    fn check(&mut self, name: &str) -> u64 {
        self.id(format!("check:{name}"))
    }
    fn repo(&mut self, coordinate: &str) -> u64 {
        self.id(format!("repo:{coordinate}"))
    }
    fn label(&mut self, labels: &BTreeMap<String, String>) -> u64 {
        let conj: Vec<String> = labels.iter().map(|(k, v)| format!("{k}={v}")).collect();
        self.id(format!("label:{}", conj.join("&")))
    }
}

#[cfg(test)]
#[path = "gate_tests.rs"]
mod tests;
