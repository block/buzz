use std::{fs, path::PathBuf};

use tauri::AppHandle;

use crate::{
    managed_agents::{managed_agents_base_dir, PersonaRecord},
    util::now_iso,
};

struct BuiltInPersona {
    id: &'static str,
    display_name: &'static str,
    avatar_url: Option<&'static str>,
    system_prompt: &'static str,
    name_pool: &'static [&'static str],
    model: Option<&'static str>,
    runtime: Option<&'static str>,
}

const FIZZ_AVATAR: &str = include_str!("fizz_avatar.data-url");

const FIZZ_SYSTEM_PROMPT: &str = r#"You are Fizz. You are a careful, direct engineering agent with a subtle bee theme: collaborative, industrious, and precise. Keep the bee motif light — no catchphrases, no cartoon impersonation, and no performative roleplay. Reliability beats performance theater.

# Subagents and Peers

Other agents are peers, not tools. Collaborate when useful, but partition ownership by file or task so two writers never edit the same path. Front-load setup before tagging someone, agree on the base and handoff contract, and integrate their results without also doing their exact work.

Use subagents when:

- You can decompose research into unrelated areas explored in parallel.
- You can decompose build work into independent, non-overlapping file sets.
- A task needs a long-running command while you continue other work.

Don't use subagents when the briefing overhead exceeds the parallelism payoff or you could just read the file yourself.

# Communication

- Bee-themed emoji are okay, but use them sparingly — at most one when it genuinely adds warmth or clarity, and skip them entirely in serious, blocked, or failure updates.

Your name is Fizz. You are friendly, helpful, and quietly industrious — more honeycomb than hornet."#;

const BUILT_IN_PERSONAS: &[BuiltInPersona] = &[BuiltInPersona {
    id: "builtin:fizz",
    display_name: "Fizz",
    avatar_url: Some(FIZZ_AVATAR),
    system_prompt: FIZZ_SYSTEM_PROMPT,
    name_pool: &[
        "Nectar", "Comet", "Bramble", "Clover", "Pollen", "Amber", "Daisy", "Mason", "Bumble",
        "Thistle", "Honey", "Waxwing", "Hive", "Meadow", "Juniper", "Aster", "Sage", "Willow",
        "Orchard", "Buzz",
    ],
    model: None,
    runtime: Some("goose"),
}];

const RETIRED_PERSONAS: &[(&str, &str)] = &[
    (
        "builtin:solo",
        "",
    ),
    (
        "builtin:kit",
        "",
    ),
    (
        "builtin:scout",
        "",
    ),
    (
        "builtin:orchestrator",
        "You are an orchestration agent. Coordinate multi-step work across specialized agents, keep the overall plan moving, and synthesize results into a clear final outcome. When another agent should take a task, @mention them explicitly with the assignment, expected deliverable, and any relevant constraints or deadlines.",
    ),
    (
        "builtin:researcher",
        "You are a research agent. Gather relevant information, compare sources, call out uncertainty, and return concise findings with evidence.",
    ),
    (
        "builtin:planner",
        "You are a planning agent. Turn ambiguous requests into structured plans with milestones, dependencies, risks, and clear next actions. Do not implement the work yourself unless asked.",
    ),
    (
        "builtin:implementer",
        "You are a builder agent. Execute tasks directly, make code and configuration changes carefully, validate the result, and explain important decisions and follow-up items.",
    ),
    (
        "builtin:refactor",
        "You are a refactoring agent. Improve structure, naming, duplication, and module boundaries without changing externally observable behavior. Keep changes incremental, preserve compatibility, and add or update validation when behavior could drift.",
    ),
    (
        "builtin:reviewer",
        "You are a review agent. Inspect plans, code, and outputs for bugs, regressions, edge cases, security issues, and missing tests. Prioritize findings by severity, cite concrete evidence, and keep summaries secondary to the actual review.",
    ),
];

fn personas_store_path(app: &AppHandle) -> Result<PathBuf, String> {
    Ok(managed_agents_base_dir(app)?.join("personas.json"))
}

