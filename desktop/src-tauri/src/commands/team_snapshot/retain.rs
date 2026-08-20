//! Inline retention for the managed-agent kind:30177 event, extracted from
//! `team_snapshot.rs` to keep that file within the line-count ratchet.

use super::ManagedAgentRecord;

/// Inline retention for the managed-agent kind:30177 event — mirrors
/// `commands::personas::snapshot::import::retain_agent_pending`.
pub(super) fn retain_agent_pending(
    scope: &crate::managed_agents::retention::RetentionScope,
    record: &ManagedAgentRecord,
) {
    use crate::managed_agents::{
        agent_events::{agent_event_content, build_agent_event},
        persona_events::monotonic_created_at,
        retention::{get_retained_event, open_retention_db, retain_event, RetainedEvent},
    };
    use buzz_core_pkg::kind::KIND_MANAGED_AGENT;
    use nostr::JsonUtil;

    let result = (|| -> Result<(), String> {
        let conn = open_retention_db(&scope.db_path)?;
        let content = serde_json::to_string(&agent_event_content(record))
            .map_err(|e| format!("failed to serialize agent content: {e}"))?;
        let (owner_pubkey, event) = {
            let keys = &scope.owner_keys;
            let owner_pubkey = keys.public_key().to_hex();
            let existing =
                get_retained_event(&conn, KIND_MANAGED_AGENT, &owner_pubkey, &record.pubkey)?;
            if existing.as_ref().is_some_and(|row| row.content == content) {
                return Ok(());
            }
            let event = build_agent_event(record)?
                .custom_created_at(monotonic_created_at(existing.map(|row| row.created_at)))
                .sign_with_keys(keys)
                .map_err(|e| format!("failed to sign agent event: {e}"))?;
            (owner_pubkey, event)
        };
        retain_event(
            &conn,
            &RetainedEvent {
                kind: KIND_MANAGED_AGENT,
                pubkey: owner_pubkey,
                d_tag: record.pubkey.clone(),
                content: event.content.to_string(),
                created_at: event.created_at.as_secs() as i64,
                raw_event: event.as_json(),
                pending_sync: true,
            },
        )
    })();
    if let Err(e) = result {
        eprintln!("buzz-desktop: team-snapshot-import retain-agent: {e}");
    }
}
