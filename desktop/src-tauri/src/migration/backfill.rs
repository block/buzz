//! B5 (unified agent model): one-time backfill of standalone agents into
//! definition-linked records. Every keyed record with `persona_id: None`
//! adopts exactly one matching folded definition when possible. Records with
//! no match get a key-less definition manufactured from their own settings;
//! ambiguous records are skipped rather than creating another duplicate.

use std::path::Path;

use crate::managed_agents::{
    persona_events::{persona_content_hash, persona_event_content},
    AgentDefinition, ManagedAgentRecord,
};

/// Link standalone agents to definitions (B5 backfill).
///
/// For each keyed record with `persona_id: None`, adopt one matching key-less
/// definition left by the persona fold. If there is no match, append a key-less
/// definition snapshotting the agent's own config and link the agent to it. If
/// multiple definitions match, skip the record for manual disambiguation.
/// Safety rails (pinned in the B5 review gates):
/// - **Idempotent**: linked records are skipped, so a second run is a no-op.
/// - **`.bak` create-if-absent**: the pre-migration backup is taken once and
///   never clobbered — a partial first run must not replace the pristine
///   backup with a half-migrated snapshot on re-run.
/// - **Unambiguous adoption**: only one active, non-team custom definition
///   whose snapshotted config matches can be adopted; multiple matches are
///   logged and skipped instead of manufacturing a third definition.
/// - **Fail loudly per record**: a record that cannot be backfilled (ambiguous
///   match or slug collision) is logged and skipped; the rest proceed.
/// - **Behavior preservation**: adoption requires compatible persisted
///   harness and spawn config and leaves instance-only fields untouched. A
///   manufactured definition snapshots the record's own values (prompt
///   present-even-if-empty via `to_definition_view`'s `unwrap_or_default`, env
///   COPIED so later instances inherit a working config, quad copied to the
///   definition defaults), keeping `spawn_config_hash` stable. In both paths
///   the record gains `persona_source_version` = the linked definition's
///   content hash so the drift badge starts in sync.
///
/// The manufactured definition's slug is the agent's pubkey: 64-hex passes
/// the NIP-AP slug grammar on both relay and desktop ends, and agent pubkeys
/// are unique, so the coordinate is collision-free by construction.
pub fn backfill_standalone_agents(app: &tauri::AppHandle) {
    let Ok(base_dir) = crate::managed_agents::managed_agents_base_dir(app) else {
        return;
    };
    match backfill_standalone_agents_in_dir(&base_dir) {
        Ok(0) => {}
        Ok(backfilled) => {
            eprintln!(
                "buzz-desktop: standalone-backfill: {backfilled} agents linked to definitions"
            );
        }
        Err(e) => eprintln!("buzz-desktop: standalone-backfill: {e}"),
    }
}

fn runtime_matches(definition: &AgentDefinition, record: &ManagedAgentRecord) -> bool {
    let definition_runtime = definition
        .runtime
        .as_deref()
        .map(str::trim)
        .filter(|runtime| !runtime.is_empty());

    if let Some(override_command) = record
        .agent_command_override
        .as_deref()
        .map(str::trim)
        .filter(|command| !command.is_empty())
    {
        // Snapshot application only clears a known override when the
        // definition names a different known runtime. A matching known pin,
        // a custom command, or a definition without a catalog runtime remains
        // authoritative and keeps the effective harness unchanged.
        if let (Some(definition_runtime), Some(override_runtime)) = (
            definition_runtime.and_then(crate::managed_agents::known_acp_runtime_exact),
            crate::managed_agents::known_acp_runtime(override_command),
        ) {
            return definition_runtime.id == override_runtime.id;
        }
        return true;
    }

    // Pre-unified standalone records have neither `runtime` nor an override;
    // their persisted `agent_command` is the only record of the harness they
    // were created to run. The current general resolver intentionally ignores
    // that legacy snapshot, but this migration must use it to recognize the
    // just-folded definition that originally produced the agent.
    let legacy_command = record.agent_command.trim();
    let before_command =
        if record.runtime.is_none() && record.persona_id.is_none() && !legacy_command.is_empty() {
            legacy_command.to_string()
        } else {
            crate::managed_agents::record_agent_command(record, &[])
        };
    let default_command = crate::managed_agents::default_agent_command();
    let after_command = definition_runtime
        .and_then(crate::managed_agents::known_acp_runtime_exact)
        .and_then(|runtime| runtime.commands.first().copied())
        .unwrap_or(default_command.as_str());
    match (
        crate::managed_agents::known_acp_runtime(&before_command),
        crate::managed_agents::known_acp_runtime(after_command),
    ) {
        (Some(before), Some(after)) => before.id == after.id,
        _ => before_command == after_command,
    }
}

