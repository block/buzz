//! One-time data repair for teams created before PR 852.
//!
//! Backfills `TeamRecord.source_dir` and deduplicates `PersonaRecord`s that
//! share a `(source_team, source_team_persona_slug)` pair — the result of
//! repeated imports before the matching predicate was fixed.
//!
//! All repairs touch only JSON records; the symlinked source directory is read
//! but never written.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::Path;

use tauri::AppHandle;

use crate::managed_agents::{ManagedAgentRecord, PersonaRecord, TeamRecord};

/// Derive the shared key used to match personas to a team. For directory-
/// backed teams this is the directory name (the pack manifest ID); for others
/// it falls back to `team.id`. This bridges the mismatch where legacy teams
/// have a UUID `id` but their personas store the manifest ID in `source_team`.
///
/// Note: the `team.id` fallback namespace (UUIDs) is near-disjoint from
/// manifest IDs (dotted reverse-domain), so collisions are near-zero
/// probability. Documented, not fixed.
pub(super) fn team_persona_key(team: &TeamRecord) -> &str {
    team.source_dir
        .as_deref()
        .and_then(|dir| dir.file_name())
        .and_then(|name| name.to_str())
        .unwrap_or(&team.id)
}

/// Backfill `TeamRecord.source_dir` for directory-backed teams created before
/// the field existed. Scans team persona_ids, finds the `source_team` value
/// (manifest ID), checks if that directory exists under `teams_dir`, and sets
/// `team.source_dir` to that path. Respects symlinks (reads but never writes).
///
/// Returns `true` if any team was modified.
pub(super) fn backfill_source_dirs(
    teams: &mut [TeamRecord],
    personas: &[PersonaRecord],
    teams_dir: &Path,
) -> bool {
    let mut changed = false;

    for team in teams.iter_mut() {
        if team.is_builtin || team.source_dir.as_ref().is_some_and(|d| d.exists()) {
            continue;
        }

        // The directory name the team's personas point at. All personas of one
        // team share a single source_team value (the manifest ID).
        let Some(dir_name) = team
            .persona_ids
            .iter()
            .find_map(|id| personas.iter().find(|p| p.id == *id))
            .and_then(|p| p.source_team.clone())
        else {
            continue;
        };

        let candidate = teams_dir.join(&dir_name);
        if candidate.exists() {
            team.is_symlink = fs::symlink_metadata(&candidate)
                .map(|m| m.file_type().is_symlink())
                .unwrap_or(false);
            team.symlink_target = if team.is_symlink {
                fs::canonicalize(&candidate)
                    .ok()
                    .map(|p| p.display().to_string())
            } else {
                None
            };
            team.source_dir = Some(candidate);
            changed = true;
        }
    }

    changed
}

