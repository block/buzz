use crate::managed_agents::ManagedAgentRecord;

/// Final write boundary for a newly minted identity. The caller holds the
/// managed-agent store lock, so the binding check and persistence are one
/// critical section. The callback keeps this seam testable while production
/// still uses the real managed-agent store.
pub(super) fn commit_created_managed_agent(
    records: &mut Vec<ManagedAgentRecord>,
    record: ManagedAgentRecord,
    force_new_instance: bool,
    persist: impl FnOnce(&[ManagedAgentRecord]) -> Result<(), String>,
) -> Result<(), String> {
    if !force_new_instance {
        ensure_persona_deployment_identity_available(
            records,
            record.persona_id.as_deref(),
            record.team_id.as_deref(),
        )?;
    }

    records.push(record);
    if let Err(error) = persist(records) {
        records.pop();
        return Err(error);
    }
    Ok(())
}

/// Reject a second live identity for the same persona deployment binding.
/// Unbound persona instances converge on one identity; team-bound instances
/// converge per team, so distinct teams can still deploy the same persona with
/// different instructions.
pub(super) fn ensure_persona_deployment_identity_available(
    records: &[ManagedAgentRecord],
    persona_id: Option<&str>,
    team_id: Option<&str>,
) -> Result<(), String> {
    let Some(persona_id) = persona_id else {
        return Ok(());
    };

    if let Some(existing) = records.iter().find(|record| {
        record.is_active
            && record.persona_id.as_deref() == Some(persona_id)
            && record.team_id.as_deref() == team_id
    }) {
        let deployment = team_id
            .map(|team_id| format!("team {team_id}"))
            .unwrap_or_else(|| "the unbound deployment".to_string());
        return Err(format!(
            "{deployment} already has agent {} for persona {persona_id}; reuse that identity",
            existing.pubkey
        ));
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Barrier, Mutex};
    use std::thread;

    fn record(pubkey: &str, persona_id: Option<&str>, team_id: Option<&str>) -> ManagedAgentRecord {
        serde_json::from_value(serde_json::json!({
            "pubkey": pubkey,
            "name": "Agent",
            "persona_id": persona_id,
            "team_id": team_id,
            "private_key_nsec": "nsec1fake",
            "relay_url": "wss://localhost:3000",
            "acp_command": "buzz-acp",
            "agent_command": "goose",
            "agent_args": [],
            "mcp_command": "",
            "turn_timeout_seconds": 320,
            "system_prompt": null,
            "model": null,
            "provider": null,
            "env_vars": {},
            "created_at": "2026-01-01T00:00:00Z",
            "updated_at": "2026-01-01T00:00:00Z",
            "last_started_at": null,
            "last_stopped_at": null,
            "last_exit_code": null,
            "last_error": null
        }))
        .expect("managed agent record")
    }

    #[test]
    fn repeated_team_persona_deployment_is_rejected() {
        let records = vec![record("existing", Some("persona-a"), Some("team-a"))];

        let error = ensure_persona_deployment_identity_available(
            &records,
            Some("persona-a"),
            Some("team-a"),
        )
        .expect_err("a retry must not mint another identity");

        assert!(error.contains("existing"));
        assert!(error.contains("reuse that identity"));
    }

    #[test]
    fn distinct_teams_can_deploy_the_same_persona() {
        let records = vec![record("existing", Some("persona-a"), Some("team-a"))];

        ensure_persona_deployment_identity_available(&records, Some("persona-a"), Some("team-b"))
            .expect("a different team gets its own identity");
    }

    #[test]
    fn standalone_creation_is_not_constrained_by_team_identity() {
        let records = vec![record("existing", Some("persona-a"), Some("team-a"))];

        ensure_persona_deployment_identity_available(&records, Some("persona-a"), None)
            .expect("a team-bound identity does not block an unbound deployment");

        let unbound = vec![record("unbound", Some("persona-a"), None)];
        let error = ensure_persona_deployment_identity_available(&unbound, Some("persona-a"), None)
            .expect_err("an unbound retry must not mint another identity");
        assert!(error.contains("unbound"));
    }

    #[test]
    fn inactive_identity_does_not_block_replacement() {
        let mut existing = record("existing", Some("persona-a"), Some("team-a"));
        existing.is_active = false;

        ensure_persona_deployment_identity_available(
            &[existing],
            Some("persona-a"),
            Some("team-a"),
        )
        .expect("an inactive identity can be replaced");
    }

    #[test]
    fn concurrent_retries_persist_one_stable_identity() {
        let records = Arc::new(Mutex::new(Vec::<ManagedAgentRecord>::new()));
        let persisted = Arc::new(Mutex::new(Vec::<ManagedAgentRecord>::new()));
        let barrier = Arc::new(Barrier::new(3));

        let handles = ["first", "second"].map(|pubkey| {
            let records = Arc::clone(&records);
            let persisted = Arc::clone(&persisted);
            let barrier = Arc::clone(&barrier);
            thread::spawn(move || {
                let candidate = record(pubkey, Some("persona-a"), Some("team-a"));
                barrier.wait();
                let mut records = records.lock().expect("record lock");
                commit_created_managed_agent(&mut records, candidate, false, |next| {
                    *persisted.lock().expect("persistence lock") = next.to_vec();
                    Ok(())
                })
            })
        });

        barrier.wait();
        let results = handles.map(|handle| handle.join().expect("create thread"));
        assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 1);
        assert_eq!(results.iter().filter(|result| result.is_err()).count(), 1);

        let stored = persisted.lock().expect("persistence lock");
        assert_eq!(stored.len(), 1);
        assert_eq!(stored[0].persona_id.as_deref(), Some("persona-a"));
        assert_eq!(stored[0].team_id.as_deref(), Some("team-a"));
        assert!(stored[0].pubkey == "first" || stored[0].pubkey == "second");
        assert_eq!(
            records.lock().expect("record lock")[0].pubkey,
            stored[0].pubkey
        );
    }

    #[test]
    fn persistence_failure_rolls_back_in_memory_record() {
        let mut records = Vec::new();
        let error = commit_created_managed_agent(
            &mut records,
            record("candidate", Some("persona-a"), Some("team-a")),
            false,
            |_| Err("disk full".to_string()),
        )
        .expect_err("persistence failure must propagate");

        assert_eq!(error, "disk full");
        assert!(records.is_empty());
    }
}