fn effective_env_matches(definition: &AgentDefinition, record: &ManagedAgentRecord) -> bool {
    let without_definition =
        crate::managed_agents::merged_user_env(&Default::default(), &record.env_vars);
    let with_definition =
        crate::managed_agents::merged_user_env(&definition.env_vars, &record.env_vars);
    without_definition == with_definition
}

fn behavior_defaults_match(definition: &AgentDefinition, record: &ManagedAgentRecord) -> bool {
    let mode_matches = definition
        .respond_to
        .as_deref()
        .map(|mode| mode == record.respond_to.as_str())
        .unwrap_or(true);
    let allowlist_matches = definition.respond_to.as_deref()
        != Some(crate::managed_agents::RespondTo::Allowlist.as_str())
        || definition.respond_to_allowlist == record.respond_to_allowlist;
    let parallelism_matches = definition
        .parallelism
        .map(|parallelism| parallelism == record.parallelism)
        .unwrap_or(true);

    mode_matches && allowlist_matches && parallelism_matches
}

fn snapshot_field_matches(definition_value: Option<&str>, record_value: Option<&str>) -> bool {
    definition_value
        .filter(|value| !value.trim().is_empty())
        .map(|value| record_value == Some(value))
        .unwrap_or(true)
}

fn folded_definition_matches(definition: &AgentDefinition, record: &ManagedAgentRecord) -> bool {
    !definition.is_builtin
        && definition.is_active
        && definition.source_team.is_none()
        && definition.source_team_persona_slug.is_none()
        // Folded definitions cannot represent relay-mesh instance metadata.
        && record.relay_mesh.is_none()
        && definition.display_name
            == record
                .display_name
                .as_deref()
                .unwrap_or(record.name.as_str())
        && definition.system_prompt == record.system_prompt.as_deref().unwrap_or_default()
        // Persona folding can retain a data URI after agent creation has
        // uploaded that avatar and stored its media URL. Snapshot application
        // never overwrites the instance avatar, so it is not an identity key.
        && runtime_matches(definition, record)
        // Snapshot application preserves the record when either optional
        // definition field is absent or blank.
        && snapshot_field_matches(definition.model.as_deref(), record.model.as_deref())
        && snapshot_field_matches(definition.provider.as_deref(), record.provider.as_deref())
        // Definition env is layered under per-instance overrides. Only adopt
        // when activating that lower layer leaves the effective env unchanged.
        && effective_env_matches(definition, record)
        && (record.name_pool.is_empty() || definition.name_pool == record.name_pool)
        // These fields were absent before B5, so absence means unknown. Once a
        // definition carries explicit defaults they help disambiguate identity.
        && behavior_defaults_match(definition, record)
}

enum FoldedDefinitionMatch<'a> {
    NoMatch,
    Unique(&'a AgentDefinition),
    Ambiguous,
}

fn matching_definition<'a>(
    definitions: &'a [AgentDefinition],
    record: &ManagedAgentRecord,
) -> FoldedDefinitionMatch<'a> {
    let mut matches = definitions
        .iter()
        .filter(|definition| folded_definition_matches(definition, record));
    let Some(matching) = matches.next() else {
        return FoldedDefinitionMatch::NoMatch;
    };
    if matches.next().is_some() {
        FoldedDefinitionMatch::Ambiguous
    } else {
        FoldedDefinitionMatch::Unique(matching)
    }
}

