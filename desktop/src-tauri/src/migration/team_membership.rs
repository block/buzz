//! Repair team↔member links that a membership edit failed to propagate.
//!
//! Two independent defects, both rooted in a team-membership change not
//! reaching the records that depend on it, are healed in one pass over
//! `teams.json` + `managed-agents.json`:
//!
//! 1. **Stale `persona_ids`.** Team records written before persona ids were
//!    namespaced hold bare slugs (`thufir`) instead of the namespaced id
//!    (`sietch-tabr:thufir`). Nothing rewrites them, and the interactive save
//!    path (`ensure_persona_ids_are_active`) *drops* an id it cannot resolve —
//!    silently shrinking the team. This migration rewrites a stale id to the
//!    persona it names whenever that persona is unambiguous, and — unlike the
//!    save path — never drops one it cannot resolve.
//!
//! 2. **Orphaned or stale instance `team_id`.** Team instructions are injected
//!    at spawn by matching `record.team_id`
//!    (`spawn_snapshot::effective_team_instructions`), so an instance's binding
//!    must track its persona's membership. Two ways it drifts: adding a persona
//!    to a team does not backfill `team_id` on that persona's already-running
//!    instances (a member in the roster but not in behavior), and removing a
//!    persona while keeping its agents leaves the binding pointing at a team
//!    that no longer lists it (still drawing that team's instructions). This
//!    backfills an unset binding and heals a stale one — always on the same
//!    single-team evidence rule, never guessing across teams.
//!
//! The stale-id rewrite is strictly additive (rewrite-or-leave); the binding
//! repair converges to a fixed point (bound-to-a-listing-team or unbound), so a
//! second boot is a clean no-op either way. Runs BEFORE
//! `detach_directory_backed_teams` so a not-yet-detached directory-backed team
//! can still be scoped by its `source_dir`, and before any UI save can drop an
//! unresolvable id.

use std::collections::HashMap;
use std::path::Path;

use crate::managed_agents::{team_persona_key, ManagedAgentRecord, TeamRecord};

/// Repair stale team `persona_ids`/instance `team_id`, then detach
/// directory-backed teams — but only when the repair succeeded.
///
/// `repair` clears no `source_dir`; the downstream detach does. A stale bare
/// slug shared across source teams is disambiguated by `source_dir`, so if
/// repair fails (its backup or write errored) and detach still ran, the next
/// boot would see only ambiguous candidates and the original membership-loss
/// path recurs. Gating detach on a clean repair preserves `source_dir` as retry
/// evidence for that boot; the next boot retries repair and, once clean,
/// detaches.
pub(super) fn repair_then_detach_teams(app: &tauri::AppHandle) {
    let Ok(base_dir) = crate::managed_agents::managed_agents_base_dir(app) else {
        return;
    };
    orchestrate_repair_then_detach(
        || repair_team_membership_in_dir(&base_dir),
        || super::detach::detach_directory_backed_teams_in_dir(&base_dir),
    );
}

/// Gate `detach` on a successful `repair`: run detach only when repair returned
/// `Ok`. Injected ops keep the gate `AppHandle`-free so a failing repair's
/// skip-detach behavior is unit-testable without a filesystem fault.
fn orchestrate_repair_then_detach(
    repair: impl FnOnce() -> Result<usize, String>,
    detach: impl FnOnce() -> Result<usize, String>,
) {
    match repair() {
        Ok(repaired) => {
            if repaired > 0 {
                eprintln!("buzz-desktop: team-membership-repair: repaired {repaired} record(s)");
            }
            match detach() {
                Ok(0) => {}
                Ok(n) => {
                    eprintln!(
                        "buzz-desktop: detach-dir-teams: detached {n} directory-backed team(s)"
                    )
                }
                Err(e) => eprintln!("buzz-desktop: detach-dir-teams: {e}"),
            }
        }
        Err(e) => eprintln!(
            "buzz-desktop: team-membership-repair: {e} — skipping directory-backed detach this \
             boot to preserve source_dir for a clean-repair retry"
        ),
    }
}

