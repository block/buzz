//! The store-mutation half of `add_team_from_catalog`: turn a verified
//! projection into local records with byte-level rollback on error.
//!
//! [`plan_add`] computes both stores in memory before anything is written, so
//! a member-resolution failure cannot leave a half-added team on disk. Only
//! the two saves remain: before either write we snapshot the raw bytes of both
//! files (or record their absence), and on a failed save we restore both
//! snapshots byte-exactly — including a reactivated member copy whose logical
//! undo would be a field revert with no row to delete.
//!
//! **Crash window.** A kill between the two commits (or between the second and
//! a successful restore) leaves the stores inconsistent: the team without some
//! member copies, or the copies without the team. The next add of the same
//! publication is idempotent — the replay check in `plan_add` finds the team
//! if present, and orphaned copies are reused by provenance matching. Retry is
//! the recovery path.

use std::path::Path;

use tauri::{AppHandle, Manager};
use uuid::Uuid;

use crate::{
    app_state::AppState,
    managed_agents::{
        load_personas, load_teams, managed_agents_store_path, save_personas, save_teams,
        team_catalog::{
            builtin_catalog_slug, local_member_projection_hash, TeamCatalogContent,
            TeamCatalogMember,
        },
        teams_store_path, try_regenerate_nest, AgentDefinition, RespondTo, TeamCatalogSource,
        TeamMemberCatalogSource, TeamRecord,
    },
    util::now_iso,
};

use super::AddTeamFromCatalogResult;

/// The complete post-add state of both stores, plus the team to report.
///
/// `stores` is `None` when nothing needs writing — the replay case.
#[derive(Debug)]
pub(super) struct AddPlan {
    pub stores: Option<(Vec<AgentDefinition>, Vec<TeamRecord>)>,
    /// The member copies the add created or reactivated — the rows that need a
    /// retention head enqueued once the commit succeeds, so a crash before the
    /// next boot reconcile cannot lose the only copy. Reused built-ins are
    /// untouched local records and contribute nothing; a replay carries an
    /// empty vec because it writes nothing.
    pub retain_personas: Vec<AgentDefinition>,
    pub team: TeamRecord,
}

/// One resolved member: the local id to put in the team's membership, and
/// whether the resolution created or reactivated a row that must be retained.
struct ResolvedMember {
    id: String,
    retain: bool,
}

/// Read the raw bytes of `path`, or `None` if the file does not yet exist.
///
/// Delegates to `managed_agents::storage::snapshot_store`.
pub(super) use crate::managed_agents::storage::snapshot_store as snapshot;

/// Write both stores with byte-level rollback on failure, using
/// caller-supplied pre-computed snapshots.
///
/// Both restores are attempted independently, so a persona-restore failure
/// does not prevent the team restore; errors from both are aggregated (I5).
///
/// Delegates to `managed_agents::storage::commit_stores_with_snapshots`.
pub(super) use crate::managed_agents::storage::commit_stores_with_snapshots as commit_stores_with_snaps;

/// Write both stores with byte-level rollback on failure.
///
/// Snapshots the files just before the writes. Prefer
/// [`commit_stores_with_snaps`] when you need to snapshot before a write-on-load
/// call that precedes the actual writes.
#[cfg_attr(not(test), allow(dead_code))]
pub(super) fn commit_stores(
    personas_path: &Path,
    teams_path: &Path,
    write_personas: impl FnOnce() -> Result<(), String>,
    write_teams: impl FnOnce() -> Result<(), String>,
) -> Result<(), String> {
    let personas_snap = snapshot(personas_path)?;
    let teams_snap = snapshot(teams_path)?;
    commit_stores_with_snaps(
        personas_path,
        teams_path,
        personas_snap,
        teams_snap,
        write_personas,
        write_teams,
    )
}