/// Core backfill logic, decoupled from the Tauri `AppHandle` for testing.
/// Returns the number of records backfilled (0 = nothing to do).
fn backfill_standalone_agents_in_dir(base_dir: &Path) -> Result<usize, String> {
    let agents_path = base_dir.join("managed-agents.json");
    if !agents_path.exists() {
        return Ok(0);
    }
    let content = std::fs::read_to_string(&agents_path)
        .map_err(|e| format!("failed to read managed-agents.json: {e}"))?;
    let mut all: Vec<ManagedAgentRecord> = serde_json::from_str(&content)
        .map_err(|e| format!("failed to parse managed-agents.json: {e}"))?;

    let needs_backfill =
        |record: &ManagedAgentRecord| !record.pubkey.is_empty() && record.persona_id.is_none();
    if !all.iter().any(needs_backfill) {
        return Ok(0);
    }

    // Pre-migration backup, taken ONCE: a re-run after a partial failure must
    // not overwrite the pristine backup with a half-migrated snapshot.
    let bak_path = base_dir.join("managed-agents.json.pre-backfill.bak");
    if !bak_path.exists() {
        std::fs::write(&bak_path, &content)
            .map_err(|e| format!("failed to write pre-backfill backup: {e}"))?;
    }

    let existing_slugs: std::collections::HashSet<String> =
        all.iter().filter_map(|r| r.slug.clone()).collect();
    let existing_definitions: Vec<AgentDefinition> = all
        .iter()
        .filter(|record| record.pubkey.is_empty())
        .filter_map(ManagedAgentRecord::to_definition_view)
        .collect();

    let mut manufactured: Vec<ManagedAgentRecord> = Vec::new();
    let mut backfilled = 0usize;
    for record in all.iter_mut().filter(|r| needs_backfill(r)) {
        match matching_definition(&existing_definitions, record) {
            FoldedDefinitionMatch::Unique(definition) => {
                record.persona_id = Some(definition.id.clone());
                record.persona_source_version =
                    Some(persona_content_hash(&persona_event_content(definition)));
                backfilled += 1;
                continue;
            }
            FoldedDefinitionMatch::Ambiguous => {
                eprintln!(
                    "buzz-desktop: standalone-backfill: multiple folded definitions match agent {} \
                     — skipped; disambiguate the definitions to let the next launch backfill it",
                    record.pubkey
                );
                continue;
            }
            FoldedDefinitionMatch::NoMatch => {}
        }

        // Pubkeys are unique so this cannot fire against another manufactured
        // definition — only against a pre-existing definition improbably
        // slugged as this agent's pubkey. Fail loudly, skip, continue: the
        // agent keeps working persona-less (`persona_id: None`), and the
        // backfill retries it on every boot. Recovery path: delete or re-slug
        // the colliding definition, then relaunch.
        if existing_slugs.contains(&record.pubkey) {
            eprintln!(
                "buzz-desktop: standalone-backfill: slug collision for agent {} — skipped; \
                 delete or re-slug the colliding definition to let the next launch backfill it",
                record.pubkey
            );
            continue;
        }

        // Snapshot the record's own config as a definition. Via the same
        // fold path every definition takes: a temporary persona view of the
        // record (prompt unwrap_or_default = present-even-if-empty — the
        // heal source old devices hard-require) folded into a key-less
        // definition record. Quad + env come along so future instances
        // minted from this definition inherit a working config.
        let mut view_source = record.clone();
        view_source.slug = Some(record.pubkey.clone());
        // Standalone agents have no definition-level quad — the INSTANCE
        // fields are the author's intent; copy them up.
        view_source.definition_respond_to = Some(record.respond_to.as_str().to_string());
        view_source.definition_respond_to_allowlist = record.respond_to_allowlist.clone();
        view_source.definition_parallelism = Some(record.parallelism);
        let Some(persona_view) = view_source.to_definition_view() else {
            eprintln!(
                "buzz-desktop: standalone-backfill: agent {} produced no persona view — skipped",
                record.pubkey
            );
            continue;
        };

        // Link the record BEFORE computing the version so the hash covers the
        // definition exactly as manufactured.
        let source_version = persona_content_hash(&persona_event_content(&persona_view));
        let definition = persona_view.into_agent_record();
        record.persona_id = Some(record.pubkey.clone());
        record.persona_source_version = Some(source_version);
        manufactured.push(definition);
        backfilled += 1;
    }

    if backfilled == 0 {
        return Ok(0);
    }
    all.extend(manufactured);
    let payload = serde_json::to_vec_pretty(&all)
        .map_err(|e| format!("failed to serialize unified store: {e}"))?;
    crate::managed_agents::atomic_write_json_restricted(&agents_path, &payload)?;
    Ok(backfilled)
}

#[cfg(test)]
#[path = "backfill_tests.rs"]
mod tests;