/// Core logic, decoupled from the Tauri `AppHandle` for testing.
///
/// `base_dir` is the managed-agents base directory (`<AppDataDir>/agents/`).
/// Returns the number of records changed across both files (0 = nothing to do,
/// nothing written, so a re-run is a clean no-op).
pub(super) fn repair_team_membership_in_dir(base_dir: &Path) -> Result<usize, String> {
    let teams_path = base_dir.join("teams.json");
    let agents_path = base_dir.join("managed-agents.json");

    // Definitions and teams both live in these two files; without either there
    // is nothing to link.
    if !teams_path.exists() || !agents_path.exists() {
        return Ok(0);
    }

    let teams_content = std::fs::read_to_string(&teams_path)
        .map_err(|e| format!("failed to read teams.json: {e}"))?;
    let mut teams: Vec<TeamRecord> = serde_json::from_str(&teams_content)
        .map_err(|e| format!("failed to parse teams.json: {e}"))?;

    let agents_content = std::fs::read_to_string(&agents_path)
        .map_err(|e| format!("failed to read managed-agents.json: {e}"))?;
    let mut agents: Vec<ManagedAgentRecord> = serde_json::from_str(&agents_content)
        .map_err(|e| format!("failed to parse managed-agents.json: {e}"))?;

    let rewrites = rewrite_stale_persona_ids(&mut teams, &agents);
    let backfills = backfill_instance_team_ids(&teams, &mut agents);

    if rewrites == 0 && backfills == 0 {
        return Ok(0);
    }

    // Pre-migration backups, both taken BEFORE either live store write: the
    // stated contract is a full recovery pair even if a crash lands between the
    // two writes, so neither store may be rewritten until both pristine backups
    // exist. A stale bare slug shared across source teams is disambiguated by
    // `source_dir`, which the downstream detach clears — so the pristine
    // pre-repair `teams.json` is the evidence a retry needs. Each backup is
    // created once (create-new), so a re-run after a partial failure never
    // overwrites the pristine copy with a half-migrated snapshot.
    if rewrites > 0 {
        let bak = crate::util::resolved_backup_path(
            &teams_path,
            "teams.json.pre-team-membership-repair.bak",
        );
        crate::util::create_restricted_backup_once(&bak, teams_content.as_bytes())
            .map_err(|e| format!("failed to write teams.json backup: {e}"))?;
    }
    if backfills > 0 {
        let bak = crate::util::resolved_backup_path(
            &agents_path,
            "managed-agents.json.pre-team-membership-repair.bak",
        );
        crate::util::create_restricted_backup_once(&bak, agents_content.as_bytes())
            .map_err(|e| format!("failed to write managed-agents.json backup: {e}"))?;
    }

    if rewrites > 0 {
        let payload = serde_json::to_vec_pretty(&teams)
            .map_err(|e| format!("failed to serialize teams.json: {e}"))?;
        crate::managed_agents::atomic_write_json(&teams_path, &payload)?;
    }

    if backfills > 0 {
        // Restricted: this store can carry plaintext agent nsecs on a
        // keyringless host (SECURITY.md:90).
        let payload = serde_json::to_vec_pretty(&agents)
            .map_err(|e| format!("failed to serialize managed-agents.json: {e}"))?;
        crate::managed_agents::atomic_write_json_restricted(&agents_path, &payload)?;
    }

    Ok(rewrites + backfills)
}

/// Set of persona ids that resolve to a definition — the definition records are
/// the key-less unified-store entries (`pubkey == ""`); their `slug` is the id
/// a team references.
fn resolvable_ids(agents: &[ManagedAgentRecord]) -> Vec<&str> {
    agents
        .iter()
        .filter(|r| r.pubkey.is_empty())
        .filter_map(|r| r.slug.as_deref())
        .collect()
}

/// Rewrite each team's stale `persona_ids` to the persona they name, when
/// unambiguous. Returns the number of ids rewritten.
///
/// An id is *stale* when no definition slug equals it. Its repair target is the
/// definition whose `source_team_persona_slug` equals the stale id — i.e. the
/// bare slug is the pre-namespacing form of that persona's namespaced slug. The
/// rewrite happens only when exactly one such definition exists (optionally
/// scoped to the team's source team); zero or many candidates leave the id
/// untouched, which is strictly safer than the save path that drops it.
fn rewrite_stale_persona_ids(teams: &mut [TeamRecord], agents: &[ManagedAgentRecord]) -> usize {
    let resolvable = resolvable_ids(agents);
    let definitions: Vec<&ManagedAgentRecord> =
        agents.iter().filter(|r| r.pubkey.is_empty()).collect();

    let mut rewritten = 0usize;
    for team in teams.iter_mut() {
        // Scope candidate personas to this team's source team when derivable:
        // a directory-backed team keys off its source_dir name; a detached team
        // keys off the unique source_team of its already-resolvable members.
        let scope = team_source_scope(team, &definitions);
        for id in team.persona_ids.iter_mut() {
            if resolvable.contains(&id.as_str()) {
                continue;
            }
            let candidates: Vec<&&ManagedAgentRecord> = definitions
                .iter()
                .filter(|d| d.source_team_persona_slug.as_deref() == Some(id.as_str()))
                .filter(|d| match scope.as_deref() {
                    Some(team_key) => d.source_team.as_deref() == Some(team_key),
                    None => true,
                })
                .collect();
            let [only] = candidates.as_slice() else {
                eprintln!(
                    "buzz-desktop: team-membership-repair: team {:?}: leaving unresolvable \
                     persona id {:?} ({} candidate(s))",
                    team.id,
                    id,
                    candidates.len()
                );
                continue;
            };
            if let Some(slug) = only.slug.as_deref() {
                *id = slug.to_string();
                rewritten += 1;
            }
        }
    }
    rewritten
}

