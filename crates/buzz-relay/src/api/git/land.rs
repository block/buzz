//! The governed land: the one Postgres transaction in which a Git change
//! takes effect (`docs/spec/GitGovernanceLand.tla`, protocol "row").
//!
//! ```text
//!   prepare   packs + manifest into the object store (content-addressed, inert)
//!   land      hold ─► read authority ─► decide ─► swap the row ─► receipt ─► commit
//!   cache     pointer CAS to the committed manifest
//! ```
//!
//! The repository row's manifest is the committed state; the object-store
//! pointer is a cache of it, written after commit. Each manifest names its
//! parent, so a reader tells a cache that is one land behind (a relay died
//! between commit and cache write) from a write the row never made (a relay
//! that predates governed lands). The second is adopted when it was built
//! directly on the row and the repository is ungoverned; otherwise writes
//! refuse until an operator reconciles it. Old relays are drained before new
//! ones take writes, so this covers only the pushes of the drain.
//!
//! The pre-receive hook still answers the client early with the same rules;
//! this is the decision that binds, taken under the authority in force at
//! the commit.

// Response is the natural error type for axum handlers.
#![allow(clippy::result_large_err)]

use std::collections::BTreeMap;
use std::path::Path;

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use buzz_core::channel::MemberRole;
use buzz_core::git_perms::{parse_protection_tags, Denial, ProtectionRule, RefUpdate, UpdateKind};
use buzz_core::governance::{GrantAction, RepoCoordinate};
use buzz_core::TenantContext;
use buzz_db::git_land;
use serde_json::json;
use tokio::process::Command;
use tracing::{error, warn};

use super::binding::{resolve_binding_tags, RepoBinding};
use super::gate::{Counted, Decision};
use super::hydrate::{get_verified_limited, load_pointer, HydrateError, MAX_MANIFEST_BYTES};
use super::manifest::{pointer_key, Manifest};
use super::store::{CasOutcome, ETag, GitStore, Precond};
use crate::state::AppState;
use buzz_conformance::git_land::{self as trace, LandAction, LandGuard, LandStep, LandTracer};

const ZERO_OID: &str = "0000000000000000000000000000000000000000";

/// The repository a Git route names, as the land keys it: `repo` is the id
/// `validate_repo_id` returned, used as is.
pub(super) fn coordinate(owner: &str, repo: &str) -> RepoCoordinate {
    RepoCoordinate {
        owner: owner.to_string(),
        name: repo.to_string(),
    }
}

/// What the land read, under its locks, for the decision.
pub(super) struct Authority {
    /// The role the actor is credited with in the bound channel.
    pub role: Option<MemberRole>,
    /// The repository's protection rules.
    pub rules: Vec<ProtectionRule>,
    /// The bound channel the authority was read in.
    pub channel: uuid::Uuid,
    /// The actor's facts and, for an agent, its owner's.
    pub facts: git_land::KeyFacts,
    /// For an agent: a live grant from its owner for this repository and
    /// change. Always true for a person.
    pub granted: bool,
    /// When the authority relied on lapses with no write, if it does.
    pub valid_until: Option<chrono::DateTime<chrono::Utc>>,
}

/// One change to land.
pub(super) struct Change<'a> {
    /// `"push"` or `"default_branch"`, recorded on the receipt.
    pub kind: &'static str,
    /// The repository.
    pub repo: &'a RepoCoordinate,
    /// The key the change is credited to.
    pub actor: &'a [u8],
    /// The Git grant the actor presented, for an agent.
    pub grant: Option<&'a buzz_core::governance::GitGrant>,
    /// The committed manifest the change was built on.
    pub seen: Option<&'a str>,
    /// The manifest it installs.
    pub new: &'a str,
    /// The ref changes it makes.
    pub updates: &'a [RefUpdate],
}

/// How a land ended.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum Verdict {
    /// The change committed.
    Landed,
    /// The authority in force refused it.
    Refused(Vec<Denial>),
    /// Another change committed first.
    Superseded,
}

