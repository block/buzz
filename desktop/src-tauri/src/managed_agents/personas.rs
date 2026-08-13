use std::fs;

use base64::{engine::general_purpose::STANDARD, Engine as _};
use tauri::AppHandle;

use crate::{managed_agents::AgentDefinition, util::now_iso};

struct BuiltInPersona {
    id: &'static str,
    display_name: &'static str,
    avatar_png: Option<&'static [u8]>,
    system_prompt: &'static str,
    name_pool: &'static [&'static str],
    model: Option<&'static str>,
    runtime: Option<&'static str>,
    default_active: bool,
}

const DIEGO_AVATAR_PNG: &[u8] = include_bytes!("assets/diego-hat.png");
const MURIETTA_AVATAR_PNG: &[u8] = include_bytes!("assets/murietta-mask.png");
const MONTERO_AVATAR_PNG: &[u8] = include_bytes!("assets/montero-cape.png");

const DIEGO_SYSTEM_PROMPT: &str = concat!(
    "You are Diego, a seasoned project manager with deep experience connecting customers, ",
    "projects, and the teams responsible for delivery. You turn goals into practical plans, ",
    "establish clear ownership, identify dependencies, and keep work moving toward timely ",
    "completion. ",
    "Base your recommendations on available data, explicit assumptions, and calculated risks. ",
    "When information is incomplete, distinguish what is known from what is assumed. Ask for ",
    "clarification when ambiguity would materially affect the outcome; otherwise, state a ",
    "reasonable assumption and proceed. ",
    "Break large or ambiguous problems into manageable parts, reframe obstacles constructively, ",
    "and propose concrete next steps. Communicate progress, risks, decisions, and changes early ",
    "enough for stakeholders to act. Adapt the level of detail to the audience without ",
    "overwhelming them. ",
    "Be positive, realistic, concise, and direct. Do not manufacture certainty, minimize risks, ",
    "or promise outcomes unsupported by the evidence.",
);

const MURIETTA_SYSTEM_PROMPT: &str = concat!(
    "You are Murietta, a former teacher and creative problem-solver who advocates for customers ",
    "throughout the design process. Begin by understanding the audience, their goals, their ",
    "context, and the problems they need to solve. Frame designs and recommendations in terms of ",
    "the customer experience while accounting for accessibility, feasibility, and relevant ",
    "business constraints. Translate complicated ideas into clear, approachable language without ",
    "losing important meaning or speaking down to the audience. Adapt your structure, vocabulary, ",
    "tone, and level of detail to the people you are addressing. When customer needs are uncertain, ",
    "identify what is known, state your assumptions, and seek evidence or clarification from the ",
    "customer's perspective. Explore unconventional solutions when they could create meaningful ",
    "value, but clearly label them as out-of-the-box ideas and explain their assumptions, risks, ",
    "tradeoffs, and validation needs. Distinguish exploratory concepts from proven recommendations ",
    "so stakeholders can evaluate them appropriately. Be positive, imaginative, warm, and lightly ",
    "playful when appropriate. Favor experiences that are useful, inclusive, and easy to understand.",
);

const MONTERO_SYSTEM_PROMPT: &str = concat!(
    "You are Montero, a former university researcher who lives and breathes research, evidence, ",
    "and data. You investigate questions deeply enough to support sound decisions and move ",
    "projects forward, while keeping the effort proportional to the importance and uncertainty of ",
    "the decision. Begin by defining the question, identifying what evidence would answer it, and ",
    "distinguishing established facts from interpretations, assumptions, and unresolved gaps. ",
    "Support factual claims and recommendations with relevant sources when sources are available, ",
    "prioritizing primary evidence and corroborating important findings across independent sources. ",
    "Evaluate each source critically for authority, methodology, relevance, recency, limitations, ",
    "and potential conflicts of interest, and communicate that assessment transparently. Never ",
    "invent a source, citation, or level of certainty. If evidence is unavailable or inconclusive, ",
    "say so clearly and explain what would reduce the uncertainty. Explore cutting-edge and ",
    "bleeding-edge research when relevant, but label preliminary findings appropriately and ",
    "distinguish promising developments from well-established evidence. Be rigorous, candid, ",
    "realistic, and concise. Favor conclusions and solutions grounded in the strongest available ",
    "evidence, and explain the reasoning, limitations, and tradeoffs behind them.",
);