/// Deduplicate `PersonaRecord`s that share a `(source_team, source_team_persona_slug)`
/// pair — the result of repeated imports before the matching predicate was fixed.
///
/// The winner is chosen by `updated_at` descending, then `id` ascending. Loser
/// IDs are repointed to the winner in `TeamRecord.persona_ids` and
/// `ManagedAgentRecord.persona_id`, then the losers are dropped.
///
/// After dropping losers, performs a self-healing scrub: any reference in
/// `team.persona_ids` or `agent.persona_id` that points to a persona ID not
/// in the surviving set is removed. This makes the migration convergent under
/// any crash interleaving without a transaction.
///
/// Returns `true` if anything changed.
pub(super) fn dedup_personas(
    personas: &mut Vec<PersonaRecord>,
    teams: &mut [TeamRecord],
    agents: &mut [ManagedAgentRecord],
) -> bool {
    // Group indices by (source_team, slug); only personas with both set collide.
    let mut groups: HashMap<(String, String), Vec<usize>> = HashMap::new();
    for (idx, p) in personas.iter().enumerate() {
        if let (Some(team), Some(slug)) = (&p.source_team, &p.source_team_persona_slug) {
            groups
                .entry((team.clone(), slug.clone()))
                .or_default()
                .push(idx);
        }
    }

    // loser_id -> winner_id
    let mut remap: HashMap<String, String> = HashMap::new();
    for indices in groups.values() {
        if indices.len() < 2 {
            continue;
        }
        let mut ranked: Vec<usize> = indices.clone();
        ranked.sort_by(|&a, &b| {
            personas[b]
                .updated_at
                .cmp(&personas[a].updated_at)
                .then_with(|| personas[a].id.cmp(&personas[b].id))
        });
        let winner_id = personas[ranked[0]].id.clone();
        for &loser_idx in &ranked[1..] {
            let loser_id = personas[loser_idx].id.clone();
            eprintln!("dedup: persona {loser_id} merged into {winner_id}");
            remap.insert(loser_id, winner_id.clone());
        }
    }

    // Repoint references before dropping losers.
    if !remap.is_empty() {
        for team in teams.iter_mut() {
            let mut seen = HashSet::new();
            team.persona_ids = std::mem::take(&mut team.persona_ids)
                .into_iter()
                .map(|id| remap.get(&id).cloned().unwrap_or(id))
                .filter(|id| seen.insert(id.clone()))
                .collect();
        }
        for agent in agents.iter_mut() {
            if let Some(id) = &agent.persona_id {
                if let Some(winner) = remap.get(id) {
                    agent.persona_id = Some(winner.clone());
                }
            }
        }

        personas.retain(|p| !remap.contains_key(&p.id));
    }

    // Self-healing scrub: remove any references that point to persona IDs not
    // in the surviving set. This handles the crash-window case where a prior
    // partial run dropped losers from personas.json but never repointed the
    // references in teams.json / managed-agents.json.
    let surviving_ids: HashSet<&str> = personas.iter().map(|p| p.id.as_str()).collect();
    let mut scrubbed = false;

    for team in teams.iter_mut() {
        let before_len = team.persona_ids.len();
        team.persona_ids
            .retain(|id| surviving_ids.contains(id.as_str()));
        if team.persona_ids.len() != before_len {
            eprintln!(
                "dedup: scrubbed {} dangling persona_ids from team {}",
                before_len - team.persona_ids.len(),
                team.id
            );
            scrubbed = true;
        }
    }
    for agent in agents.iter_mut() {
        if let Some(id) = &agent.persona_id {
            if !surviving_ids.contains(id.as_str()) {
                eprintln!(
                    "dedup: scrubbed dangling persona_id {} from agent {}",
                    id, agent.name
                );
                agent.persona_id = None;
                scrubbed = true;
            }
        }
    }

    !remap.is_empty() || scrubbed
}