/// Land `change` if `decide` allows it under the authority read in the land.
/// Every verdict commits a receipt; an error commits nothing.
pub(super) async fn land(
    state: &AppState,
    lease: &buzz_db::deletion::ServingWriteLease,
    change: Change<'_>,
    decide: impl AsyncFnOnce(&mut buzz_db::AdmittedTx, &Authority) -> Result<Decision, Response>,
) -> Result<Verdict, Response> {
    let (_guard, tracer) = LandGuard::arm(state.land_tracer.clone(), "git_land");
    let trace = Trace::new(&tracer, &change);
    let mut tx = state
        .db
        .begin_serving_write_transaction(lease)
        .await
        .map_err(unavailable)?;
    let mut counted = BTreeMap::new();
    let (verdict, role, version) = match judge(state, &mut tx, &change).await? {
        Judged::Refused(reason) => {
            let denials = refuse_all(&change, reason);
            trace.decide(&denials, 0, None, &counted);
            (Verdict::Refused(denials), None, None)
        }
        Judged::Held(authority, version) => {
            let decision = decide(&mut tx, &authority).await?;
            counted = decision.counted;
            let denials = decision.verdict.err().unwrap_or_default();
            trace.decide(&denials, version, authority.valid_until, &counted);
            if !denials.is_empty() {
                (Verdict::Refused(denials), authority.role, Some(version))
            } else {
                let swapped = git_land::swap(
                    &mut tx,
                    change.repo,
                    change.seen,
                    change.new,
                    authority.valid_until,
                )
                .await
                .map_err(unavailable)?;
                trace.land(swapped.as_ref());
                match swapped {
                    Some(swapped) => (Verdict::Landed, authority.role, Some(swapped.version)),
                    None => (Verdict::Superseded, authority.role, Some(version)),
                }
            }
        }
    };
    let reasons: Vec<_> = match &verdict {
        Verdict::Refused(denials) => denials
            .iter()
            .map(|d| json!({"ref": d.ref_name, "reason": d.reason}))
            .collect(),
        Verdict::Superseded => vec![json!({"reason": "superseded"})],
        Verdict::Landed => vec![],
    };
    let entry = buzz_audit::NewAuditEntry {
        community_id: tx.community(),
        action: if verdict == Verdict::Landed {
            buzz_audit::AuditAction::GitLanded
        } else {
            buzz_audit::AuditAction::GitRefused
        },
        actor_pubkey: Some(change.actor.to_vec()),
        object_id: Some(change.repo.to_string()),
        detail: json!({
            "kind": change.kind,
            "stage": "land",
            "manifest": change.new,
            "parent": change.seen,
            "updates": change.updates.iter().map(|u| json!({
                "ref": u.ref_name, "old": u.old_oid, "new": u.new_oid, "kind": format!("{:?}", u.kind),
            })).collect::<Vec<_>>(),
            "role": role.map(|r| r.as_str()),
            "authority_version": version,
            "reasons": reasons,
            "counted": counted.iter().map(|(r, c): (&String, &Counted)| (r.clone(), json!({
                "evidence": c.evidence, "checks": c.checks,
            }))).collect::<serde_json::Map<_, _>>(),
        }),
    };
    buzz_audit::AuditService::log_in_transaction_before(&mut tx, entry, None)
        .await
        .map_err(|e| {
            error!(repo = %change.repo, error = %e, "git land: receipt failed; nothing landed");
            internal()
        })?;
    tx.commit().await.map_err(unavailable)?;
    trace.receipts();
    Ok(verdict)
}

