use crate::managed_agents::{normalize_managed_agent_avatar, ManagedAgentRecord};

pub(super) fn normalize_runtime_avatars(records: &mut [ManagedAgentRecord]) {
    for record in records {
        let command = record
            .runtime
            .as_deref()
            .filter(|value| !value.trim().is_empty())
            .unwrap_or(&record.agent_command);
        record.avatar_url = normalize_managed_agent_avatar(command, record.avatar_url.take());
    }
}
