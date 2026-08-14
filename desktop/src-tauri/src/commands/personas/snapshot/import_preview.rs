use serde::Serialize;

use crate::managed_agents::agent_snapshot::{AgentSnapshot, MemoryLevel};

/// Materialized preview returned to the UI before any write is committed.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentSnapshotImportPreview {
    pub display_name: String,
    pub is_builtin: bool,
    pub model: Option<String>,
    pub runtime: Option<String>,
    pub provider: Option<String>,
    pub system_prompt: Option<String>,
    pub avatar_url: Option<String>,
    pub source_avatar_url: Option<String>,
    pub name_pool: Vec<String>,
    pub respond_to: Option<String>,
    pub parallelism: Option<u32>,
    pub memory_level: String,
    pub memory_entry_count: usize,
    pub has_source_allowlist: bool,
    pub source_allowlist_count: usize,
    pub source_allowlist: Vec<String>,
    pub manifest_json: String,
    pub locked: bool,
}

pub(crate) fn build_agent_snapshot_import_preview(
    snapshot: &AgentSnapshot,
    locked: bool,
) -> Result<AgentSnapshotImportPreview, String> {
    let memory_level = match snapshot.memory.level {
        MemoryLevel::None => "none",
        MemoryLevel::Core => "core",
        MemoryLevel::Everything => "everything",
    }
    .to_string();
    let manifest_json = serde_json::to_string_pretty(snapshot)
        .map_err(|e| format!("failed to render snapshot manifest: {e}"))?;
    let source_allowlist = snapshot.definition.respond_to_allowlist.clone();

    Ok(AgentSnapshotImportPreview {
        display_name: snapshot.profile.display_name.clone(),
        is_builtin: snapshot.definition.source_is_builtin,
        model: snapshot.definition.model.clone(),
        runtime: snapshot.definition.runtime.clone(),
        provider: snapshot.definition.provider.clone(),
        system_prompt: snapshot.definition.system_prompt.clone(),
        avatar_url: snapshot
            .profile
            .avatar_data_url
            .clone()
            .or_else(|| snapshot.profile.avatar_url.clone()),
        source_avatar_url: snapshot.profile.avatar_url.clone(),
        name_pool: snapshot.definition.name_pool.clone(),
        respond_to: snapshot.definition.respond_to.clone(),
        parallelism: snapshot.definition.parallelism,
        memory_level,
        memory_entry_count: snapshot.memory.entries.len(),
        source_allowlist_count: source_allowlist.len(),
        has_source_allowlist: !source_allowlist.is_empty(),
        source_allowlist,
        manifest_json,
        locked,
    })
}