pub(super) fn add_verified_team(
    app: &AppHandle,
    scope: crate::managed_agents::retention::RetentionScope,
    source: &TeamCatalogSource,
    content: &TeamCatalogContent,
) -> Result<AddTeamFromCatalogResult, String> {
    let state = app.state::<AppState>();
    // Held across load, plan, and save: the replay check is only meaningful if
    // no concurrent add of the same coordinate can interleave.
    let _store_guard = state
        .managed_agents_store_lock
        .lock()
        .map_err(|error| error.to_string())?;

    // Community-boundary fence (Carl r11 P1). `scope` was captured before the
    // relay round-trip; here — under the store lock, before ANY store mutation —
    // reject if the workspace has switched relay or identity since. Without this
    // an adoption started in community A but completed after a switch to B would
    // commit A's team into the workspace-global stores and enqueue A's owner
    // heads in B's retention db, so B's flush publishes A's config into the wrong
    // community.
    assert_adoption_scope_unchanged(
        &scope,
        &crate::relay::relay_api_base_url_with_override(&state),
        &state.signing_keys()?.public_key().to_hex(),
    )?;

    let personas_path = managed_agents_store_path(app)?;
    let teams_path = teams_store_path(app)?;

    // Snapshot raw bytes BEFORE any load: load_personas() can write merged
    // built-ins on first call (write-on-load). Snapshotting after that write
    // would capture post-merge bytes as "before", so rollback would restore
    // the wrong content (I5).
    let personas_snap = snapshot(&personas_path)?;
    let teams_snap = snapshot(&teams_path)?;

    let personas_before = load_personas(app)?;
    let teams_before = load_teams(app)?;
    let plan = plan_add(&personas_before, &teams_before, source, content, &now_iso())?;

    // The seam owns the durable commit and the retention enqueue as one unit,
    // so there is no route to an adoption commit that skips retention: the
    // commit and the scope resolution are injected here but sequenced inside
    // `commit_and_enqueue`. Snapshots were taken before any load effect, so the
    // rollback inside the commit closure is byte-exact even for a reactivated
    // member copy whose logical undo is a field revert. Retention enqueues into
    // the CAPTURED scope (fenced above), never a re-resolved live one.
    let result = commit_and_enqueue(
        plan,
        |personas, teams| {
            commit_stores_with_snaps(
                &personas_path,
                &teams_path,
                personas_snap,
                teams_snap,
                || save_personas(app, personas),
                || save_teams(app, teams),
            )
        },
        || Ok(scope),
    )?;

    if !result.already_present {
        try_regenerate_nest(app);
    }
    Ok(result)
}

/// Fail closed when the workspace switched relay or identity between capturing
/// the adoption scope and committing it. Relay + owner together key the
/// retention scope, so requiring BOTH to still match the live workspace proves
/// the captured `scope` still owns it — a relay-only match would miss a
/// same-relay identity switch, and an owner-only match would miss a
/// cross-community move. Pure over the captured scope and the two live reads so
/// the fence is testable without a Tauri app.
pub(super) fn assert_adoption_scope_unchanged(
    scope: &crate::managed_agents::retention::RetentionScope,
    live_api_base_url: &str,
    live_signer_hex: &str,
) -> Result<(), String> {
    crate::relay::assert_expected_relay_scope(Some(&scope.relay_url), live_api_base_url)?;
    crate::relay::assert_expected_signer(
        Some(&scope.owner_keys.public_key().to_hex()),
        live_signer_hex,
    )
}

/// The app-independent core of an adoption commit: skip on replay, otherwise
/// write both stores durably and — only once that commit succeeds — enqueue the
/// retention heads. This is the SOLE route to a durable adoption commit; the
/// command injects the real store write and scope resolution as closures but
/// never commits directly, so retention cannot be silently bypassed by a
/// commit that sidesteps this seam.
///
/// `commit` performs the byte-rollback store write; a replay (`plan.stores ==
/// None`) never calls it. Retention is best-effort per the snapshot-import
/// policy — a scope-resolution or enqueue hiccup must not fail an add whose
/// disk write already succeeded; the boot reconcile is the backstop. A failed
/// commit propagates and enqueues nothing.
pub(super) fn commit_and_enqueue(
    plan: AddPlan,
    commit: impl FnOnce(&[AgentDefinition], &[TeamRecord]) -> Result<(), String>,
    resolve_scope: impl FnOnce() -> Result<crate::managed_agents::retention::RetentionScope, String>,
) -> Result<AddTeamFromCatalogResult, String> {
    let Some((personas, teams)) = plan.stores else {
        return Ok(AddTeamFromCatalogResult {
            team: plan.team,
            already_present: true,
        });
    };

    commit(&personas, &teams)?;

    // The commit is durable; enqueue retention heads so a crash before the next
    // boot reconcile cannot lose the only adopted copy. Resolving the scope
    // needs signable owner keys — the same precondition every retain path has.
    match resolve_scope() {
        Ok(scope) => enqueue_adoption_retention(&scope, &plan.retain_personas, &plan.team),
        Err(e) => eprintln!("buzz-desktop: adopt-retain scope unavailable: {e}"),
    }

    Ok(AddTeamFromCatalogResult {
        team: plan.team,
        already_present: false,
    })
}

/// Enqueue a pending retention head for every member copy the add wrote and for
/// the adopted team, in an already-resolved scope. Each failure is logged and
/// swallowed independently so one bad row never strands the rest — the boot
/// reconcile remains the backstop. Pure over the scope + records, so a test can
/// drive it against a temp-dir scope and assert the exact pending rows.
pub(super) fn enqueue_adoption_retention(
    scope: &crate::managed_agents::retention::RetentionScope,
    retain_personas: &[AgentDefinition],
    team: &TeamRecord,
) {
    for persona in retain_personas {
        if let Err(e) = crate::commands::personas::retain_persona_pending_at(scope, persona) {
            eprintln!("buzz-desktop: adopt persona-retain: {e}");
        }
    }
    if let Err(e) = crate::commands::teams::retain_team_pending_at(scope, team) {
        eprintln!("buzz-desktop: adopt team-retain: {e}");
    }
}

