//! One-time lift of instance-only ACP commands into their definitions.
//!
//! Before definitions carried an ACP command, a custom transport (for example
//! `buzz-janet-acp`) could only be saved on a linked instance. The definition
//! now owns the command for every start path, so an instance-only value would
//! be silently replaced by stock `buzz-acp`. This migration moves such a value
//! up to its definition once, preserving what the agent already ran.
//!
//! Safety rails:
//! - **Once**: a sentinel is written after a successful pass. Later boots never
//!   lift again, so a deliberate reset of the definition to stock is not undone
//!   by a stale instance mirror.
//! - **Unambiguous only**: a definition is lifted only when every linked
//!   instance with a custom command agrees on the same value. Conflicts are
//!   logged and left unchanged.
//! - **Definition choice wins**: a definition that already names a non-stock
//!   command is never modified.
//! - **Backup once** before the first write.
//! - **No false drift**: an instance whose recorded source version matched the
//!   definition before the lift is advanced to the lifted version, because it
//!   was already configured for that command.
//!
//! The next event-sync pass republishes the changed definitions, so the relay
//! head does not revert them.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use crate::managed_agents::{
    persona_events::{persona_content_hash, persona_event_content},
    ManagedAgentRecord, DEFAULT_ACP_COMMAND,
};

fn definition_version(record: &ManagedAgentRecord) -> Option<String> {
    record
        .to_definition_view()
        .map(|view| persona_content_hash(&persona_event_content(&view)))
}

pub(super) const SENTINEL: &str = "acp-command-lift.migrated";

pub fn lift_instance_acp_commands(app: &tauri::AppHandle) {
    let Ok(base_dir) = crate::managed_agents::managed_agents_base_dir(app) else {
        return;
    };
    match lift_instance_acp_commands_in_dir(&base_dir) {
        Ok(0) => {}
        Ok(lifted) => eprintln!(
            "buzz-desktop: acp-command-lift: {lifted} definitions adopted their instance ACP command"
        ),
        Err(e) => eprintln!("buzz-desktop: acp-command-lift: {e}"),
    }
}

fn is_custom(command: &str) -> bool {
    let command = command.trim();
    !command.is_empty() && command != DEFAULT_ACP_COMMAND
}

/// Core logic, decoupled from the Tauri `AppHandle` for testing. Returns the
/// number of definitions changed.
pub(super) fn lift_instance_acp_commands_in_dir(base_dir: &Path) -> Result<usize, String> {
    let sentinel = base_dir.join(SENTINEL);
    if sentinel.exists() {
        return Ok(0);
    }
    let agents_path = base_dir.join("managed-agents.json");
    let lifted = if agents_path.exists() {
        lift_store(&agents_path)?
    } else {
        0
    };
    std::fs::write(&sentinel, "")
        .map_err(|e| format!("failed to write sentinel {}: {e}", sentinel.display()))?;
    Ok(lifted)
}

fn lift_store(agents_path: &Path) -> Result<usize, String> {
    let content = std::fs::read_to_string(agents_path)
        .map_err(|e| format!("failed to read managed-agents.json: {e}"))?;
    let mut all: Vec<ManagedAgentRecord> = serde_json::from_str(&content)
        .map_err(|e| format!("failed to parse managed-agents.json: {e}"))?;

    // Custom commands carried by keyed instances, grouped by linked definition.
    let mut wanted: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for record in all.iter().filter(|r| !r.pubkey.is_empty()) {
        if let Some(persona_id) = record.persona_id.as_deref() {
            if is_custom(&record.acp_command) {
                wanted
                    .entry(persona_id.to_string())
                    .or_default()
                    .insert(record.acp_command.trim().to_string());
            }
        }
    }
    if wanted.is_empty() {
        return Ok(0);
    }

    let mut changes: Vec<(usize, String)> = Vec::new();
    for (index, definition) in all.iter().enumerate() {
        if !definition.pubkey.is_empty() || definition.is_builtin {
            continue;
        }
        let Some(slug) = definition.slug.as_deref() else {
            continue;
        };
        let Some(commands) = wanted.get(slug) else {
            continue;
        };
        if is_custom(&definition.acp_command) {
            continue;
        }
        if commands.len() > 1 {
            eprintln!(
                "buzz-desktop: acp-command-lift: definition {slug} has instances with different \
                 ACP commands ({}); left unchanged — choose one in the definition editor",
                commands.iter().cloned().collect::<Vec<_>>().join(", ")
            );
            continue;
        }
        if let Some(command) = commands.iter().next() {
            changes.push((index, command.clone()));
        }
    }
    if changes.is_empty() {
        return Ok(0);
    }

    let bak_path =
        crate::util::resolved_backup_path(agents_path, "managed-agents.json.pre-acp-lift.bak");
    crate::util::create_restricted_backup_once(&bak_path, content.as_bytes())
        .map_err(|e| format!("failed to write pre-lift backup: {e}"))?;

    let now = crate::util::now_iso();
    for (index, command) in &changes {
        let before = definition_version(&all[*index]);
        let definition = &mut all[*index];
        definition.acp_command = command.clone();
        definition.updated_at = now.clone();
        let after = definition_version(definition);
        let slug = definition.slug.clone();
        if let (Some(before), Some(after)) = (before, after) {
            for instance in all.iter_mut().filter(|r| {
                !r.pubkey.is_empty()
                    && r.persona_id == slug
                    && r.persona_source_version.as_deref() == Some(before.as_str())
            }) {
                instance.persona_source_version = Some(after.clone());
            }
        }
    }
    let payload = serde_json::to_vec_pretty(&all)
        .map_err(|e| format!("failed to serialize unified store: {e}"))?;
    crate::managed_agents::atomic_write_json_restricted(agents_path, &payload)?;
    Ok(changes.len())
}

#[cfg(test)]
#[path = "acp_command_lift_tests.rs"]
mod tests;