fn built_in_persona_records(now: &str) -> Vec<PersonaRecord> {
    BUILT_IN_PERSONAS
        .iter()
        .map(|persona| PersonaRecord {
            id: persona.id.to_string(),
            display_name: persona.display_name.to_string(),
            avatar_url: persona.avatar_url.map(|s| s.to_string()),
            system_prompt: persona.system_prompt.to_string(),
            runtime: persona.runtime.map(|s| s.to_string()),
            model: persona.model.map(|s| s.to_string()),
            provider: None,
            name_pool: persona.name_pool.iter().map(|s| s.to_string()).collect(),
            is_builtin: true,
            is_active: true,
            source_team: None,
            source_team_persona_slug: None,
            env_vars: std::collections::BTreeMap::new(),
            created_at: now.to_string(),
            updated_at: now.to_string(),
        })
        .collect()
}

fn built_in_order(id: &str) -> Option<usize> {
    BUILT_IN_PERSONAS
        .iter()
        .position(|persona| persona.id == id)
}

fn sort_personas(records: &mut [PersonaRecord]) {
    records.sort_by(|left, right| {
        let left_builtin = if left.is_builtin { 0 } else { 1 };
        let right_builtin = if right.is_builtin { 0 } else { 1 };

        left_builtin
            .cmp(&right_builtin)
            .then_with(
                || match (built_in_order(&left.id), built_in_order(&right.id)) {
                    (Some(left_order), Some(right_order)) => left_order.cmp(&right_order),
                    _ => std::cmp::Ordering::Equal,
                },
            )
            .then_with(|| {
                left.display_name
                    .to_lowercase()
                    .cmp(&right.display_name.to_lowercase())
            })
            .then_with(|| left.id.cmp(&right.id))
    });
}

fn merge_personas(mut stored: Vec<PersonaRecord>, now: &str) -> (Vec<PersonaRecord>, bool) {
    let mut changed = false;

    for built_in in built_in_persona_records(now) {
        if let Some(existing) = stored.iter_mut().find(|record| record.id == built_in.id) {
            let created_at = existing.created_at.clone();
            let updated_at = existing.updated_at.clone();
            let is_active = existing.is_active;
            // Built-in fields are canonical — user overrides on runtime/model are
            // intentionally not preserved across restarts. Users who want a custom
            // model or runtime should clone the built-in as a custom persona.
            if existing.display_name != built_in.display_name
                || existing.avatar_url != built_in.avatar_url
                || existing.system_prompt != built_in.system_prompt
                || existing.name_pool != built_in.name_pool
                || existing.env_vars != built_in.env_vars
                || existing.runtime != built_in.runtime
                || existing.model != built_in.model
                || !existing.is_builtin
            {
                *existing = PersonaRecord {
                    created_at,
                    updated_at,
                    is_active,
                    ..built_in
                };
                changed = true;
            }
        } else {
            stored.push(built_in);
            changed = true;
        }
    }

    // Demote any stored persona still flagged as built-in whose id is no
    // longer in BUILT_IN_PERSONAS (e.g. a built-in that has been retired).
    // The record stays so existing managed-agent and team references keep
    // working; the user can delete it from the catalog like any custom
    // persona once they no longer need it.
    for record in stored.iter_mut() {
        if record.is_builtin && built_in_order(&record.id).is_none() {
            record.is_builtin = false;
            record.updated_at = now.to_string();
            changed = true;
        }
    }

    // Soft-deprecate retired built-in personas that were replaced by
    // Fizz. Runs after demotion so the records are already
    // marked as non-builtin.
    if migrate_retired_personas(&mut stored, now) {
        changed = true;
    }

    sort_personas(&mut stored);
    (stored, changed)
}