/// The source-team key that scopes a team's persona candidates, or `None` when
/// it cannot be derived (matching then falls back to a global unique slug).
///
/// Directory-backed teams use `team_persona_key` (the pack manifest id). A
/// detached team (`source_dir` cleared) has no such key, so we infer it from
/// the unique `source_team` among its members that already resolve.
fn team_source_scope(team: &TeamRecord, definitions: &[&ManagedAgentRecord]) -> Option<String> {
    if team.source_dir.is_some() {
        return Some(team_persona_key(team).to_string());
    }
    let mut source_teams: Vec<&str> = team
        .persona_ids
        .iter()
        .filter_map(|id| {
            definitions
                .iter()
                .find(|d| d.slug.as_deref() == Some(id.as_str()))
                .and_then(|d| d.source_team.as_deref())
        })
        .collect();
    source_teams.sort_unstable();
    source_teams.dedup();
    match source_teams.as_slice() {
        [only] => Some((*only).to_string()),
        _ => None,
    }
}

/// Repair instance `team_id` against the current rosters. Returns the number of
/// instances changed.
///
/// Two directions, both conservative and evidence-gated:
///
/// - **Unbound → bound (backfill).** An instance whose persona is a team member
///   but whose own `team_id` is unset is bound to that team, so it spawns with
///   the team's instructions. Only when the persona belongs to *exactly one*
///   team — a persona spanning several teams has no evidence selecting one
///   (JSON team order is not ownership), so it is left unbound and logged.
/// - **Stale binding → cleared or re-pointed.** An instance bound to a team
///   whose roster no longer lists its persona (a "keep agents" removal left the
///   binding behind, so the kept instance keeps drawing that team's
///   instructions at spawn) is healed: re-pointed when the persona now belongs
///   to exactly one *other* team (same single-evidence rule), otherwise unbound
///   and logged. A binding whose team still lists the persona is authoritative
///   and never touched.
///
/// Idempotent: after a repair every instance is either bound to a team that
/// lists it or unbound with no single-team evidence, so a second pass is a
/// no-op.
fn backfill_instance_team_ids(teams: &[TeamRecord], agents: &mut [ManagedAgentRecord]) -> usize {
    // persona_id → the sole team referencing it, or None once a *distinct*
    // second team is seen (ambiguous → never used as binding evidence). A
    // persona listed twice within one team is not ambiguity — duplicates are
    // not prohibited at the storage boundary (`ensure_persona_ids_are_active`
    // checks existence only; create/update/inbound persist the vector
    // unchanged), so poisoning on a same-team repeat would strand a
    // legitimately single-team instance.
    let mut persona_to_team: HashMap<&str, Option<&str>> = HashMap::new();
    // Team ids that exist in the store, and the (team_id, persona_id) pairs they
    // list. A binding is *stale* only when its team still exists but no longer
    // lists the persona — a binding to an absent team is left alone (it already
    // degrades to no instructions via `effective_team_instructions`, and a
    // deleted team is not this repair's concern).
    let mut team_ids: std::collections::HashSet<&str> = std::collections::HashSet::new();
    let mut membership: std::collections::HashSet<(&str, &str)> = std::collections::HashSet::new();
    for team in teams {
        team_ids.insert(team.id.as_str());
        for persona_id in &team.persona_ids {
            membership.insert((team.id.as_str(), persona_id.as_str()));
            persona_to_team
                .entry(persona_id.as_str())
                .and_modify(|slot| {
                    if slot.is_some_and(|seen| seen != team.id.as_str()) {
                        *slot = None;
                    }
                })
                .or_insert(Some(team.id.as_str()));
        }
    }

    let mut repaired = 0usize;
    for agent in agents.iter_mut() {
        if agent.pubkey.is_empty() {
            continue;
        }
        let Some(persona_id) = agent.persona_id.as_deref() else {
            continue;
        };
        match agent.team_id.as_deref() {
            // Live binding, or a binding to an absent team: leave it. A binding
            // is only stale when its team exists and dropped the persona.
            Some(bound)
                if !team_ids.contains(bound) || membership.contains(&(bound, persona_id)) => {}
            // Stale binding: the still-present bound team dropped this persona.
            // Re-point on single-team evidence, else unbind — never guess.
            Some(_) => match persona_to_team.get(persona_id) {
                Some(Some(team_id)) => {
                    agent.team_id = Some((*team_id).to_string());
                    repaired += 1;
                }
                _ => {
                    eprintln!(
                        "buzz-desktop: team-membership-repair: unbinding instance {:?} — persona \
                         {persona_id:?} left its team's roster with no single-team successor",
                        agent.pubkey
                    );
                    agent.team_id = None;
                    repaired += 1;
                }
            },
            // Unbound: backfill on single-team evidence.
            None => match persona_to_team.get(persona_id) {
                Some(Some(team_id)) => {
                    agent.team_id = Some((*team_id).to_string());
                    repaired += 1;
                }
                Some(None) => eprintln!(
                    "buzz-desktop: team-membership-repair: leaving instance {:?} unbound — persona \
                     {persona_id:?} spans multiple teams",
                    agent.pubkey
                ),
                None => {}
            },
        }
    }
    repaired
}

#[cfg(test)]
#[path = "team_membership_tests.rs"]
mod tests;