enum Judged {
    Held(Authority, u64),
    Refused(&'static str),
}

/// Hold the repository and read the authority the decision needs.
async fn judge(
    state: &AppState,
    tx: &mut buzz_db::AdmittedTx,
    change: &Change<'_>,
) -> Result<Judged, Response> {
    let Some(held) = git_land::hold(tx, change.repo).await.map_err(unavailable)? else {
        return Ok(Judged::Refused("repository not found"));
    };
    let Some(tags) = git_land::announcement_tags(tx, change.repo)
        .await
        .map_err(unavailable)?
    else {
        return Ok(Judged::Refused("repository not found"));
    };
    // The pinned channel is the binding. The announcement's tag only seeds a
    // pin the row does not have yet; a re-announcement moves nothing.
    let channel = match held.channel {
        Some(pinned) => pinned,
        None => {
            let announced = match resolve_binding_tags(tags.iter().map(Vec::as_slice)) {
                RepoBinding::Bound(channel) => channel,
                RepoBinding::NotBound => {
                    return Ok(Judged::Refused(
                        buzz_core::git_perms::GIT_NO_CHANNEL_BINDING_BODY,
                    ))
                }
                RepoBinding::Broken => return Ok(Judged::Refused("invalid channel binding")),
            };
            git_land::pin_channel(tx, change.repo, announced)
                .await
                .map_err(unavailable)?
                .unwrap_or(announced)
        }
    };
    if git_land::channel_closed(tx, channel)
        .await
        .map_err(unavailable)?
    {
        return Ok(Judged::Refused("channel is archived (read-only)"));
    }
    let rules = match parse_protection_tags(&tags) {
        Ok(parsed) if parsed.unknown_rules.is_empty() => parsed.rules,
        Ok(_) => {
            return Ok(Judged::Refused(
                "unknown protection rule: this relay does not enforce it, so it denies",
            ))
        }
        Err(_) => return Ok(Judged::Refused("malformed protection rules")),
    };
    let facts = git_land::key_facts(tx, channel, change.actor)
        .await
        .map_err(unavailable)?;
    // An agent acts only under a live Git grant from its owner, judged
    // under the locks the floor's writer takes. The swap applies only before
    // the grant lapses; the commit may follow just after, as the model
    // allows ("commit-after-expiry").
    let granted = match &facts.owner {
        Some((owner, _)) => {
            let action = match change.kind {
                "push" => GrantAction::Push,
                _ => GrantAction::Manage,
            };
            buzz_db::governance::live_grant(tx, change.grant, change.repo, owner, action)
                .await
                .map_err(unavailable)?
        }
        None => true,
    };
    if !granted {
        return Ok(Judged::Refused(super::authority::NO_GRANT));
    }
    let open = !state.config.require_relay_membership;
    let role = if !facts.admitted(open) {
        None
    } else {
        match &facts.owner {
            Some((_, owner)) if !owner.admitted(open) => None,
            owner => super::authority::credit(
                facts.key.channel_role.as_deref(),
                owner.as_ref().map(|(_, f)| f.channel_role.as_deref()),
            ),
        }
    };
    let valid_until = match (&facts.owner, change.grant.and_then(|g| g.before)) {
        (Some(_), Some(at)) => Some(
            i64::try_from(at)
                .ok()
                .and_then(|at| chrono::DateTime::from_timestamp(at, 0))
                .ok_or_else(|| {
                    unavailable(buzz_db::DbError::InvalidData("invalid grant expiry".into()))
                })?,
        ),
        _ => None,
    };
    Ok(Judged::Held(
        Authority {
            role,
            rules,
            channel,
            facts,
            granted,
            valid_until,
        },
        held.version,
    ))
}

fn refuse_all(change: &Change<'_>, reason: &str) -> Vec<Denial> {
    let refs: Vec<&str> = if change.updates.is_empty() {
        vec!["HEAD"]
    } else {
        change.updates.iter().map(|u| u.ref_name.as_str()).collect()
    };
    refs.into_iter()
        .map(|ref_name| Denial {
            ref_name: ref_name.to_string(),
            reason: reason.to_string(),
        })
        .collect()
}

/// The ref changes from `before` to `after`, classified with the ancestry
/// `repo_path` holds. Run before the land: it spawns Git, and the land holds
/// locks.
pub(super) async fn ref_updates(
    repo_path: &Path,
    before: &BTreeMap<String, String>,
    after: &BTreeMap<String, String>,
) -> Vec<RefUpdate> {
    let mut updates = Vec::new();
    let names = before
        .keys()
        .chain(after.keys().filter(|k| !before.contains_key(*k)));
    for name in names {
        let old = before.get(name).map_or(ZERO_OID, String::as_str);
        let new = after.get(name).map_or(ZERO_OID, String::as_str);
        if old == new {
            continue;
        }
        let is_ancestor =
            old != ZERO_OID && new != ZERO_OID && is_ancestor(repo_path, old, new).await;
        updates.push(RefUpdate {
            ref_name: name.clone(),
            kind: UpdateKind::classify(old, new, is_ancestor),
            old_oid: old.to_string(),
            new_oid: new.to_string(),
        });
    }
    updates
}

/// `git merge-base --is-ancestor`; anything but a yes is a no, which can
/// only make the decision stricter.
async fn is_ancestor(repo_path: &Path, old: &str, new: &str) -> bool {
    let mut cmd = Command::new("git");
    cmd.args(["merge-base", "--is-ancestor", old, new])
        .current_dir(repo_path)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true);
    super::transport::harden_git_env(&mut cmd);
    matches!(cmd.status().await, Ok(status) if status.success())
}