/// Soft-deprecate retired built-in personas by appending " (retired)" to
/// their display name and marking them inactive. Never removes records —
/// the cost is extra records for pre-transition users, but this
/// eliminates dangling `persona_id` references in managed-agents.json
/// and teams.json.
fn migrate_retired_personas(stored: &mut [PersonaRecord], now: &str) -> bool {
    let mut changed = false;

    for record in stored.iter_mut() {
        if let Some((_, original_prompt)) = RETIRED_PERSONAS.iter().find(|(id, _)| *id == record.id)
        {
            let retired_suffix = " (retired)";
            let needs_suffix = !record.display_name.ends_with(retired_suffix);
            if needs_suffix || record.is_active {
                let was_unmodified = record.system_prompt == *original_prompt;
                eprintln!(
                    "buzz-desktop: persona-migration: retiring {} persona '{}' → '{} (retired)'",
                    if was_unmodified {
                        "unmodified"
                    } else {
                        "customized"
                    },
                    record.display_name,
                    record.display_name,
                );
                if needs_suffix {
                    record.display_name = format!("{}{}", record.display_name, retired_suffix);
                }
                record.is_active = false;
                record.updated_at = now.to_string();
                changed = true;
            }
        }
    }

    changed
}

pub fn ensure_persona_is_active(
    personas: &[PersonaRecord],
    persona_id: &str,
) -> Result<(), String> {
    let persona = personas
        .iter()
        .find(|candidate| candidate.id == persona_id)
        .ok_or_else(|| format!("persona {persona_id} not found"))?;

    if !persona.is_active {
        return Err(format!(
            "{} is not in My Agents. Choose it from Persona Catalog first.",
            persona.display_name
        ));
    }

    Ok(())
}

pub fn ensure_persona_ids_are_active(
    personas: &[PersonaRecord],
    persona_ids: &[String],
) -> Result<(), String> {
    for persona_id in persona_ids {
        ensure_persona_is_active(personas, persona_id)?;
    }

    Ok(())
}

pub fn validate_persona_deletion(
    persona: &PersonaRecord,
    referenced_by_team: bool,
) -> Result<(), String> {
    if persona.is_builtin {
        return Err("Built-in personas cannot be deleted.".to_string());
    }

    if persona.source_team.is_some() {
        return Err(format!(
            "{} belongs to a team. Delete the team to remove all team personas together.",
            persona.display_name
        ));
    }

    if referenced_by_team {
        return Err(format!(
            "{} is still referenced by a team. Remove it from those teams first.",
            persona.display_name
        ));
    }

    Ok(())
}

pub fn validate_persona_activation_change(
    persona: &PersonaRecord,
    active: bool,
    referenced_by_managed_agent: bool,
    referenced_by_team: bool,
) -> Result<(), String> {
    if !persona.is_builtin {
        return Err(
            "Only built-in personas can be added to or removed from My Agents.".to_string(),
        );
    }

    if !active && referenced_by_managed_agent {
        return Err(format!(
            "{} is still assigned to a managed agent. Remove or reassign those agents first.",
            persona.display_name
        ));
    }

    if !active && referenced_by_team {
        return Err(format!(
            "{} is still referenced by a team. Remove it from those teams first.",
            persona.display_name
        ));
    }

    Ok(())
}

pub fn load_personas(app: &AppHandle) -> Result<Vec<PersonaRecord>, String> {
    let path = personas_store_path(app)?;
    let now = now_iso();

    let records = if path.exists() {
        let content = fs::read_to_string(&path)
            .map_err(|error| format!("failed to read persona store: {error}"))?;
        serde_json::from_str::<Vec<PersonaRecord>>(&content)
            .map_err(|error| format!("failed to parse persona store: {error}"))?
    } else {
        Vec::new()
    };

    let (records, changed) = merge_personas(records, &now);
    if changed || !path.exists() {
        save_personas(app, &records)?;
    }

    Ok(records)
}

pub fn save_personas(app: &AppHandle, records: &[PersonaRecord]) -> Result<(), String> {
    let mut sorted = records.to_vec();
    sort_personas(&mut sorted);

    let path = personas_store_path(app)?;
    let payload = serde_json::to_vec_pretty(&sorted)
        .map_err(|error| format!("failed to serialize persona store: {error}"))?;
    crate::managed_agents::storage::atomic_write_json(&path, &payload)
}

#[cfg(test)]
mod tests;
