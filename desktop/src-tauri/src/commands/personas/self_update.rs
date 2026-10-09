//! `apply_agent_self_update`: apply an agent's own `draft-update` without owner
//! review when the agent's record allows every field it touches (#6287).
//!
//! The decision (`managed_agents::self_update::evaluate_self_update`) runs
//! twice: once to build the request, and again under the store lock right
//! before the write via `update_persona_guarded`, so a policy change or a
//! concurrent owner edit between the two sends the draft back to review
//! rather than clobbering anything. Every refusal is a `Review` outcome, never
//! an error: the caller then opens today's prefilled form.

use std::sync::{Arc, Mutex};

use tauri::AppHandle;

use chrono::{DateTime, Utc};

use crate::{
    app_state::AppState,
    managed_agents::{
        evaluate_self_update, fresh_draft_issued_at, load_managed_agents, load_personas,
        self_update_request, SelfUpdateDraft, SelfUpdateField, SelfUpdatePlan, SelfUpdateRejection,
    },
};

use super::retain_persona_pending;

/// Result of a self-update attempt. `Review` is the normal fallback and carries
/// the reason for the audit line; `Applied` means the definition was saved and
/// any restart is left to the auto-restart policy, exactly as a Save from the
/// edit form would be.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum AgentSelfUpdateOutcome {
    /// The definition was saved without review.
    Applied {
        /// Pubkey of the agent whose draft was applied.
        agent_pubkey: String,
        /// Id of the definition that changed.
        persona_id: String,
        /// Display name after the apply (the new one, if it was renamed).
        display_name: String,
        /// Fields the draft changed, in canonical order.
        fields: Vec<SelfUpdateField>,
    },
    /// The draft stays on the owner-review path.
    Review {
        /// Human-readable reason, for the review form and the audit line.
        reason: String,
        /// `true` when the only reason is the default empty policy, so the UI
        /// can stay quiet instead of explaining a policy the owner never set.
        policy_empty: bool,
    },
}

fn plan_under_lock(
    app: &AppHandle,
    agent_pubkey: &str,
    draft: &SelfUpdateDraft,
    issued_at: DateTime<Utc>,
) -> Result<
    Result<(SelfUpdatePlan, crate::managed_agents::UpdatePersonaRequest), SelfUpdateRejection>,
    String,
> {
    use tauri::Manager;
    let state = app.state::<AppState>();
    let _store_guard = state
        .managed_agents_store_lock
        .lock()
        .map_err(|error| error.to_string())?;
    let records = load_managed_agents(app)?;
    let personas = load_personas(app)?;
    Ok(
        evaluate_self_update(agent_pubkey, &records, &personas, draft, issued_at).and_then(
            |plan| {
                let persona = personas
                    .iter()
                    .find(|persona| persona.id == plan.persona_id)
                    .ok_or(SelfUpdateRejection::DefinitionChanged)?;
                let request = self_update_request(persona, &plan);
                Ok((plan, request))
            },
        ),
    )
}

/// The owner-review fallback, with its audit line.
fn review(
    agent_pubkey: &str,
    draft: &SelfUpdateDraft,
    reason: SelfUpdateRejection,
) -> Result<AgentSelfUpdateOutcome, String> {
    eprintln!(
        "buzz-desktop: agent self-update sent to owner review agent={agent_pubkey} target={:?} reason={reason}",
        draft.agent_name
    );
    Ok(AgentSelfUpdateOutcome::Review {
        policy_empty: reason == SelfUpdateRejection::PolicyEmpty,
        reason: reason.to_string(),
    })
}

#[tauri::command]
pub async fn apply_agent_self_update(
    agent_pubkey: String,
    draft: SelfUpdateDraft,
    issued_at: Option<String>,
    app: AppHandle,
) -> Result<AgentSelfUpdateOutcome, String> {
    let Some(issued_at) = fresh_draft_issued_at(issued_at.as_deref(), Utc::now()) else {
        return review(&agent_pubkey, &draft, SelfUpdateRejection::Stale);
    };

    let planned = tokio::task::spawn_blocking({
        let app = app.clone();
        let agent_pubkey = agent_pubkey.clone();
        let draft = draft.clone();
        move || plan_under_lock(&app, &agent_pubkey, &draft, issued_at)
    })
    .await
    .map_err(|e| format!("spawn_blocking failed: {e}"))??;
    let (plan, request) = match planned {
        Ok(planned) => planned,
        Err(rejection) => return review(&agent_pubkey, &draft, rejection),
    };

    // Re-check inside the write boundary. The closure only has a `String`
    // error channel, so the typed rejection travels out through this slot.
    let rejected: Arc<Mutex<Option<SelfUpdateRejection>>> = Arc::new(Mutex::new(None));
    let authorize = {
        let rejected = Arc::clone(&rejected);
        let agent_pubkey = agent_pubkey.clone();
        let draft = draft.clone();
        let expected_updated_at = plan.expected_updated_at.clone();
        move |app: &AppHandle,
              _state: &AppState,
              persona: &crate::managed_agents::AgentDefinition| {
            let records = load_managed_agents(app)?;
            let personas = load_personas(app)?;
            let verdict =
                evaluate_self_update(&agent_pubkey, &records, &personas, &draft, issued_at)
                    .and_then(|fresh| {
                        if fresh.persona_id != persona.id
                            || persona.updated_at != expected_updated_at
                        {
                            Err(SelfUpdateRejection::DefinitionChanged)
                        } else {
                            Ok(())
                        }
                    });
            match verdict {
                Ok(()) => Ok(()),
                Err(rejection) => {
                    let message = rejection.to_string();
                    if let Ok(mut slot) = rejected.lock() {
                        *slot = Some(rejection);
                    }
                    Err(message)
                }
            }
        }
    };
    let retain =
        |app: &AppHandle, state: &AppState, persona: &crate::managed_agents::AgentDefinition| {
            retain_persona_pending(app, state, persona);
            crate::commands::refresh_team_catalog_heads_for_persona(app, state, &persona.id);
            Ok(())
        };

    let fields = plan.fields();
    match super::update::update_persona_guarded(request, app, authorize, retain).await {
        Ok((persona, ())) => {
            // Audit line: who changed what, and that no human reviewed it.
            eprintln!(
                "buzz-desktop: agent self-update applied agent={agent_pubkey} persona={} fields={:?} channel={} review=none restart=auto-restart-policy",
                persona.id,
                fields.iter().map(|field| field.as_str()).collect::<Vec<_>>(),
                draft.channel_id
            );
            Ok(AgentSelfUpdateOutcome::Applied {
                agent_pubkey,
                persona_id: persona.id,
                display_name: persona.display_name,
                fields,
            })
        }
        Err(error) => {
            let rejection = rejected.lock().ok().and_then(|slot| slot.clone());
            match rejection {
                Some(rejection) => review(&agent_pubkey, &draft, rejection),
                None => Err(error),
            }
        }
    }
}