/// Write the cache after a land committed. Losing the race is benign: the
/// only writers after a commit are later lands and readers advancing the
/// cache past it.
pub(super) async fn write_cache(
    store: &GitStore,
    tenant: &TenantContext,
    repo: &RepoCoordinate,
    seen: Option<&ETag>,
    digest: &str,
) {
    let pointer = pointer_key(tenant.community(), &repo.owner, &repo.name);
    let precond = seen.map_or(Precond::IfNoneMatchStar, |e| Precond::IfMatch(e.clone()));
    match store
        .put_pointer(&pointer, digest.as_bytes(), precond)
        .await
    {
        Ok(CasOutcome::Won(_)) => {}
        Ok(CasOutcome::LostRace) => {
            warn!(repo = %repo, "git land: cache moved after commit; leaving it to its writer")
        }
        Err(e) => warn!(repo = %repo, error = %e, "git land: cache write failed after commit"),
    }
}

/// The committed state a write builds on: the row's manifest, with the cache
/// advanced to it if it is one land behind. `None` for a repository with no
/// state yet.
pub(super) async fn committed_for_write(
    state: &AppState,
    tenant: &TenantContext,
    repo: &RepoCoordinate,
) -> Result<Option<(ETag, String, Manifest)>, Response> {
    let store = &state.git_store;
    let cache = load_pointer(store, tenant, &repo.owner, &repo.name)
        .await
        .map_err(|e| hydrate_failed(repo, e))?;
    let Some(row) = row_manifest(state, tenant, repo, cache.as_ref()).await? else {
        return Ok(cache);
    };
    let cache_digest = cache.as_ref().map(|(_, digest, _)| digest.as_str());
    if cache_digest == Some(row.as_str()) {
        return Ok(cache);
    }
    if adopted(state, tenant, repo, &row, cache.as_ref()).await? {
        return Ok(cache);
    }
    let manifest = read_manifest(store, repo, &row).await?;
    if manifest.parent.as_deref() != cache_digest {
        error!(repo = %repo, committed = %row, cache = ?cache_digest, "git: object-store pointer was written outside the land");
        return Err((
            StatusCode::CONFLICT,
            "repository state was changed outside this relay's commit point; writes are refused until an operator reconciles it",
        )
            .into_response());
    }
    let pointer = pointer_key(tenant.community(), &repo.owner, &repo.name);
    let precond = cache
        .as_ref()
        .map_or(Precond::IfNoneMatchStar, |(e, _, _)| {
            Precond::IfMatch(e.clone())
        });
    match store.put_pointer(&pointer, row.as_bytes(), precond).await {
        Ok(CasOutcome::Won(etag)) => Ok(Some((etag, row, manifest))),
        Ok(CasOutcome::LostRace) => Err(superseded()),
        Err(e) => Err(hydrate_failed(repo, e.into())),
    }
}