const BUILT_IN_PERSONAS: &[BuiltInPersona] = &[
    BuiltInPersona {
        id: "builtin:diego",
        display_name: "Diego",
        avatar_png: Some(DIEGO_AVATAR_PNG),
        system_prompt: DIEGO_SYSTEM_PROMPT,
        name_pool: &[
            "Nectar", "Comet", "Bramble", "Clover", "Pollen", "Amber", "Daisy", "Mason", "Thistle",
            "Waxwing", "Hive", "Meadow", "Juniper", "Aster", "Sage", "Willow", "Orchard", "Zorro",
        ],
        model: None,
        runtime: None,
        default_active: true,
    },
    BuiltInPersona {
        id: "builtin:murietta",
        display_name: "Murietta",
        avatar_png: Some(MURIETTA_AVATAR_PNG),
        system_prompt: MURIETTA_SYSTEM_PROMPT,
        name_pool: &["Murietta"],
        model: None,
        runtime: None,
        default_active: true,
    },
    BuiltInPersona {
        id: "builtin:montero",
        display_name: "Montero",
        avatar_png: Some(MONTERO_AVATAR_PNG),
        system_prompt: MONTERO_SYSTEM_PROMPT,
        name_pool: &["Montero"],
        model: None,
        runtime: None,
        default_active: true,
    },
];

fn avatar_data_url(png: &[u8]) -> String {
    format!("data:image/png;base64,{}", STANDARD.encode(png))
}

pub(crate) fn built_in_persona_avatar_url(id: &str) -> Option<String> {
    BUILT_IN_PERSONAS
        .iter()
        .find(|persona| persona.id == id)
        .and_then(|persona| persona.avatar_png)
        .map(avatar_data_url)
}

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

fn built_in_persona_records(now: &str) -> Vec<AgentDefinition> {
    BUILT_IN_PERSONAS
        .iter()
        .map(|persona| AgentDefinition {
            id: persona.id.to_string(),
            display_name: persona.display_name.to_string(),
            avatar_url: persona.avatar_png.map(avatar_data_url),
            system_prompt: persona.system_prompt.to_string(),
            runtime: persona.runtime.map(|s| s.to_string()),
            model: persona.model.map(|s| s.to_string()),
            provider: None,
            name_pool: persona.name_pool.iter().map(|s| s.to_string()).collect(),
            is_builtin: true,
            is_active: persona.default_active,
            shared: false,
            source_team: None,
            source_team_persona_slug: None,
            catalog_source: None,
            env_vars: std::collections::BTreeMap::new(),
            respond_to: None,
            respond_to_allowlist: Vec::new(),
            parallelism: None,
            created_at: now.to_string(),
            updated_at: now.to_string(),
        })
        .collect()
}

pub(crate) fn built_in_persona_definition(id: &str, now: &str) -> Option<AgentDefinition> {
    built_in_persona_records(now)
        .into_iter()
        .find(|persona| persona.id == id)
}

fn built_in_order(id: &str) -> Option<usize> {
    BUILT_IN_PERSONAS
        .iter()
        .position(|persona| persona.id == id)
}

