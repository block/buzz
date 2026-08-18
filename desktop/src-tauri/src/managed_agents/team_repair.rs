use crate::managed_agents::TeamRecord;

/// Derive the shared key used to match personas to a team. For directory-
/// backed teams this is the directory name (the pack manifest ID); for others
/// it falls back to `team.id`. This bridges the mismatch where legacy teams
/// have a UUID `id` but their personas store the manifest ID in `source_team`.
///
/// Note: the `team.id` fallback namespace (UUIDs) is near-disjoint from
/// manifest IDs (dotted reverse-domain), so collisions are near-zero
/// probability. Documented, not fixed.
pub fn team_persona_key(team: &TeamRecord) -> &str {
    team.source_dir
        .as_deref()
        .and_then(|dir| dir.file_name())
        .and_then(|name| name.to_str())
        .unwrap_or(&team.id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::managed_agents::TeamRecord;
    use std::path::PathBuf;

    fn team(id: &str) -> TeamRecord {
        TeamRecord {
            id: id.to_string(),
            name: id.to_string(),
            description: None,
            instructions: None,
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
}
