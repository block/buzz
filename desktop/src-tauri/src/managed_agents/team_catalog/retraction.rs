//! Deferred work carried OUT of lock-held store mutations, never detached.
use tauri::{AppHandle, Manager};

use super::{publication::CatalogPlan, CatalogInputs};
use crate::{active_user_signer::ActiveUserSigner, app_state::AppState};

/// A captured scope, disk decision, and inert tombstone plan. Callers must
/// return this work from their blocking phase and await completion.
#[must_use = "return catalog retractions out of the store lock and await them"]
pub(crate) struct CatalogRetraction {
    pub(crate) plan: CatalogPlan,
    pub(crate) inputs: CatalogInputs,
    pub(crate) signer: ActiveUserSigner,
    pub(crate) boot_inputs: bool,
    pub(crate) notice: Option<(String, String)>,
}

/// Production asynchronous writer. PREPARE happened under the caller's store
/// lock; SIGN holds no storage guards; COMMIT revalidates under store then SQL.
/// Cancellation during signing only drops the inert plan. Once the short
/// blocking COMMIT starts, cancellation may not stop it, but it can only commit
/// the whole purge+witness or roll back both. No signing future runs there.
pub(crate) async fn tombstone_team_catalog_coordinate<R: tauri::Runtime>(
    work: CatalogRetraction,
    app: &AppHandle<R>,
) -> Result<(), String> {
    let id = work.plan.d_tag().to_owned();
    let signed = work.plan.sign(&work.signer).await?;
    let app = app.clone();
    tokio::task::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let _guard = state
            .managed_agents_store_lock
            .lock()
            .map_err(|e| e.to_string())?;
        let (teams, personas) = if work.boot_inputs {
            let base = crate::managed_agents::managed_agents_base_dir(&app)?;
            (
                crate::event_sync::read_teams_strict(&base)?,
                crate::event_sync::read_persona_definitions(&base)?,
            )
        } else {
            (
                crate::managed_agents::load_teams(&app)?,
                crate::managed_agents::load_personas(&app)?,
            )
        };
        if CatalogInputs::capture(&id, &teams, &personas)? != work.inputs {
            return Err(
                "catalog tombstone conflict: disk inputs changed; retry reconciliation".into(),
            );
        }
        signed.commit()?;
        if let Some((name, reason)) = work.notice {
            use tauri::Emitter;
            let _ = app.emit(
                "team-catalog-auto-retracted",
                serde_json::json!({
                    "teamName": name, "reason": reason,
                }),
            );
        }
        Ok(())
    })
    .await
    .map_err(|e| format!("catalog tombstone commit join failed: {e}"))?
}

/// Best-effort mutation callers still await every retraction. Errors preserve
/// the retained head/retry witness, and never report a removal as queued.
pub(crate) async fn finish_catalog_retractions<R: tauri::Runtime>(
    app: &AppHandle<R>,
    work: Vec<CatalogRetraction>,
) {
    for retraction in work {
        if let Err(error) = super::tombstone_team_catalog_coordinate(retraction, app).await {
            eprintln!("buzz-desktop: team-catalog-retraction: {error}");
        }
    }
}