fn sort_personas(records: &mut [AgentDefinition]) {
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

fn merge_personas(mut stored: Vec<AgentDefinition>, now: &str) -> (Vec<AgentDefinition>, bool) {
    let mut changed = false;

    for built_in in built_in_persona_records(now) {
        if let Some(existing) = stored.iter_mut().find(|record| record.id == built_in.id) {
            if !existing.is_builtin {
                existing.is_builtin = true;
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
    // Diego. Runs after demotion so the records are already
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
fn migrate_retired_personas(stored: &mut [AgentDefinition], now: &str) -> bool {
    let mut changed = false;

    for record in stored.iter_mut() {
        if let Some((_, original_prompt)) = RETIRED_PERSONAS.iter().find(|(id, _)| *id == record.id)
        {
            let retired_suffix = " (retired)";
            let needs_suffix = !record.display_name.ends_with(retired_suffix);
            if needs_suffix || record.is_active {
                let was_unmodified = record.system_prompt == *original_prompt;
                eprintln!(
                    "zorro-desktop: persona-migration: retiring {} persona '{}' → '{} (retired)'",
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
    personas: &[AgentDefinition],
    persona_id: &str,
) -> Result<(), String> {
    let persona = personas
        .iter()
        .find(|candidate| candidate.id == persona_id)
        .ok_or_else(|| format!("agent {persona_id} not found"))?;

    if !persona.is_active {
        return Err(format!("{} is not in My Agents.", persona.display_name));
    }

    Ok(())
}

pub fn ensure_persona_ids_are_active(
    personas: &[AgentDefinition],
    persona_ids: &[String],
) -> Result<(), String> {
    for persona_id in persona_ids {
        ensure_persona_is_active(personas, persona_id)?;
    }

    Ok(())
}

pub fn validate_persona_deletion(
    persona: &AgentDefinition,
    referenced_by_team: bool,
) -> Result<(), String> {
    if persona.is_builtin {
        return Err("Built-in agents cannot be deleted.".to_string());
    }

    if persona.source_team.is_some() {
        return Err(format!(
            "{} belongs to a team. Delete the team to remove all team agents together.",
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
    persona: &AgentDefinition,
    active: bool,
    referenced_by_managed_agent: bool,
    referenced_by_team: bool,
) -> Result<(), String> {
    if !persona.is_builtin {
        return Err("Only built-in agents can be added to or removed from My Agents.".to_string());
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

pub fn load_personas(app: &AppHandle) -> Result<Vec<AgentDefinition>, String> {
    let now = now_iso();

    // Post-fold: definitions live in the unified agent store, presented in
    // the legacy shape. Pre-fold stores are converted by
    // `fold_personas_into_agent_store` in boot migrations before any caller
    // reaches this shim.
    let records = crate::managed_agents::storage::load_agent_definitions(app)?
        .iter()
        .filter_map(|record| record.to_definition_view())
        .collect();

    let (records, changed) = merge_personas(records, &now);
    if changed {
        save_personas(app, &records)?;
    }

    Ok(records)
}

/// Read the raw persona records at `path` — no built-in merge, no write-back.
/// The single disk-read seam for persona definitions: `load_personas` layers
/// the built-in merge on top, and the boot-time readers that need raw records
/// without an `AppHandle` (`event_sync`, `migration::load_persona_runtimes`)
/// call it directly. The A2 store fold retargets THIS function at the unified
/// store; its callers stay unchanged.
pub(crate) fn load_personas_from_path(
    path: &std::path::Path,
) -> Result<Vec<AgentDefinition>, String> {
    if !path.exists() {
        return Ok(Vec::new());
    }
    let content = fs::read_to_string(path)
        .map_err(|error| format!("failed to read persona store: {error}"))?;
    serde_json::from_str::<Vec<AgentDefinition>>(&content)
        .map_err(|error| format!("failed to parse persona store: {error}"))
}

pub fn save_personas(app: &AppHandle, records: &[AgentDefinition]) -> Result<(), String> {
    let mut sorted = records.to_vec();
    sort_personas(&mut sorted);

    // Post-fold: persona saves write key-less definition records into the
    // unified agent store (instances preserved by `save_agent_definitions`).
    let definitions: Vec<_> = sorted
        .into_iter()
        .map(|persona| persona.into_agent_record())
        .collect();
    crate::managed_agents::storage::save_agent_definitions(app, &definitions)
}

#[cfg(test)]
mod tests;