/// The committed manifest a read serves: the row's, else the cache's.
pub(super) async fn committed_for_read(
    state: &AppState,
    tenant: &TenantContext,
    repo: &RepoCoordinate,
) -> Result<Option<Manifest>, Response> {
    let row = state
        .db
        .git_repo_manifest(tenant.community(), repo)
        .await
        .map_err(unavailable)?;
    let cache = load_pointer(&state.git_store, tenant, &repo.owner, &repo.name)
        .await
        .map_err(|e| hydrate_failed(repo, e))?;
    match (row, cache) {
        (Some(row), Some((_, digest, manifest))) if row == digest => Ok(Some(manifest)),
        (Some(row), cache) if adopted(state, tenant, repo, &row, cache.as_ref()).await? => {
            Ok(cache.map(|(_, _, manifest)| manifest))
        }
        (Some(row), _) => read_manifest(&state.git_store, repo, &row).await.map(Some),
        (None, cache) => Ok(cache.map(|(_, _, manifest)| manifest)),
    }
}

/// Whether the row now holds `cache`: a manifest published outside the land
/// directly on `row`, adopted because the repository is ungoverned.
async fn adopted(
    state: &AppState,
    tenant: &TenantContext,
    repo: &RepoCoordinate,
    row: &str,
    cache: Option<&(ETag, String, Manifest)>,
) -> Result<bool, Response> {
    let Some((_, digest, manifest)) = cache.filter(|(_, _, m)| m.parent.as_deref() == Some(row))
    else {
        return Ok(false);
    };
    let adopted = state
        .db
        .git_repo_adopt(tenant.community(), repo, row, digest)
        .await
        .map_err(unavailable)?;
    if adopted {
        warn!(repo = %repo, from = %row, to = %digest, parent = ?manifest.parent, "git: adopted an ungoverned push published outside the land");
    }
    Ok(adopted)
}

/// The row's manifest, seeding a row that has none from the cache.
async fn row_manifest(
    state: &AppState,
    tenant: &TenantContext,
    repo: &RepoCoordinate,
    cache: Option<&(ETag, String, Manifest)>,
) -> Result<Option<String>, Response> {
    let community = tenant.community();
    let row = state
        .db
        .git_repo_manifest(community, repo)
        .await
        .map_err(unavailable)?;
    match (row, cache) {
        (Some(row), _) => Ok(Some(row)),
        (None, None) => Ok(None),
        (None, Some((_, digest, _))) => state
            .db
            .git_repo_seed_manifest(community, repo, digest)
            .await
            .map_err(unavailable),
    }
}

async fn read_manifest(
    store: &GitStore,
    repo: &RepoCoordinate,
    digest: &str,
) -> Result<Manifest, Response> {
    let read = async {
        let key = format!("manifests/{digest}");
        let bytes = get_verified_limited(store, &key, digest, MAX_MANIFEST_BYTES).await?;
        let manifest = Manifest::from_bytes(&bytes)?;
        manifest.validate()?;
        Ok::<_, HydrateError>(manifest)
    };
    read.await.map_err(|e| hydrate_failed(repo, e))
}

fn hydrate_failed(repo: &RepoCoordinate, e: HydrateError) -> Response {
    super::transport::hydrate_error_to_response(&repo.owner, &repo.name, e)
}

pub(super) fn superseded() -> Response {
    (
        StatusCode::CONFLICT,
        "push superseded by a concurrent writer; pull and retry",
    )
        .into_response()
}

fn unavailable(e: buzz_db::DbError) -> Response {
    error!(error = %e, "git land: database unavailable");
    (StatusCode::SERVICE_UNAVAILABLE, "authorization unavailable").into_response()
}

fn internal() -> Response {
    (StatusCode::INTERNAL_SERVER_ERROR, "git backend error").into_response()
}

