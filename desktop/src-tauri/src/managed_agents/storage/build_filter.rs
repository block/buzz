use std::{fs, path::Path};

use crate::managed_agents::ManagedAgentRecord;
use tauri::AppHandle;

use super::{backup_invalid_store, hydrate_keys, load_agent_store};

pub(crate) fn load_agent_store_from_path(path: &Path) -> Result<Vec<ManagedAgentRecord>, String> {
    if !path.exists() {
        return Ok(Vec::new());
    }

    let content =
        fs::read_to_string(path).map_err(|error| format!("failed to read agent store: {error}"))?;
    serde_json::from_str(&content).map_err(|error| {
        // Fail loudly and preserve the evidence: a later in-app save rewrites
        // this file wholesale, which would silently destroy a malformed hand
        // edit. Best-effort file-authoring contract (see managed_agents::
        // reconcile): the broken content survives as `.invalid` for the user
        // to recover, and the parse error propagates instead of being
        // swallowed into an empty store.
        backup_invalid_store(path);
        format!("failed to parse agent store (preserved as .invalid): {error}")
    })
}

/// Load the keyed agent *instances*. Key-less definitions (former personas,
/// folded into the same store) are filtered out so every pre-fold call site
/// keeps seeing exactly the records it always did.
pub fn load_managed_agents<R: tauri::Runtime>(
    app: &AppHandle<R>,
) -> Result<Vec<ManagedAgentRecord>, String> {
    let mut records = load_agent_store(app)?;
    filter_managed_agents_for_build(
        &mut records,
        crate::managed_agents::personas::bestie_build_enabled(),
    );
    hydrate_keys(&mut records);
    Ok(records)
}

/// Load the key-less agent *definitions* (former personas) from the unified
/// store. The persona compatibility shim (`load_personas`) presents these in
/// the legacy shape via `to_definition_view`.
pub(crate) fn load_agent_definitions<R: tauri::Runtime>(
    app: &AppHandle<R>,
) -> Result<Vec<ManagedAgentRecord>, String> {
    let mut records = load_agent_definitions_unfiltered(app)?;
    filter_agent_definitions_for_build(
        &mut records,
        crate::managed_agents::personas::bestie_build_enabled(),
    );
    Ok(records)
}

pub(crate) fn load_agent_definitions_unfiltered<R: tauri::Runtime>(
    app: &AppHandle<R>,
) -> Result<Vec<ManagedAgentRecord>, String> {
    let mut records = load_agent_store(app)?;
    records.retain(|record| record.pubkey.is_empty());
    Ok(records)
}

pub(crate) fn filter_managed_agents_for_build(
    records: &mut Vec<ManagedAgentRecord>,
    include_bestie: bool,
) {
    records.retain(|record| {
        !record.pubkey.is_empty()
            && record.persona_id.as_deref().is_none_or(|persona_id| {
                crate::managed_agents::personas::persona_available_in_build(
                    persona_id,
                    include_bestie,
                )
            })
    });
}

pub(crate) fn filter_agent_definitions_for_build(
    records: &mut Vec<ManagedAgentRecord>,
    include_bestie: bool,
) {
    records.retain(|record| {
        record.pubkey.is_empty()
            && record.slug.as_deref().is_none_or(|slug| {
                crate::managed_agents::personas::persona_available_in_build(slug, include_bestie)
            })
    });
}

pub(super) fn instances_for_save(
    records: &[ManagedAgentRecord],
    existing: &[ManagedAgentRecord],
    include_bestie: bool,
) -> Vec<ManagedAgentRecord> {
    let mut complete: Vec<_> = records
        .iter()
        .filter(|record| {
            !record.pubkey.is_empty()
                && record.persona_id.as_deref().is_none_or(|persona_id| {
                    crate::managed_agents::personas::persona_available_in_build(
                        persona_id,
                        include_bestie,
                    )
                })
        })
        .cloned()
        .collect();

    if !include_bestie {
        let hidden: Vec<_> = existing
            .iter()
            .filter(|record| {
                !record.pubkey.is_empty()
                    && record.persona_id.as_deref().is_some_and(|persona_id| {
                        !crate::managed_agents::personas::persona_available_in_build(
                            persona_id,
                            include_bestie,
                        )
                    })
                    && !complete
                        .iter()
                        .any(|candidate| candidate.pubkey == record.pubkey)
            })
            .cloned()
            .collect();
        complete.extend(hidden);
    }

    complete
}