/// Compute both stores as they will be after the add. Pure — no I/O, so every
/// resolution rule below is testable without a Tauri app or a relay.
pub(super) fn plan_add(
    personas_before: &[AgentDefinition],
    teams_before: &[TeamRecord],
    source: &TeamCatalogSource,
    content: &TeamCatalogContent,
    now: &str,
) -> Result<AddPlan, String> {
    // Replay: the same publication added twice returns the team already held
    // instead of minting a second copy.
    if let Some(existing) = teams_before
        .iter()
        .find(|team| team.catalog_source.as_ref() == Some(source))
    {
        return Ok(AddPlan {
            stores: None,
            retain_personas: Vec::new(),
            team: existing.clone(),
        });
    }

    let mut personas = personas_before.to_vec();
    let resolved = content
        .members
        .iter()
        .map(|member| resolve_member(&mut personas, source, member, now))
        .collect::<Result<Vec<_>, _>>()?;
    // Retain only the rows this add created or reactivated, so a byte-identical
    // reused built-in is never republished under the adopter's identity.
    let retain_ids: std::collections::HashSet<&str> = resolved
        .iter()
        .filter(|resolved| resolved.retain)
        .map(|resolved| resolved.id.as_str())
        .collect();
    let retain_personas = personas
        .iter()
        .filter(|persona| retain_ids.contains(persona.id.as_str()))
        .cloned()
        .collect();
    let persona_ids = resolved.into_iter().map(|resolved| resolved.id).collect();
    let team = TeamRecord {
        id: Uuid::new_v4().to_string(),
        name: content.name.clone(),
        description: content.description.clone(),
        instructions: content.instructions.clone(),
        persona_ids,
        is_builtin: false,
        // A copy is not published. Sharing it is a separate, explicit act by
        // its new owner, at their own coordinate.
        shared: false,
        catalog_source: Some(source.clone()),
        source_dir: None,
        is_symlink: false,
        symlink_target: None,
        version: None,
        created_at: now.to_string(),
        updated_at: now.to_string(),
    };

    let mut teams = teams_before.to_vec();
    teams.push(team.clone());
    Ok(AddPlan {
        stores: Some((personas, teams)),
        retain_personas,
        team,
    })
}

/// Resolve one published member to a local persona id, adding or reactivating
/// a record as needed. Returns the local id to put in the team's membership
/// and whether the resolution wrote a row that must be retained.
fn resolve_member(
    personas: &mut Vec<AgentDefinition>,
    source: &TeamCatalogSource,
    member: &TeamCatalogMember,
    now: &str,
) -> Result<ResolvedMember, String> {
    if let Some(local_id) = reusable_builtin(personas, member) {
        // A byte-identical local built-in: no row is written, nothing to
        // retain.
        return Ok(ResolvedMember {
            id: local_id,
            retain: false,
        });
    }
    if let Some(existing) = personas
        .iter_mut()
        .find(|persona| member_provenance_matches(persona, source, member))
    {
        // A copy of this exact member version already exists from an earlier
        // add of this publication. Reuse it, reactivating if a prior team
        // delete left it inactive. Reuse is NOT extended across publications:
        // two teams by one publisher embedding an identical member get one
        // copy each, so deleting either cannot orphan a record the other uses.
        //
        // Always retain the reuse. On the ordinary success path this
        // re-publishes the copy's 30175 at a bumped `created_at` — a harmless
        // monotonic no-op. It is load-bearing on the documented crash-recovery
        // retry: the first attempt wrote the persona but died before post-commit
        // retention, so this copy has NO 30175 row yet. `plan_add` short-circuits
        // once the team row exists, so this reuse branch is the only place a
        // recovery retry can enqueue the missing member head — a `retain: false`
        // here (the prior `reactivated`-only value) would omit it permanently.
        // Retaining unconditionally is conservative, not exact: a copy still
        // referenced by a standalone managed agent stays active after a team
        // delete, so an active reuse can already hold a live head; re-retaining
        // it only bumps that head. Reused built-ins are handled above and never
        // reach here, so the adopter never republishes someone else's built-in.
        let reactivated = !existing.is_active;
        if reactivated {
            existing.is_active = true;
            existing.updated_at = now.to_string();
        }
        return Ok(ResolvedMember {
            id: existing.id.clone(),
            retain: true,
        });
    }
    let copy = member_copy(source, member, now)?;
    let id = copy.id.clone();
    personas.push(copy);
    Ok(ResolvedMember { id, retain: true })
}