/// The land's conformance trace (`buzz_conformance::git_land`). Steps are
/// built only when the tracer observes them.
struct Trace<'a> {
    tracer: &'a std::sync::Arc<dyn LandTracer>,
    push: trace_id::Push,
    /// Every ref the change publishes and the tip it publishes there.
    refs: Vec<(String, String)>,
    repo: String,
    actor: String,
    decided: std::cell::RefCell<Vec<(String, trace::LandVerdict, Counted)>>,
}

mod trace_id {
    /// The push's opaque id: the manifest it installs.
    pub(super) type Push = buzz_conformance::OpaqueId;
}

impl<'a> Trace<'a> {
    fn new(tracer: &'a std::sync::Arc<dyn LandTracer>, change: &Change<'_>) -> Self {
        let refs = if change.updates.is_empty() {
            vec![("HEAD".to_string(), change.new.to_string())]
        } else {
            change
                .updates
                .iter()
                .map(|u| (u.ref_name.clone(), u.new_oid.clone()))
                .collect()
        };
        Self {
            tracer,
            push: buzz_conformance::OpaqueId(change.new.to_string()),
            refs,
            repo: change.repo.to_string(),
            actor: hex::encode(change.actor),
            decided: Default::default(),
        }
    }

    fn record(&self, action: LandAction) {
        self.tracer.record(LandStep::new(action));
    }

    fn decide(
        &self,
        denials: &[Denial],
        read_version: u64,
        valid_until: Option<chrono::DateTime<chrono::Utc>>,
        counted: &BTreeMap<String, Counted>,
    ) {
        for (git_ref, tip) in &self.refs {
            let verdict = if denials.iter().any(|d| &d.ref_name == git_ref) {
                trace::LandVerdict::Deny
            } else if denials.is_empty() {
                trace::LandVerdict::Allow
            } else {
                // The push lands whole or not at all.
                trace::LandVerdict::Deny
            };
            let c = counted.get(git_ref).cloned().unwrap_or_default();
            self.decided
                .borrow_mut()
                .push((git_ref.clone(), verdict, c.clone()));
            if !self.tracer.enabled() {
                continue;
            }
            self.record(LandAction::Decide {
                push: self.push.clone(),
                repo: buzz_conformance::OpaqueId(self.repo.clone()),
                git_ref: git_ref.clone(),
                new_tip: buzz_conformance::OpaqueId(tip.clone()),
                actor: buzz_conformance::ActorLabel(self.actor.clone()),
                read_version,
                verdict,
                valid_until: valid_until.and_then(|t| u64::try_from(t.timestamp_millis()).ok()),
                evidence: opaque(&c.evidence),
                checks: opaque(&c.checks),
            });
        }
    }

    fn land(&self, swapped: Option<&git_land::Swapped>) {
        if !self.tracer.enabled() {
            return;
        }
        let refs = self
            .refs
            .iter()
            .map(|(r, t)| (r.clone(), buzz_conformance::OpaqueId(t.clone())))
            .collect();
        self.record(match swapped {
            Some(s) => LandAction::Land {
                push: self.push.clone(),
                refs,
                cas_version: s.version,
                swapped_at: u64::try_from(s.swapped_at.timestamp_millis()).unwrap_or(0),
                cas: trace::CasOutcome::Won,
            },
            None => LandAction::Land {
                push: self.push.clone(),
                refs,
                cas_version: 0,
                swapped_at: 0,
                cas: trace::CasOutcome::Lost,
            },
        });
    }

    /// Recorded after commit: a receipt exists only once it is durable.
    fn receipts(&self) {
        if !self.tracer.enabled() {
            return;
        }
        for (git_ref, verdict, c) in self.decided.borrow().iter() {
            self.record(LandAction::Receipt {
                push: self.push.clone(),
                git_ref: git_ref.clone(),
                verdict: *verdict,
                evidence: opaque(&c.evidence),
                checks: opaque(&c.checks),
            });
        }
    }
}

fn opaque(
    ids: &std::collections::BTreeSet<String>,
) -> std::collections::BTreeSet<buzz_conformance::OpaqueId> {
    ids.iter()
        .cloned()
        .map(buzz_conformance::OpaqueId)
        .collect()
}