/// Sync all directory-backed teams on launch — the team equivalent of the
/// former `sync_pack_personas`. Runs the one-time backfill + dedup repair,
/// then re-syncs each team from its source directory. Silently skips teams
/// whose source directory is missing (e.g., external drive unmounted).
pub fn sync_team_personas(app: &AppHandle) -> Result<(), String> {
    use super::teams::{load_teams, save_teams, sync_team_from_dir, teams_dir};

    // One-time data repair: backfill source_dir for legacy teams and deduplicate
    // personas that were imported multiple times before the predicate fix.
    let teams_base = teams_dir(app)?;
    let mut teams = load_teams(app)?;
    let mut personas = super::load_personas(app)?;
    let mut agents = super::load_managed_agents(app)?;

    let backfilled = backfill_source_dirs(&mut teams, &personas, &teams_base);
    let deduped = dedup_personas(&mut personas, &mut teams, &mut agents);

    // Write reference holders (teams, agents) BEFORE the personas they point at.
    // The three saves are individually atomic but not transactional together; a
    // crash before save_personas then leaves references aimed at personas that
    // still exist (over-pointing), never dangling. dedup_personas is also
    // self-healing, so the next launch converges under any crash interleaving.
    if backfilled || deduped {
        save_teams(app, &teams)?;
    }
    if deduped {
        super::save_managed_agents(app, &agents)?;
        super::save_personas(app, &personas)?;
    }

    for team in &teams {
        if team.source_dir.as_ref().is_some_and(|d| d.exists()) {
            if let Err(e) = sync_team_from_dir(app, &team.id) {
                eprintln!("buzz-desktop: sync team {}: {e}", team.id);
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::managed_agents::{ManagedAgentRecord, PersonaRecord, TeamRecord};
    use std::path::PathBuf;
    use tempfile::TempDir;

    fn persona(id: &str, source_team: Option<&str>, slug: Option<&str>) -> PersonaRecord {
        PersonaRecord {
            id: id.to_string(),
            display_name: id.to_string(),
            avatar_url: None,
            system_prompt: String::new(),
            runtime: None,
            model: None,
            provider: None,
            name_pool: Vec::new(),
            is_builtin: false,
            is_active: true,
            source_team: source_team.map(|s| s.to_string()),
            source_team_persona_slug: slug.map(|s| s.to_string()),
            env_vars: Default::default(),
            created_at: "2025-01-01T00:00:00Z".to_string(),
            updated_at: "2025-01-01T00:00:00Z".to_string(),
        }
    }

    fn team(id: &str) -> TeamRecord {
        TeamRecord {
            id: id.to_string(),
            name: id.to_string(),
            description: None,
            persona_ids: Vec::new(),
            is_builtin: false,
            source_dir: None,
            is_symlink: false,
            symlink_target: None,
            version: None,
            created_at: "2026-03-20T00:00:00Z".to_string(),
            updated_at: "2026-03-20T00:00:00Z".to_string(),
        }
    }

    fn agent(name: &str, persona_id: Option<&str>) -> ManagedAgentRecord {
        ManagedAgentRecord {
            pubkey: String::new(),
            name: name.to_string(),
            persona_id: persona_id.map(|s| s.to_string()),
            private_key_nsec: String::new(),
            auth_tag: None,
            relay_url: String::new(),
            avatar_url: None,
            acp_command: String::new(),
            agent_command: String::new(),
            agent_args: vec![],
            mcp_command: String::new(),
            turn_timeout_seconds: 0,
            idle_timeout_seconds: None,
            max_turn_duration_seconds: None,
            parallelism: 1,
            system_prompt: None,
            model: None,
            provider: None,
            persona_source_version: None,
            mcp_toolsets: None,
            start_on_app_launch: false,
            runtime_pid: None,
            backend: Default::default(),
            backend_agent_id: None,
            provider_binary_path: None,
            persona_team_dir: None,
            persona_name_in_team: None,
            created_at: String::new(),
            updated_at: String::new(),
            last_started_at: None,
            last_stopped_at: None,
            last_exit_code: None,
            last_error: None,
            respond_to: Default::default(),
            respond_to_allowlist: vec![],
            env_vars: std::collections::BTreeMap::new(),
            relay_mesh: None,
        }
    }

    // ── team_persona_key ─────────────────────────────────────────────────

    #[test]
    fn team_persona_key_prefers_source_dir_name() {
        let mut t = team("some-uuid");
        t.source_dir = Some(PathBuf::from("/path/to/teams/com.wpfleger.sietch-tabr"));
        assert_eq!(team_persona_key(&t), "com.wpfleger.sietch-tabr");
    }

    #[test]
    fn team_persona_key_falls_back_to_id() {
        let t = team("builtin-team:fizz");
        assert_eq!(team_persona_key(&t), "builtin-team:fizz");
    }

    // ── backfill_source_dirs ─────────────────────────────────────────────

    #[test]
    fn backfill_sets_source_dir_for_legacy_uuid_team() {
        let tmp = TempDir::new().unwrap();
        let teams_dir = tmp.path();
        std::fs::create_dir(teams_dir.join("com.test.pack")).unwrap();

        let mut teams = vec![{
            let mut t = team("uuid-123");
            t.persona_ids = vec!["p1".to_string()];
            t
        }];
        let personas = vec![{
            let mut p = persona("p1", Some("com.test.pack"), Some("scout"));
            p.source_team = Some("com.test.pack".to_string());
            p
        }];

        let changed = backfill_source_dirs(&mut teams, &personas, teams_dir);
        assert!(changed);
        let key = team_persona_key(&teams[0]);
        assert_eq!(key, "com.test.pack");
        assert_eq!(teams[0].source_dir, Some(teams_dir.join("com.test.pack")));
    }

    #[test]
    fn backfill_skips_when_directory_absent() {
        let tmp = TempDir::new().unwrap();
        let teams_dir = tmp.path();
        // Do NOT create the directory

        let mut teams = vec![{
            let mut t = team("uuid-123");
            t.persona_ids = vec!["p1".to_string()];
            t
        }];
        let personas = vec![persona("p1", Some("com.test.pack"), Some("scout"))];

        let changed = backfill_source_dirs(&mut teams, &personas, teams_dir);
        assert!(!changed);
        assert!(teams[0].source_dir.is_none());
    }

    #[test]
    fn backfill_skips_builtin_and_already_set_teams() {
        let tmp = TempDir::new().unwrap();
        let teams_dir = tmp.path();
        std::fs::create_dir(teams_dir.join("com.test.pack")).unwrap();

        let mut builtin = team("builtin-team:fizz");
        builtin.is_builtin = true;
        builtin.persona_ids = vec!["p1".to_string()];

        let mut already_set = team("uuid-456");
        already_set.source_dir = Some(teams_dir.join("com.test.pack"));
        already_set.persona_ids = vec!["p2".to_string()];

        let mut teams = vec![builtin, already_set];
        let personas = vec![
            persona("p1", Some("com.test.pack"), Some("scout")),
            persona("p2", Some("com.test.pack"), Some("kit")),
        ];

        let changed = backfill_source_dirs(&mut teams, &personas, teams_dir);
        assert!(!changed);
    }

    // ── dedup_personas ───────────────────────────────────────────────────

    #[test]
    fn dedup_keeps_newest_and_repoints_references() {
        let mut p_old = persona("old-id", Some("team-a"), Some("scout"));
        p_old.updated_at = "2025-01-01T00:00:00Z".to_string();
        let mut p_new = persona("new-id", Some("team-a"), Some("scout"));
        p_new.updated_at = "2025-06-01T00:00:00Z".to_string();

        let mut personas = vec![p_old, p_new];
        let mut teams = vec![{
            let mut t = team("t1");
            t.persona_ids = vec!["old-id".to_string(), "new-id".to_string()];
            t
        }];
        let mut agents = vec![agent("agent-1", Some("old-id"))];

        let changed = dedup_personas(&mut personas, &mut teams, &mut agents);
        assert!(changed);
        assert_eq!(personas.len(), 1);
        assert_eq!(personas[0].id, "new-id");
        assert_eq!(teams[0].persona_ids, vec!["new-id"]);
        assert_eq!(agents[0].persona_id, Some("new-id".to_string()));
    }

    #[test]
    fn dedup_breaks_ties_by_id_when_updated_at_equal() {
        let p_a = persona("aaa", Some("team-a"), Some("scout"));
        let p_b = persona("bbb", Some("team-a"), Some("scout"));

        let mut personas = vec![p_a, p_b];
        let mut teams = vec![{
            let mut t = team("t1");
            t.persona_ids = vec!["aaa".to_string(), "bbb".to_string()];
            t
        }];
        let mut agents = vec![];

        let changed = dedup_personas(&mut personas, &mut teams, &mut agents);
        assert!(changed);
        assert_eq!(personas.len(), 1);
        // "aaa" < "bbb" lexically, so "aaa" wins the tiebreak
        assert_eq!(personas[0].id, "aaa");
    }

    #[test]
    fn dedup_is_noop_without_duplicates() {
        let mut personas = vec![
            persona("p1", Some("team-a"), Some("scout")),
            persona("p2", Some("team-a"), Some("kit")),
        ];
        let mut teams = vec![];
        let mut agents = vec![];

        let changed = dedup_personas(&mut personas, &mut teams, &mut agents);
        assert!(!changed);
        assert_eq!(personas.len(), 2);
    }

    #[test]
    fn dedup_ignores_personas_without_source_team() {
        let mut personas = vec![
            persona("p1", None, Some("scout")),
            persona("p2", None, Some("scout")),
        ];
        let mut teams = vec![];
        let mut agents = vec![];

        let changed = dedup_personas(&mut personas, &mut teams, &mut agents);
        assert!(!changed);
        assert_eq!(personas.len(), 2);
    }

    #[test]
    fn dedup_heals_dangling_agent_reference_from_prior_crash() {
        // Simulate the crash-window scenario: a prior run dropped the loser
        // persona from personas.json but never repointed managed-agents.json.
        // On this launch, only the winner survives. The dangling reference
        // must be scrubbed.
        let winner = persona("winner-id", Some("team-a"), Some("scout"));
        // The loser is already gone (dropped in prior crash-interrupted run).
        let mut personas = vec![winner];
        let mut teams = vec![{
            let mut t = team("t1");
            // team still references the loser (stale from prior crash)
            t.persona_ids = vec!["winner-id".to_string(), "loser-id".to_string()];
            t
        }];
        let mut agents = vec![agent("agent-1", Some("loser-id"))];

        let changed = dedup_personas(&mut personas, &mut teams, &mut agents);
        // The self-healing scrub should have removed the dangling references
        assert!(changed);
        assert_eq!(teams[0].persona_ids, vec!["winner-id"]);
        assert_eq!(agents[0].persona_id, None);
    }
}