/// A local built-in that is byte-identical to the published member.
///
/// Substitution requires BOTH the canonical `builtin:<slug>` to exist locally
/// AND the local built-in's projection hash to equal the published
/// `projection_hash`. That published hash is trustworthy here because the
/// parse boundary (`validate_member`) already recomputed it from this member's
/// own embedded fields and rejected the head on any mismatch — so a
/// `projection_hash` reaching this point provably describes the reviewed
/// projection, not an unrelated built-in's definition. A retired slug or a
/// slug whose local definition has drifted still fails the equality test here
/// and falls through to an ordinary copy built from the embedded
/// (authoritative) fields.
fn reusable_builtin(personas: &[AgentDefinition], member: &TeamCatalogMember) -> Option<String> {
    let slug = member.builtin_slug.as_deref()?;
    let published_hash = member.projection_hash.as_deref()?;
    personas
        .iter()
        .find(|persona| {
            builtin_catalog_slug(persona) == Some(slug)
                && local_member_projection_hash(persona).eq_ignore_ascii_case(published_hash)
        })
        .map(|persona| persona.id.clone())
}

/// Whether a local persona is a copy of exactly this published member.
///
/// All four components must match. Dropping `projection_hash` would collapse
/// two versions of one published member onto a single mutable local record, so
/// adding the newer team would silently rewrite the copy the older team uses.
fn member_provenance_matches(
    persona: &AgentDefinition,
    source: &TeamCatalogSource,
    member: &TeamCatalogMember,
) -> bool {
    persona.team_catalog_source.as_ref().is_some_and(|held| {
        held.owner_pubkey == source.owner_pubkey
            && held.team_d_tag == source.team_d_tag
            && held.member_key == member.member_key
            && held.projection_hash == member_version_hash(member)
    })
}

/// The version stamp stored on a copy.
///
/// A publisher-supplied `projection_hash` is present only on built-in reuse
/// hints and is publisher-controlled either way, so it cannot serve as the
/// version for ordinary members. Recomputing it locally over the member as
/// published makes the stamp mean "this exact projection" for every member.
fn member_version_hash(member: &TeamCatalogMember) -> String {
    use sha2::{Digest, Sha256};
    let json = serde_json::to_vec(member).unwrap_or_default();
    hex::encode(Sha256::digest(&json))
}

/// Build a local persona from a published member's embedded fields.
///
/// Embedding is authoritative: every field comes from the projection, never
/// from a local record that shares a name. Fields absent from the projection
/// by design — env vars, allowlist pubkeys — are absent here too, so a copy
/// starts with no inherited secrets and no inherited audience.
fn member_copy(
    source: &TeamCatalogSource,
    member: &TeamCatalogMember,
    now: &str,
) -> Result<AgentDefinition, String> {
    Ok(AgentDefinition {
        id: Uuid::new_v4().to_string(),
        display_name: member.display_name.clone(),
        // Team catalog members carry no public description; an adopted copy
        // starts without one.
        description: None,
        avatar_url: member.avatar_url.clone(),
        system_prompt: member.system_prompt.clone().unwrap_or_default(),
        runtime: member.runtime.clone(),
        model: member.model.clone(),
        provider: member.provider.clone(),
        name_pool: member.name_pool.clone(),
        is_builtin: false,
        is_active: true,
        shared: false,
        source_team: None,
        source_team_persona_slug: None,
        catalog_source: None,
        team_catalog_source: Some(TeamMemberCatalogSource {
            owner_pubkey: source.owner_pubkey.clone(),
            team_d_tag: source.team_d_tag.clone(),
            member_key: member.member_key.clone(),
            projection_hash: member_version_hash(member),
        }),
        env_vars: Default::default(),
        // Validated at the boundary rather than copied opaquely: an
        // unrecognized mode from a foreign publisher must not become a local
        // definition whose audience differs from what the recipient sees.
        // `allowlist` is normalized to `owner-only`: allowlist pubkeys are
        // never published (privacy), so adopting `allowlist` with an empty
        // allowlist would mint a persona that fails at mint time. The recipient
        // can widen from `owner-only` in the edit dialog.
        respond_to: member
            .respond_to
            .as_deref()
            .map(|mode| -> Result<Option<String>, String> {
                let parsed =
                    RespondTo::parse_wire(mode).map_err(|e| format!("invalid respond_to: {e}"))?;
                if parsed == RespondTo::Allowlist {
                    Ok(Some(RespondTo::OwnerOnly.as_str().to_string()))
                } else {
                    Ok(Some(mode.to_string()))
                }
            })
            .transpose()?
            .flatten(),
        respond_to_allowlist: Vec::new(),
        parallelism: member.parallelism,
        created_at: now.to_string(),
        updated_at: now.to_string(),
    })
}
